pub mod actions;
mod gamepad;
mod players;
use concerto_ecs::resource::Resource;
use glam::Vec2;
use std::collections::HashMap;

pub use gamepad::{Gamepad, GamepadAxis, GamepadButton, GamepadId, GamepadSettings, GamepadStick};
pub(crate) use players::update_players;
pub use players::{
    AssignError, JoinSettings, PlayerDevice, PlayerDevices, PlayerEvent, PlayerIndex, Players,
};
pub use winit::event::MouseButton;
pub use winit::keyboard::{KeyCode, PhysicalKey};

#[derive(Clone, Copy, PartialEq)]
pub enum InputState {
    Pressed,
    Down,
    Released,
    Up,
}

#[derive(Clone, Copy, Default)]
struct ButtonState {
    down: bool,
    pressed: bool,
    released: bool,
}

impl ButtonState {
    fn press(&mut self) {
        if !self.down {
            self.down = true;
            self.pressed = true;
        }
    }

    fn release(&mut self) {
        if self.down {
            self.down = false;
            self.released = true;
        }
    }

    fn apply(&mut self, pressed: bool) {
        if pressed {
            self.press()
        } else {
            self.release()
        }
    }

    fn end_frame(&mut self) {
        self.pressed = false;
        self.released = false;
    }

    fn state(self) -> InputState {
        if self.pressed {
            InputState::Pressed
        } else if self.released {
            InputState::Released
        } else if self.down {
            InputState::Down
        } else {
            InputState::Up
        }
    }

    fn held(self) -> bool {
        self.down || self.pressed
    }
}

#[derive(Resource)]
pub struct Input {
    input_map: HashMap<winit::keyboard::PhysicalKey, ButtonState>,
    mouse_button_map: HashMap<MouseButton, ButtonState>,
    mouse_delta: Vec2,
    mouse_position: Vec2,
    typed_chars: Vec<char>,
    gamepads: Vec<Gamepad>,
    gamepad_settings: GamepadSettings,
}

impl Input {
    pub fn new() -> Self {
        Self {
            input_map: HashMap::new(),
            mouse_button_map: HashMap::new(),
            mouse_delta: Vec2::ZERO,
            mouse_position: Vec2::ZERO,
            typed_chars: Vec::new(),
            gamepads: Vec::new(),
            gamepad_settings: GamepadSettings::default(),
        }
    }

    pub fn get_key_state(&self, key: PhysicalKey) -> InputState {
        self.input_map
            .get(&key)
            .map_or(InputState::Up, |state| state.state())
    }

