pub mod actions;
use concerto_ecs::resource::Resource;
use glam::Vec2;
use std::collections::HashMap;
use winit::event::ElementState;

pub use winit::event::MouseButton;
pub use winit::keyboard::{KeyCode, PhysicalKey};

#[derive(Clone, Copy, PartialEq)]
pub enum InputState {
    Pressed,
    Down,
    Released,
    Up,
}

/// One key or button: whether it is held, and what happened to it this frame.
///
/// Presses and releases are recorded as events rather than folded into a
/// single state, so a tap that goes down *and* up within one frame (a quick
/// click, a touchpad tap, any input during a slow frame) is still seen as
/// pressed that frame, and its release is never lost.
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

    fn apply(&mut self, state: ElementState) {
        match state {
            ElementState::Pressed => self.press(),
            ElementState::Released => self.release(),
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

    /// Held now, or tapped at some point this frame.
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
}

impl Input {
    pub fn new() -> Self {
        Self {
            input_map: HashMap::new(),
            mouse_button_map: HashMap::new(),
            mouse_delta: Vec2::ZERO,
            mouse_position: Vec2::ZERO,
            typed_chars: Vec::new(),
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
    }

    pub fn update_key_input(&mut self, key: PhysicalKey, state: ElementState) {
        self.input_map.entry(key).or_default().apply(state);
    }

    pub fn update_mouse_position(&mut self, x: f64, y: f64) {
        self.mouse_position = Vec2::new(x as f32, y as f32);
    }

    pub fn update_mouse_button(&mut self, button: MouseButton, state: ElementState) {
        self.mouse_button_map
            .entry(button)
            .or_default()
            .apply(state);
    }

    /// Adds one raw mouse-motion event. Several can arrive per frame (high
    /// polling-rate mice, or a slow frame), so they accumulate until
    /// [`update`](Self::update) clears them. Every platform, the web included,
    /// reports relative motion here (`movementX/Y` in browsers).
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
    fn a_tap_within_one_frame_is_seen_and_does_not_stick() {
        let mut input = Input::new();
        input.update_key_input(KEY, ElementState::Pressed);
        input.update_key_input(KEY, ElementState::Released);
        assert!(input.is_just_pressed(KEY));
        assert!(input.is_held(KEY));
        input.update();
        assert!(!input.is_held(KEY));
        assert!(input.get_key_state(KEY) == InputState::Up);
        // And the next tap is seen too.
        input.update_mouse_button(MouseButton::Left, ElementState::Pressed);
        input.update_mouse_button(MouseButton::Left, ElementState::Released);
        assert!(input.is_mouse_button_just_pressed(MouseButton::Left));
        input.update();
        input.update_mouse_button(MouseButton::Left, ElementState::Pressed);
        assert!(input.is_mouse_button_just_pressed(MouseButton::Left));
    }

    #[test]
    fn held_keys_go_pressed_down_released_up() {
        let mut input = Input::new();
        input.update_key_input(KEY, ElementState::Pressed);
        assert!(input.get_key_state(KEY) == InputState::Pressed);
        input.update();
        assert!(input.get_key_state(KEY) == InputState::Down);
        // Key repeat sends more presses while held; they are not new presses.
        input.update_key_input(KEY, ElementState::Pressed);
        assert!(!input.is_just_pressed(KEY));
        input.update_key_input(KEY, ElementState::Released);
        assert!(input.is_just_released(KEY));
        input.update();
        assert!(input.get_key_state(KEY) == InputState::Up);
    }

    #[test]
    fn mouse_motion_accumulates_within_a_frame() {
        let mut input = Input::new();
        input.update_mouse_delta((3.0, -1.0));
        input.update_mouse_delta((2.0, 4.0));
        assert_eq!(input.mouse_delta(), Vec2::new(5.0, 3.0));
        input.update();
        assert_eq!(input.mouse_delta(), Vec2::ZERO);
    }
}
