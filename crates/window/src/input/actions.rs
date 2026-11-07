//! Named actions bound to key combinations, resolved through a context stack.
//!
//! Systems ask "did the user ask to frame the selection?", not "is F down?".
//! That is what lets one key mean different things in different panels, lets a
//! text field swallow plain letters without every system checking focus, and
//! lets bindings live in one table instead of scattered through the code.
use concerto_ecs::{define_label, events::Event, intern::Interned, resource::Resource};
use glam::Vec2;
use winit::keyboard::{KeyCode, PhysicalKey};

use crate::input::{Gamepad, GamepadButton, GamepadId, GamepadStick, Input, MouseButton};

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
            logo: held(KeyCode::MetaLeft, KeyCode::MetaRight),
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

/// One button-like input an action can be bound to.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ButtonBinding {
    Key(Shortcut),
    Mouse(MouseButton),
    Gamepad(GamepadButton),
}

impl From<Shortcut> for ButtonBinding {
    fn from(shortcut: Shortcut) -> Self {
        Self::Key(shortcut)
    }
}

impl From<MouseButton> for ButtonBinding {
    fn from(button: MouseButton) -> Self {
        Self::Mouse(button)
    }
}

impl From<GamepadButton> for ButtonBinding {
    fn from(button: GamepadButton) -> Self {
        Self::Gamepad(button)
    }
}

impl ButtonBinding {
    fn same_kind(self, other: ButtonBinding) -> bool {
        std::mem::discriminant(&self) == std::mem::discriminant(&other)
    }
}

/// A source of a two-dimensional action value, such as movement.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Axis2dBinding {
    Keys {
        up: KeyCode,
        down: KeyCode,
        left: KeyCode,
        right: KeyCode,
    },
    Stick(GamepadStick),
}

impl Axis2dBinding {
    pub fn wasd() -> Self {
        Self::Keys {
            up: KeyCode::KeyW,
            down: KeyCode::KeyS,
            left: KeyCode::KeyA,
            right: KeyCode::KeyD,
        }
    }

