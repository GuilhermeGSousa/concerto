//! Named actions bound to key combinations, resolved through a context stack.
//!
//! Systems ask "did the user ask to frame the selection?", not "is F down?".
//! That is what lets one key mean different things in different panels, lets a
//! text field swallow plain letters without every system checking focus, and
//! lets bindings live in one table instead of scattered through the code.
use ecs::{define_label, events::Event, intern::Interned, resource::Resource};
use winit::keyboard::{KeyCode, PhysicalKey};

use crate::input::Input;

define_label!(ActionLabel);
define_label!(ContextLabel);

pub type InternedActionLabel = Interned<dyn ActionLabel>;
pub type InternedContextLabel = Interned<dyn ContextLabel>;

/// Defines a named action, following the same pattern as a schedule label.
#[macro_export]
macro_rules! define_action {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, PartialEq, Eq, Hash, Debug)]
        pub struct $name;

        impl $crate::input::actions::ActionLabel for $name {
            fn dyn_clone(&self) -> Box<dyn $crate::input::actions::ActionLabel> {
                Box::new(self.clone())
            }
        }
    };
}

/// Defines a context in which bindings apply.
#[macro_export]
macro_rules! define_context {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, PartialEq, Eq, Hash, Debug)]
        pub struct $name;

        impl $crate::input::actions::ContextLabel for $name {
            fn dyn_clone(&self) -> Box<dyn $crate::input::actions::ContextLabel> {
                Box::new(self.clone())
            }
        }
    };
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Modifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub logo: bool,
}

impl Modifiers {
    pub const NONE: Modifiers = Modifiers {
        ctrl: false,
        shift: false,
        alt: false,
        logo: false,
    };

    fn any(self) -> bool {
        self.ctrl || self.shift || self.alt || self.logo
    }