    /// Every key that entered the pressed state this frame.
    pub fn just_pressed_keys(&self) -> impl Iterator<Item = PhysicalKey> + '_ {
        self.input_map
            .iter()
            .filter(|(_, state)| state.pressed)
            .map(|(key, _)| *key)
    }

    pub fn is_just_pressed(&self, key: PhysicalKey) -> bool {
        self.input_map.get(&key).is_some_and(|state| state.pressed)
    }

    /// Every key held now, whether it went down this frame or earlier.
    pub fn held_keys(&self) -> impl Iterator<Item = PhysicalKey> + '_ {
        self.input_map
            .iter()
            .filter(|(_, state)| state.held())
            .map(|(key, _)| *key)
    }

    pub fn is_held(&self, key: PhysicalKey) -> bool {
        self.input_map.get(&key).is_some_and(|state| state.held())
    }

    pub fn is_just_released(&self, key: PhysicalKey) -> bool {
        self.input_map.get(&key).is_some_and(|state| state.released)
    }

    pub fn mouse_delta(&self) -> Vec2 {
        self.mouse_delta
    }

    pub fn mouse_position(&self) -> Vec2 {
        self.mouse_position
    }

    pub fn get_mouse_button_state(&self, button: MouseButton) -> InputState {
        self.mouse_button_map
            .get(&button)
            .map_or(InputState::Up, |state| state.state())
    }

    pub fn is_mouse_button_just_pressed(&self, button: MouseButton) -> bool {
        self.mouse_button_map
            .get(&button)
            .is_some_and(|state| state.pressed)
    }

    pub fn is_mouse_button_held(&self, button: MouseButton) -> bool {
        self.mouse_button_map
            .get(&button)
            .is_some_and(|state| state.held())
    }

    pub fn is_mouse_button_just_released(&self, button: MouseButton) -> bool {
        self.mouse_button_map
            .get(&button)
            .is_some_and(|state| state.released)
    }

    /// Characters typed this frame (text input, after modifier keys are applied).
    /// Cleared at the start of each frame by the `update_input` system.
    pub fn typed_chars(&self) -> &[char] {
        &self.typed_chars
    }

    pub fn push_typed_char(&mut self, c: char) {
        self.typed_chars.push(c);
    }

    /// Ends the frame: this frame's presses and releases become history.
    pub fn update(&mut self) {
        self.input_map.values_mut().for_each(ButtonState::end_frame);
        self.mouse_button_map
            .values_mut()
            .for_each(ButtonState::end_frame);
        self.mouse_delta = Vec2::ZERO;
        self.typed_chars.clear();
        self.gamepads.iter_mut().for_each(Gamepad::end_frame);
    }

    /// Releases every held key, mouse button and pad button and zeroes every pad axis, as when the window loses focus.
    pub fn release_all(&mut self) {
        self.release_keys_and_buttons();
        self.gamepads.iter_mut().for_each(Gamepad::release);
    }

    /// Releases every held key and mouse button and leaves pads alone.
    pub fn release_keys_and_buttons(&mut self) {
        self.input_map.values_mut().for_each(ButtonState::release);
        self.mouse_button_map
            .values_mut()
            .for_each(ButtonState::release);
    }

    /// The connected pads, in connection order.
    pub fn gamepads(&self) -> impl Iterator<Item = &Gamepad> {
        self.gamepads.iter()
    }

    pub fn gamepad(&self, id: GamepadId) -> Option<&Gamepad> {
        self.gamepads.iter().find(|pad| pad.id() == id)
    }

    /// The first connected pad.
    pub fn any_gamepad(&self) -> Option<&Gamepad> {
        self.gamepads.first()
    }

    pub fn gamepad_settings(&self) -> GamepadSettings {
        self.gamepad_settings
    }

    /// Applies `settings` to every pad, and to pads connected later.
    pub fn set_gamepad_settings(&mut self, settings: GamepadSettings) {
        self.gamepad_settings = settings;
        self.gamepads
            .iter_mut()
            .for_each(|pad| pad.set_settings(settings));
    }

    /// Adds a pad; returns false, changing nothing, when `id` is already connected.
    pub fn connect_gamepad(&mut self, id: GamepadId, name: impl Into<String>) -> bool {
        if self.gamepad(id).is_some() {
            return false;
        }
        self.gamepads
            .push(Gamepad::new(id, name.into(), self.gamepad_settings));
        true
    }

    /// Removes a pad; returns false when `id` was not connected.
    pub fn disconnect_gamepad(&mut self, id: GamepadId) -> bool {
        let before = self.gamepads.len();
        self.gamepads.retain(|pad| pad.id() != id);
        self.gamepads.len() != before
    }

    /// Sets a pad button's raw value; ignored for a pad that is not connected.
    pub fn update_gamepad_button(&mut self, id: GamepadId, button: GamepadButton, value: f32) {
        if let Some(pad) = self.gamepad_mut(id) {
            pad.update_button(button, value);
        }
    }

    /// Sets a pad axis's raw value in -1..1; ignored for a pad that is not connected.
    pub fn update_gamepad_axis(&mut self, id: GamepadId, axis: GamepadAxis, value: f32) {
        if let Some(pad) = self.gamepad_mut(id) {
            pad.update_axis(axis, value);
        }
    }

    fn gamepad_mut(&mut self, id: GamepadId) -> Option<&mut Gamepad> {
        self.gamepads.iter_mut().find(|pad| pad.id() == id)
    }

    pub fn update_key_input(&mut self, key: PhysicalKey, pressed: bool) {
        self.input_map.entry(key).or_default().apply(pressed);
    }

    pub fn update_mouse_position(&mut self, x: f64, y: f64) {
        self.mouse_position = Vec2::new(x as f32, y as f32);
    }

    pub fn update_mouse_button(&mut self, button: MouseButton, pressed: bool) {
        self.mouse_button_map
            .entry(button)
            .or_default()
            .apply(pressed);
    }

    /// Adds one raw mouse-motion event.
    pub fn update_mouse_delta(&mut self, delta: (f64, f64)) {
        self.mouse_delta += Vec2::new(delta.0 as f32, delta.1 as f32);
    }
}

impl Default for Input {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: PhysicalKey = PhysicalKey::Code(KeyCode::KeyW);

    #[test]
    fn held_keys_are_the_keys_down_or_pressed_this_frame() {
        let shift = PhysicalKey::Code(KeyCode::ShiftLeft);
        let tapped = PhysicalKey::Code(KeyCode::KeyQ);
        let mut input = Input::new();
        input.update_key_input(KEY, true);
        input.update_key_input(tapped, true);
        input.update_key_input(tapped, false);
        input.update();
        input.update_key_input(shift, true);

        let held: std::collections::HashSet<_> = input.held_keys().collect();

        assert_eq!(held, [KEY, shift].into());
    }

    #[test]
    fn a_tap_within_one_frame_is_seen_and_does_not_stick() {
        let mut input = Input::new();
        input.update_key_input(KEY, true);
        input.update_key_input(KEY, false);
        assert!(input.is_just_pressed(KEY));
        assert!(input.is_held(KEY));
        input.update();
        assert!(!input.is_held(KEY));
        assert!(input.get_key_state(KEY) == InputState::Up);
        input.update_mouse_button(MouseButton::Left, true);
        input.update_mouse_button(MouseButton::Left, false);
        assert!(input.is_mouse_button_just_pressed(MouseButton::Left));
        input.update();
        input.update_mouse_button(MouseButton::Left, true);
        assert!(input.is_mouse_button_just_pressed(MouseButton::Left));
    }
}
