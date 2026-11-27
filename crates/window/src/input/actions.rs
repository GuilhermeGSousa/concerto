//! Named actions bound to key combinations, resolved through a context stack.
//!
//! Systems ask "did the user ask to frame the selection?", not "is F down?".
//! That is what lets one key mean different things in different panels, lets a
//! text field swallow plain letters without every system checking focus, and
//! lets bindings live in one table instead of scattered through the code.
use std::collections::BTreeMap;

use concerto_ecs::{define_label, events::Event, intern::Interned, resource::Resource};
use glam::Vec2;
use winit::keyboard::{KeyCode, PhysicalKey};

use crate::input::{
    Gamepad, GamepadButton, GamepadId, GamepadStick, Input, MouseButton, PlayerDevice,
    PlayerDevices, PlayerIndex, Players,
};

define_label!(
    /// A named action, such as "jump" or "frame the selection"; declare one with
    /// [`define_action!`](crate::define_action).
    ActionLabel
);
define_label!(
    /// A context in which bindings apply, such as a panel or a game mode; declare one with
    /// [`define_context!`](crate::define_context).
    ContextLabel
);

/// An interned [`ActionLabel`], cheap to copy and compare.
pub type InternedActionLabel = Interned<dyn ActionLabel>;
/// An interned [`ContextLabel`], cheap to copy and compare.
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

/// The modifier keys a [`Shortcut`] requires.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Modifiers {
    /// Either Control key.
    pub ctrl: bool,
    /// Either Shift key.
    pub shift: bool,
    /// Either Alt key, Option on macOS.
    pub alt: bool,
    /// Either logo key: Windows, or Command on macOS.
    pub logo: bool,
}

impl Modifiers {
    /// No modifiers.
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

/// A key pressed with a set of modifiers held.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Shortcut {
    /// The key pressed.
    pub key: KeyCode,
    /// The modifiers that must be held with it.
    pub modifiers: Modifiers,
}

impl Shortcut {
    /// Creates the shortcut for `key` alone.
    pub fn key(key: KeyCode) -> Self {
        Self {
            key,
            modifiers: Modifiers::NONE,
        }
    }

    /// Creates the shortcut for `key` with Control held.
    pub fn ctrl(key: KeyCode) -> Self {
        Self::key(key).with_ctrl()
    }

    /// Requires Control as well.
    pub fn with_ctrl(mut self) -> Self {
        self.modifiers.ctrl = true;
        self
    }

    /// Requires Shift as well.
    pub fn with_shift(mut self) -> Self {
        self.modifiers.shift = true;
        self
    }

    /// Requires Alt as well.
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
    /// A key with modifiers.
    Key(Shortcut),
    /// A mouse button.
    Mouse(MouseButton),
    /// A gamepad button, on whichever pad the reading player holds.
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
    /// Four keys, each pushing the value one unit in its direction; `up` is positive Y.
    Keys {
        /// The key for positive Y.
        up: KeyCode,
        /// The key for negative Y.
        down: KeyCode,
        /// The key for negative X.
        left: KeyCode,
        /// The key for positive X.
        right: KeyCode,
    },
    /// A gamepad stick, with the dead zone applied.
    Stick(GamepadStick),
}

impl Axis2dBinding {
    /// Binds W, S, A and D to up, down, left and right.
    pub fn wasd() -> Self {
        Self::Keys {
            up: KeyCode::KeyW,
            down: KeyCode::KeyS,
            left: KeyCode::KeyA,
            right: KeyCode::KeyD,
        }
    }

    /// Binds the arrow keys.
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

    fn just_pressed(self) -> bool {
        self.pressed && !self.held_before
    }

    fn just_released(self) -> bool {
        self.released && !self.still_held
    }
}

#[derive(Clone, Copy)]
enum Devices {
    Every,
    Only(PlayerDevices),
}

impl Devices {
    fn keyboard_mouse(self) -> bool {
        match self {
            Devices::Every => true,
            Devices::Only(devices) => devices.keyboard_mouse,
        }
    }

    fn pads(self, input: &Input) -> impl Iterator<Item = &Gamepad> {
        input.gamepads().filter(move |pad| match self {
            Devices::Every => true,
            Devices::Only(devices) => devices.gamepad == Some(pad.id()),
        })
    }
}

#[derive(Clone, Copy)]
struct Scope {
    player: Option<PlayerIndex>,
    devices: Devices,
}

impl Scope {
    const SHARED: Scope = Scope {
        player: None,
        devices: Devices::Every,
    };

    fn player(player: PlayerIndex, players: &Players) -> Self {
        Self {
            player: Some(player),
            devices: Devices::Only(players.devices(player).unwrap_or_default()),
        }
    }
}

/// Fired once for each action a key or gamepad button press resolved to.
#[derive(Event, Clone, Copy, PartialEq, Eq, Debug)]
pub struct ActionFired {
    /// The action that fired.
    pub action: InternedActionLabel,
    /// The pad whose button fired it, or `None` for a key.
    pub gamepad: Option<GamepadId>,
    /// The player holding the device that fired it, or `None` for an unassigned device.
    pub player: Option<PlayerIndex>,
}