    /// The modifiers currently held down.
    pub fn from_input(input: &Input) -> Self {
        let held = |left, right| {
            input.is_held(PhysicalKey::Code(left)) || input.is_held(PhysicalKey::Code(right))
        };
        Self {
            ctrl: held(KeyCode::ControlLeft, KeyCode::ControlRight),
            shift: held(KeyCode::ShiftLeft, KeyCode::ShiftRight),
            alt: held(KeyCode::AltLeft, KeyCode::AltRight),
            logo: held(KeyCode::SuperLeft, KeyCode::SuperRight),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Shortcut {
    pub key: KeyCode,
    pub modifiers: Modifiers,
}

impl Shortcut {
    pub fn key(key: KeyCode) -> Self {
        Self {
            key,
            modifiers: Modifiers::NONE,
        }
    }

    pub fn ctrl(key: KeyCode) -> Self {
        Self::key(key).with_ctrl()
    }

    pub fn with_ctrl(mut self) -> Self {
        self.modifiers.ctrl = true;
        self
    }

    pub fn with_shift(mut self) -> Self {
        self.modifiers.shift = true;
        self
    }

    pub fn with_alt(mut self) -> Self {
        self.modifiers.alt = true;
        self
    }

    /// Whether this is the kind of press a text field consumes: a bare key that
    /// produces a character. Navigation and editing keys are not text.
    fn is_text_like(self) -> bool {
        if self.modifiers.any() {
            return false;
        }
        !matches!(
            self.key,
            KeyCode::Escape
                | KeyCode::Tab
                | KeyCode::Enter
                | KeyCode::NumpadEnter
                | KeyCode::ArrowUp
                | KeyCode::ArrowDown
                | KeyCode::ArrowLeft
                | KeyCode::ArrowRight
                | KeyCode::Home
                | KeyCode::End
                | KeyCode::PageUp
                | KeyCode::PageDown
                | KeyCode::Delete
                | KeyCode::Backspace
                | KeyCode::Insert
                | KeyCode::F1
                | KeyCode::F2
                | KeyCode::F3
                | KeyCode::F4
                | KeyCode::F5
                | KeyCode::F6
                | KeyCode::F7
                | KeyCode::F8
                | KeyCode::F9
                | KeyCode::F10
                | KeyCode::F11
                | KeyCode::F12
        )
    }
}

struct Binding {
    shortcut: Shortcut,
    action: InternedActionLabel,
    /// `None` is the global context: always active, lowest precedence.
    context: Option<InternedContextLabel>,
}

/// Fired once for each action a key press resolved to.
#[derive(Event, Clone, Copy, PartialEq, Eq, Debug)]
pub struct ActionFired {
    pub action: InternedActionLabel,
}

impl ActionFired {
    /// Whether this is the given action, so systems can match without importing
    /// the interning machinery.
    pub fn is(&self, action: impl ActionLabel) -> bool {
        self.action == action.intern()
    }
}

#[derive(Resource, Default)]
pub struct ActionMap {
    bindings: Vec<Binding>,
    contexts: Vec<InternedContextLabel>,
    capturing_text: bool,
}

impl ActionMap {
    /// Binds `action` within `context`. An action has one shortcut per context,
    /// so binding it again moves it.
    pub fn bind(
        &mut self,
        action: impl ActionLabel,
        shortcut: Shortcut,
        context: impl ContextLabel,
    ) {
        self.insert(action.intern(), shortcut, Some(context.intern()));
    }

    /// Binds `action` everywhere, unless an active context overrides the key.
    pub fn bind_global(&mut self, action: impl ActionLabel, shortcut: Shortcut) {
        self.insert(action.intern(), shortcut, None);
    }

    fn insert(
        &mut self,
        action: InternedActionLabel,
        shortcut: Shortcut,
        context: Option<InternedContextLabel>,
    ) {
        self.bindings
            .retain(|binding| !(binding.action == action && binding.context == context));
        self.bindings.push(Binding {
            shortcut,
            action,
            context,
        });
    }

    pub fn push_context(&mut self, context: impl ContextLabel) {
        let context = context.intern();
        // Re-pushing moves a context to the top rather than stacking a
        // duplicate, so one stray push cannot outlive its pop.
        self.contexts.retain(|active| *active != context);
        self.contexts.push(context);
    }

    pub fn pop_context(&mut self, context: impl ContextLabel) {
        let context = context.intern();
        self.contexts.retain(|active| *active != context);
    }

    pub fn is_active(&self, context: impl ContextLabel) -> bool {
        self.contexts.contains(&context.intern())
    }

    /// While set, unmodified text keys are being typed into a widget and must
    /// not fire actions.
    pub fn set_capturing_text(&mut self, capturing: bool) {
        self.capturing_text = capturing;
    }

    pub fn capturing_text(&self) -> bool {
        self.capturing_text
    }

    /// The action `shortcut` means right now, if any.
    ///
    /// Contexts are searched innermost-first, then the global bindings, so a
    /// panel can claim a key without knowing what else uses it.
    pub fn resolve(&self, shortcut: Shortcut) -> Option<InternedActionLabel> {
        if self.capturing_text && shortcut.is_text_like() {
            return None;
        }
        for context in self.contexts.iter().rev() {
            if let Some(action) = self.matching(shortcut, Some(*context)) {
                return Some(action);
            }
        }
        self.matching(shortcut, None)
    }

    fn matching(
        &self,
        shortcut: Shortcut,
        context: Option<InternedContextLabel>,
    ) -> Option<InternedActionLabel> {
        self.bindings
            .iter()
            .find(|binding| binding.shortcut == shortcut && binding.context == context)
            .map(|binding| binding.action)
    }
}

/// Turns this frame's key presses into [`ActionFired`] events.
pub(crate) fn resolve_actions(
    input: ecs::resource::Res<Input>,
    map: ecs::resource::Res<ActionMap>,
    mut writer: ecs::events::event_writer::EventWriter<ActionFired>,
) {
    let modifiers = Modifiers::from_input(&input);
    for key in input.just_pressed_keys() {
        let PhysicalKey::Code(key) = key else {
            continue;
        };
        if let Some(action) = map.resolve(Shortcut { key, modifiers }) {
            writer.write(ActionFired { action });
        }
    }
}