    pub fn arrows() -> Self {
        Self::Keys {
            up: KeyCode::ArrowUp,
            down: KeyCode::ArrowDown,
            left: KeyCode::ArrowLeft,
            right: KeyCode::ArrowRight,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum BindingInput {
    Button(ButtonBinding),
    Axis2d(Axis2dBinding),
}

struct Binding {
    input: BindingInput,
    action: InternedActionLabel,
    /// `None` is the global context: always active, lowest precedence.
    context: Option<InternedContextLabel>,
}

#[derive(Clone, Copy, Default)]
struct ButtonReading {
    held: bool,
    pressed: bool,
    released: bool,
    held_before: bool,
    still_held: bool,
}

impl ButtonReading {
    fn new(held: bool, pressed: bool, released: bool) -> Self {
        Self {
            held,
            pressed,
            released,
            held_before: held && !pressed,
            still_held: held && !released,
        }
    }

    fn merge(self, other: ButtonReading) -> Self {
        Self {
            held: self.held || other.held,
            pressed: self.pressed || other.pressed,
            released: self.released || other.released,
            held_before: self.held_before || other.held_before,
            still_held: self.still_held || other.still_held,
        }
    }
}

/// Fired once for each action a key or gamepad button press resolved to.
#[derive(Event, Clone, Copy, PartialEq, Eq, Debug)]
pub struct ActionFired {
    pub action: InternedActionLabel,
    /// The pad whose button fired it, or `None` for a key.
    pub gamepad: Option<GamepadId>,
}

impl ActionFired {
    /// An action fired from the keyboard.
    pub fn new(action: impl ActionLabel) -> Self {
        Self {
            action: action.intern(),
            gamepad: None,
        }
    }

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
    /// Adds `input` to the inputs that trigger `action` within `context`.
    pub fn bind(
        &mut self,
        action: impl ActionLabel,
        input: impl Into<ButtonBinding>,
        context: impl ContextLabel,
    ) {
        self.add(
            action.intern(),
            BindingInput::Button(input.into()),
            Some(context.intern()),
        );
    }

    /// Adds `input` to the inputs that trigger `action` everywhere, unless an active context overrides it.
    pub fn bind_global(&mut self, action: impl ActionLabel, input: impl Into<ButtonBinding>) {
        self.add(action.intern(), BindingInput::Button(input.into()), None);
    }

    /// Replaces `action`'s button bindings of the same device kind within `context` with `input`.
    pub fn rebind(
        &mut self,
        action: impl ActionLabel,
        input: impl Into<ButtonBinding>,
        context: impl ContextLabel,
    ) {
        self.replace(action.intern(), input.into(), Some(context.intern()));
    }

    /// Replaces `action`'s global button bindings of the same device kind with `input`.
    pub fn rebind_global(&mut self, action: impl ActionLabel, input: impl Into<ButtonBinding>) {
        self.replace(action.intern(), input.into(), None);
    }

    /// Removes every binding of `action` within `context`.
    pub fn unbind(&mut self, action: impl ActionLabel, context: impl ContextLabel) {
        self.remove(action.intern(), Some(context.intern()));
    }

    /// Removes every global binding of `action`.
    pub fn unbind_global(&mut self, action: impl ActionLabel) {
        self.remove(action.intern(), None);
    }

    /// Adds a two-dimensional source for `action` within `context`.
    pub fn bind_axis2d(
        &mut self,
        action: impl ActionLabel,
        binding: Axis2dBinding,
        context: impl ContextLabel,
    ) {
        self.add(
            action.intern(),
            BindingInput::Axis2d(binding),
            Some(context.intern()),
        );
    }

    /// Adds a two-dimensional source for `action` everywhere.
    pub fn bind_axis2d_global(&mut self, action: impl ActionLabel, binding: Axis2dBinding) {
        self.add(action.intern(), BindingInput::Axis2d(binding), None);
    }

    fn add(
        &mut self,
        action: InternedActionLabel,
        input: BindingInput,
        context: Option<InternedContextLabel>,
    ) {
        let bound = self.bindings.iter().any(|binding| {
            binding.action == action && binding.context == context && binding.input == input
        });
        if !bound {
            self.bindings.push(Binding {
                input,
                action,
                context,
            });
        }
    }

    fn replace(
        &mut self,
        action: InternedActionLabel,
        input: ButtonBinding,
        context: Option<InternedContextLabel>,
    ) {
        self.bindings.retain(|binding| {
            let same_kind =
                matches!(binding.input, BindingInput::Button(bound) if bound.same_kind(input));
            !(binding.action == action && binding.context == context && same_kind)
        });
        self.add(action, BindingInput::Button(input), context);
    }

    fn remove(&mut self, action: InternedActionLabel, context: Option<InternedContextLabel>) {
        self.bindings
            .retain(|binding| !(binding.action == action && binding.context == context));
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
        self.resolve_button(ButtonBinding::Key(shortcut))
    }

    fn resolve_button(&self, button: ButtonBinding) -> Option<InternedActionLabel> {
        let input = BindingInput::Button(button);
        for context in self.contexts.iter().rev() {
            if let Some(action) = self.matching(input, Some(*context)) {
                return Some(action);
            }
        }
        self.matching(input, None)
    }

    fn matching(
        &self,
        input: BindingInput,
        context: Option<InternedContextLabel>,
    ) -> Option<InternedActionLabel> {
        self.bindings
            .iter()
            .find(|binding| binding.input == input && binding.context == context)
            .map(|binding| binding.action)
    }

    /// The actions this frame's key and gamepad button presses fire.
    pub fn fired(&self, input: &Input) -> Vec<ActionFired> {
        let mut fired = Vec::new();
        let modifiers = Modifiers::from_input(input);
        for key in input.just_pressed_keys() {
            let PhysicalKey::Code(key) = key else {
                continue;
            };
            if let Some(action) = self.resolve(Shortcut { key, modifiers }) {
                fired.push(ActionFired {
                    action,
                    gamepad: None,
                });
            }
        }
        for pad in input.gamepads() {
            for button in GamepadButton::ALL {
                if !pad.is_just_pressed(button) {
                    continue;
                }
                if let Some(action) = self.resolve_button(ButtonBinding::Gamepad(button)) {
                    fired.push(ActionFired {
                        action,
                        gamepad: Some(pad.id()),
                    });
                }
            }
        }
        fired
    }

    /// Whether any live binding of `action` is held.
    pub fn is_held(&self, action: impl ActionLabel, input: &Input) -> bool {
        self.reading(action.intern(), input).held
    }

    /// Whether a binding of `action` went down this frame while none of its bindings was already held.
    pub fn is_just_pressed(&self, action: impl ActionLabel, input: &Input) -> bool {
        let reading = self.reading(action.intern(), input);
        reading.pressed && !reading.held_before
    }

    /// Whether a binding of `action` went up this frame and none of its bindings is still held.
    pub fn is_just_released(&self, action: impl ActionLabel, input: &Input) -> bool {
        let reading = self.reading(action.intern(), input);
        reading.released && !reading.still_held
    }

    /// The two-dimensional value of `action`, of length at most 1.
    pub fn axis2d(&self, action: impl ActionLabel, input: &Input) -> Vec2 {
        let action = action.intern();
        let mut seen = Vec::new();
        let mut keys = Vec2::ZERO;
        let mut stick = Vec2::ZERO;
        for binding in self.live(action) {
            let BindingInput::Axis2d(axis) = binding.input else {
                continue;
            };
            match axis {
                Axis2dBinding::Keys {
                    up,
                    down,
                    left,
                    right,
                } => {
                    for key in [up, down, left, right] {
                        if seen.contains(&key) || !self.key_held(key, input) {
                            continue;
                        }
                        seen.push(key);
                        keys += if key == up {
                            Vec2::Y
                        } else if key == down {
                            Vec2::NEG_Y
                        } else if key == left {
                            Vec2::NEG_X
                        } else {
                            Vec2::X
                        };
                    }
                }
                Axis2dBinding::Stick(which) => {
                    for pad in input.gamepads() {
                        let value = pad.stick(which);
                        if value.length() > stick.length() {
                            stick = value;
                        }
                    }
                }
            }
        }
        let keys = keys.normalize_or_zero();
        if stick.length() > keys.length() {
            stick
        } else {
            keys
        }
    }

    fn live(&self, action: InternedActionLabel) -> impl Iterator<Item = &Binding> + '_ {
        self.bindings.iter().filter(move |binding| {
            binding.action == action
                && binding
                    .context
                    .is_none_or(|context| self.contexts.contains(&context))
        })
    }

    fn reading(&self, action: InternedActionLabel, input: &Input) -> ButtonReading {
        self.live(action)
            .filter_map(|binding| match binding.input {
                BindingInput::Button(button) => Some(self.button_reading(button, input)),
                BindingInput::Axis2d(_) => None,
            })
            .fold(ButtonReading::default(), ButtonReading::merge)
    }

    fn button_reading(&self, button: ButtonBinding, input: &Input) -> ButtonReading {
        match button {
            ButtonBinding::Key(shortcut) => {
                if self.capturing_text && shortcut.is_text_like() {
                    return ButtonReading::default();
                }
                let key = PhysicalKey::Code(shortcut.key);
                let modifiers = modifiers_held(shortcut.modifiers, input);
                ButtonReading::new(
                    input.is_held(key) && modifiers,
                    input.is_just_pressed(key) && modifiers,
                    input.is_just_released(key),
                )
            }
            ButtonBinding::Mouse(button) => ButtonReading::new(
                input.is_mouse_button_held(button),
                input.is_mouse_button_just_pressed(button),
                input.is_mouse_button_just_released(button),
            ),
            ButtonBinding::Gamepad(button) => input
                .gamepads()
                .map(|pad| pad_reading(pad, button))
                .fold(ButtonReading::default(), ButtonReading::merge),
        }
    }

    fn key_held(&self, key: KeyCode, input: &Input) -> bool {
        if self.capturing_text && Shortcut::key(key).is_text_like() {
            return false;
        }
        input.is_held(PhysicalKey::Code(key))
    }
}

fn modifiers_held(required: Modifiers, input: &Input) -> bool {
    let held = Modifiers::from_input(input);
    (!required.ctrl || held.ctrl)
        && (!required.shift || held.shift)
        && (!required.alt || held.alt)
        && (!required.logo || held.logo)
}

fn pad_reading(pad: &Gamepad, button: GamepadButton) -> ButtonReading {
    ButtonReading::new(
        pad.is_held(button),
        pad.is_just_pressed(button),
        pad.is_just_released(button),
    )
}

/// Turns this frame's key and gamepad button presses into [`ActionFired`] events.
pub(crate) fn resolve_actions(
    input: concerto_ecs::resource::Res<Input>,
    map: concerto_ecs::resource::Res<ActionMap>,
    mut writer: concerto_ecs::events::event_writer::EventWriter<ActionFired>,
) {
    for fired in map.fired(&input) {
        writer.write(fired);
    }
}