impl ActionFired {
    /// An action fired from the keyboard.
    pub fn new(action: impl ActionLabel) -> Self {
        Self {
            action: action.intern(),
            gamepad: None,
            player: None,
        }
    }

    /// Whether this is the given action, so systems can match without importing
    /// the interning machinery.
    pub fn is(&self, action: impl ActionLabel) -> bool {
        self.action == action.intern()
    }
}

/// The bindings from inputs to actions, and the stacks of contexts that decide which apply.
///
/// Bindings in an active context take precedence over global ones, and later contexts over
/// earlier ones.
#[derive(Resource, Default)]
pub struct ActionMap {
    bindings: Vec<Binding>,
    contexts: Vec<InternedContextLabel>,
    player_contexts: BTreeMap<PlayerIndex, Vec<InternedContextLabel>>,
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

    /// Activates `context` on top of the shared stack; pushing it again moves it to the top.
    pub fn push_context(&mut self, context: impl ContextLabel) {
        push_onto(&mut self.contexts, context.intern());
    }

    /// Removes `context` from the shared stack, wherever it is in it.
    pub fn pop_context(&mut self, context: impl ContextLabel) {
        let context = context.intern();
        self.contexts.retain(|active| *active != context);
    }

    /// Whether `context` is on the shared stack.
    pub fn is_active(&self, context: impl ContextLabel) -> bool {
        self.contexts.contains(&context.intern())
    }

    /// Pushes `context` onto `player`'s own stack, which applies on top of the shared one.
    pub fn push_context_for(&mut self, player: PlayerIndex, context: impl ContextLabel) {
        push_onto(
            self.player_contexts.entry(player).or_default(),
            context.intern(),
        );
    }

    /// Removes `context` from `player`'s own stack.
    pub fn pop_context_for(&mut self, player: PlayerIndex, context: impl ContextLabel) {
        let context = context.intern();
        if let Some(stack) = self.player_contexts.get_mut(&player) {
            stack.retain(|active| *active != context);
        }
    }

    /// Whether `context` is on `player`'s own stack.
    pub fn is_active_for(&self, player: PlayerIndex, context: impl ContextLabel) -> bool {
        self.player_stack(Some(player)).contains(&context.intern())
    }

    /// Empties `player`'s own stack, as a game does when that player leaves.
    pub fn clear_contexts_for(&mut self, player: PlayerIndex) {
        self.player_contexts.remove(&player);
    }

    fn player_stack(&self, player: Option<PlayerIndex>) -> &[InternedContextLabel] {
        player
            .and_then(|player| self.player_contexts.get(&player))
            .map_or(&[], Vec::as_slice)
    }

    /// While set, unmodified text keys are being typed into a widget and must
    /// not fire actions.
    pub fn set_capturing_text(&mut self, capturing: bool) {
        self.capturing_text = capturing;
    }

    /// Whether a widget is taking text, so unmodified text keys fire no actions.
    pub fn capturing_text(&self) -> bool {
        self.capturing_text
    }

    /// The action `shortcut` means right now, if any.
    ///
    /// Contexts are searched innermost-first, then the global bindings, so a
    /// panel can claim a key without knowing what else uses it.
    pub fn resolve(&self, shortcut: Shortcut) -> Option<InternedActionLabel> {
        self.resolve_key(shortcut, None)
    }

    fn resolve_key(
        &self,
        shortcut: Shortcut,
        player: Option<PlayerIndex>,
    ) -> Option<InternedActionLabel> {
        if self.capturing_text && shortcut.is_text_like() {
            return None;
        }
        self.resolve_button(ButtonBinding::Key(shortcut), player)
    }

    fn resolve_button(
        &self,
        button: ButtonBinding,
        player: Option<PlayerIndex>,
    ) -> Option<InternedActionLabel> {
        let input = BindingInput::Button(button);
        let stacks = self.player_stack(player).iter().rev();
        for context in stacks.chain(self.contexts.iter().rev()) {
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

    /// The actions this frame's key and gamepad button presses fire, each with the player holding its device.
    pub fn fired(&self, input: &Input, players: &Players) -> Vec<ActionFired> {
        let mut fired = Vec::new();
        let modifiers = Modifiers::from_input(input);
        let keyboard_player = players.player_of(PlayerDevice::KeyboardMouse);
        for key in input.just_pressed_keys() {
            let PhysicalKey::Code(key) = key else {
                continue;
            };
            if let Some(action) = self.resolve_key(Shortcut { key, modifiers }, keyboard_player) {
                fired.push(ActionFired {
                    action,
                    gamepad: None,
                    player: keyboard_player,
                });
            }
        }
        for pad in input.gamepads() {
            let player = players.player_of(PlayerDevice::Gamepad(pad.id()));
            for button in GamepadButton::ALL {
                if !pad.is_just_pressed(button) {
                    continue;
                }
                if let Some(action) = self.resolve_button(ButtonBinding::Gamepad(button), player) {
                    fired.push(ActionFired {
                        action,
                        gamepad: Some(pad.id()),
                        player,
                    });
                }
            }
        }
        fired
    }

    /// Reads actions for `player` alone: only their devices, and their contexts on top of the shared ones.
    pub fn player<'a>(
        &'a self,
        player: PlayerIndex,
        input: &'a Input,
        players: &Players,
    ) -> PlayerActions<'a> {
        PlayerActions {
            map: self,
            input,
            scope: Scope::player(player, players),
        }
    }

