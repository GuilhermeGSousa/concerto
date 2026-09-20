pub mod actions;
use cfg_if::cfg_if;
use ecs::resource::Resource;
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

#[derive(Resource)]
pub struct Input {
    input_map: HashMap<winit::keyboard::PhysicalKey, InputState>,
    mouse_button_map: HashMap<MouseButton, InputState>,
    mouse_delta: Vec2,
    mouse_position: Vec2,
    typed_chars: Vec<char>,
    #[cfg(target_arch = "wasm32")]
    previous_mouse_delta: Vec2,
}

impl Input {
    pub fn new() -> Self {
        Self {
            input_map: HashMap::new(),
            mouse_button_map: HashMap::new(),
            mouse_delta: Vec2::ZERO,
            mouse_position: Vec2::ZERO,
            typed_chars: Vec::new(),
            #[cfg(target_arch = "wasm32")]
            previous_mouse_delta: Vec2::ZERO,
        }
    }

    pub fn get_key_state(&self, key: PhysicalKey) -> InputState {
        match self.input_map.get(&key) {
            Some(state) => *state,
            None => InputState::Up,
        }
    }

    /// Every key that entered the pressed state this frame.
    pub fn just_pressed_keys(&self) -> impl Iterator<Item = PhysicalKey> + '_ {
        self.input_map
            .iter()
            .filter(|(_, state)| **state == InputState::Pressed)
            .map(|(key, _)| *key)
    }

    pub fn is_just_pressed(&self, key: PhysicalKey) -> bool {
        self.get_key_state(key) == InputState::Pressed
    }

    pub fn is_held(&self, key: PhysicalKey) -> bool {
        matches!(
            self.get_key_state(key),
            InputState::Pressed | InputState::Down
        )
    }

    pub fn is_just_released(&self, key: PhysicalKey) -> bool {
        self.get_key_state(key) == InputState::Released
    }

    pub fn mouse_delta(&self) -> Vec2 {
        self.mouse_delta
    }

    pub fn mouse_position(&self) -> Vec2 {
        self.mouse_position
    }

    pub fn get_mouse_button_state(&self, button: MouseButton) -> InputState {
        match self.mouse_button_map.get(&button) {
            Some(state) => *state,
            None => InputState::Up,
        }
    }

    pub fn is_mouse_button_just_pressed(&self, button: MouseButton) -> bool {
        self.get_mouse_button_state(button) == InputState::Pressed
    }

    pub fn is_mouse_button_held(&self, button: MouseButton) -> bool {
        matches!(
            self.get_mouse_button_state(button),
            InputState::Pressed | InputState::Down
        )
    }

    pub fn is_mouse_button_just_released(&self, button: MouseButton) -> bool {
        self.get_mouse_button_state(button) == InputState::Released
    }

    /// Characters typed this frame (text input, after modifier keys are applied).
    /// Cleared at the start of each frame by the `update_input` system.
    pub fn typed_chars(&self) -> &[char] {
        &self.typed_chars
    }

    pub fn push_typed_char(&mut self, c: char) {
        self.typed_chars.push(c);
    }

    pub fn update(&mut self) {
        for state in self.input_map.values_mut() {
            match state {
                InputState::Pressed => *state = InputState::Down,
                InputState::Released => *state = InputState::Up,
                _ => {}
            }
        }
        for state in self.mouse_button_map.values_mut() {
            match state {
                InputState::Pressed => *state = InputState::Down,
                InputState::Released => *state = InputState::Up,
                _ => {}
            }
        }
        self.mouse_delta = Vec2::ZERO;
        self.typed_chars.clear();
    }

    pub fn update_key_input(&mut self, key: PhysicalKey, state: ElementState) {
        if let std::collections::hash_map::Entry::Vacant(e) = self.input_map.entry(key) {
            match state {
                ElementState::Pressed => {
                    e.insert(InputState::Pressed);
                }
                ElementState::Released => {
                    e.insert(InputState::Released);
                }
            };
            return;
        }

        match state {
            ElementState::Pressed => {
                if self.input_map.get(&key) == Some(&InputState::Up) {
                    self.input_map.insert(key, InputState::Pressed);
                }
            }
            ElementState::Released => {
                if self.input_map.get(&key) == Some(&InputState::Down) {
                    self.input_map.insert(key, InputState::Released);
                }
            }
        }
    }

    pub fn update_mouse_position(&mut self, x: f64, y: f64) {
        self.mouse_position = Vec2::new(x as f32, y as f32);
    }

    pub fn update_mouse_button(&mut self, button: MouseButton, state: ElementState) {
        if let std::collections::hash_map::Entry::Vacant(e) = self.mouse_button_map.entry(button) {
            match state {
                ElementState::Pressed => {
                    e.insert(InputState::Pressed);
                }
                ElementState::Released => {
                    e.insert(InputState::Released);
                }
            };
            return;
        }

        match state {
            ElementState::Pressed => {
                if self.mouse_button_map.get(&button) == Some(&InputState::Up) {
                    self.mouse_button_map.insert(button, InputState::Pressed);
                }
            }
            ElementState::Released => {
                if self.mouse_button_map.get(&button) == Some(&InputState::Down) {
                    self.mouse_button_map.insert(button, InputState::Released);
                }
            }
        }
    }

    pub fn update_mouse_delta(&mut self, delta: (f64, f64)) {
        let delta = Vec2::new(delta.0 as f32, delta.1 as f32);

        cfg_if! {
            if #[cfg(target_arch = "wasm32")] {
                self.mouse_delta = delta - self.previous_mouse_delta;
                self.previous_mouse_delta = delta;
            }
            else
            {
                self.mouse_delta = delta;
            }
        }
    }
}

impl Default for Input {
    fn default() -> Self {
        Self::new()
    }
}