    /// Whether any live binding of `action` is held.
    pub fn is_held(&self, action: impl ActionLabel, input: &Input) -> bool {
        self.reading(action.intern(), input, Scope::SHARED).held
    }

    /// Whether a binding of `action` went down this frame while none of its bindings was already held.
    pub fn is_just_pressed(&self, action: impl ActionLabel, input: &Input) -> bool {
        self.reading(action.intern(), input, Scope::SHARED)
            .just_pressed()
    }

    /// Whether a binding of `action` went up this frame and none of its bindings is still held.
    pub fn is_just_released(&self, action: impl ActionLabel, input: &Input) -> bool {
        self.reading(action.intern(), input, Scope::SHARED)
            .just_released()
    }

    /// The two-dimensional value of `action`, of length at most 1.
    pub fn axis2d(&self, action: impl ActionLabel, input: &Input) -> Vec2 {
        self.axis2d_in(action.intern(), input, Scope::SHARED)
    }

    fn axis2d_in(&self, action: InternedActionLabel, input: &Input, scope: Scope) -> Vec2 {
        let mut seen = Vec::new();
        let mut keys = Vec2::ZERO;
        let mut stick = Vec2::ZERO;
        for binding in self.live(action, scope.player) {
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
                    if !scope.devices.keyboard_mouse() {
                        continue;
                    }
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
                    for pad in scope.devices.pads(input) {
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

    fn live(
        &self,
        action: InternedActionLabel,
        player: Option<PlayerIndex>,
    ) -> impl Iterator<Item = &Binding> + '_ {
        let player_stack = self.player_stack(player);
        self.bindings.iter().filter(move |binding| {
            binding.action == action
                && binding.context.is_none_or(|context| {
                    self.contexts.contains(&context) || player_stack.contains(&context)
                })
        })
    }

    fn reading(&self, action: InternedActionLabel, input: &Input, scope: Scope) -> ButtonReading {
        self.live(action, scope.player)
            .filter_map(|binding| match binding.input {
                BindingInput::Button(button) => {
                    Some(self.button_reading(button, input, scope.devices))
                }
                BindingInput::Axis2d(_) => None,
            })
            .fold(ButtonReading::default(), ButtonReading::merge)
    }

    fn button_reading(
        &self,
        button: ButtonBinding,
        input: &Input,
        devices: Devices,
    ) -> ButtonReading {
        match button {
            ButtonBinding::Key(_) | ButtonBinding::Mouse(_) if !devices.keyboard_mouse() => {
                ButtonReading::default()
            }
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
            ButtonBinding::Gamepad(button) => devices
                .pads(input)
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

/// One player's view of the actions, from [`ActionMap::player`].
pub struct PlayerActions<'a> {
    map: &'a ActionMap,
    input: &'a Input,
    scope: Scope,
}

impl PlayerActions<'_> {
    /// Whether any of this player's live bindings of `action` is held.
    pub fn is_held(&self, action: impl ActionLabel) -> bool {
        self.map
            .reading(action.intern(), self.input, self.scope)
            .held
    }

    /// Whether this player pressed `action` this frame while none of its bindings was already held.
    pub fn is_just_pressed(&self, action: impl ActionLabel) -> bool {
        self.map
            .reading(action.intern(), self.input, self.scope)
            .just_pressed()
    }

    /// Whether this player released `action` this frame and none of its bindings is still held.
    pub fn is_just_released(&self, action: impl ActionLabel) -> bool {
        self.map
            .reading(action.intern(), self.input, self.scope)
            .just_released()
    }

    /// The two-dimensional value of `action` from this player's devices, of length at most 1.
    pub fn axis2d(&self, action: impl ActionLabel) -> Vec2 {
        self.map.axis2d_in(action.intern(), self.input, self.scope)
    }

    /// The mouse motion this frame if this player holds the keyboard and mouse, otherwise zero.
    pub fn mouse_delta(&self) -> Vec2 {
        if self.scope.devices.keyboard_mouse() {
            self.input.mouse_delta()
        } else {
            Vec2::ZERO
        }
    }
}

fn push_onto(stack: &mut Vec<InternedContextLabel>, context: InternedContextLabel) {
    // Re-pushing moves a context to the top rather than stacking a
    // duplicate, so one stray push cannot outlive its pop.
    stack.retain(|active| *active != context);
    stack.push(context);
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
    players: concerto_ecs::resource::Res<Players>,
    mut writer: concerto_ecs::events::event_writer::EventWriter<ActionFired>,
) {
    for fired in map.fired(&input, &players) {
        writer.write(fired);
    }
}
