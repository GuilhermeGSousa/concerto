use glam::Vec2;
use std::collections::HashMap;

use crate::input::{ButtonState, InputState};

/// Names one connected gamepad.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct GamepadId(u32);

impl GamepadId {
    /// Creates the id the host gives the pad at `index`.
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    /// Returns the host's index for the pad.
    pub const fn index(self) -> u32 {
        self.0
    }
}

/// A gamepad button; `LeftTrigger` and `RightTrigger` are analog.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum GamepadButton {
    /// The bottom face button: A on Xbox, Cross on PlayStation.
    South,
    /// The right face button: B on Xbox, Circle on PlayStation.
    East,
    /// The top face button: Y on Xbox, Triangle on PlayStation.
    North,
    /// The left face button: X on Xbox, Square on PlayStation.
    West,
    /// The left shoulder button.
    LeftBumper,
    /// The right shoulder button.
    RightBumper,
    /// The left analog trigger.
    LeftTrigger,
    /// The right analog trigger.
    RightTrigger,
    /// The left centre button: View, Back or Share.
    Select,
    /// The right centre button: Menu, Start or Options.
    Start,
    /// The logo button in the middle of the pad.
    Mode,
    /// Pressing the left stick in.
    LeftThumb,
    /// Pressing the right stick in.
    RightThumb,
    /// Up on the directional pad.
    DPadUp,
    /// Down on the directional pad.
    DPadDown,
    /// Left on the directional pad.
    DPadLeft,
    /// Right on the directional pad.
    DPadRight,
}

impl GamepadButton {
    /// Every button, in declaration order.
    pub const ALL: [GamepadButton; 17] = [
        GamepadButton::South,
        GamepadButton::East,
        GamepadButton::North,
        GamepadButton::West,
        GamepadButton::LeftBumper,
        GamepadButton::RightBumper,
        GamepadButton::LeftTrigger,
        GamepadButton::RightTrigger,
        GamepadButton::Select,
        GamepadButton::Start,
        GamepadButton::Mode,
        GamepadButton::LeftThumb,
        GamepadButton::RightThumb,
        GamepadButton::DPadUp,
        GamepadButton::DPadDown,
        GamepadButton::DPadLeft,
        GamepadButton::DPadRight,
    ];

    fn is_trigger(self) -> bool {
        matches!(
            self,
            GamepadButton::LeftTrigger | GamepadButton::RightTrigger
        )
    }
}

/// One stick axis, in -1..1 with Y positive up.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum GamepadAxis {
    /// The left stick's horizontal axis, positive to the right.
    LeftStickX,
    /// The left stick's vertical axis, positive up.
    LeftStickY,
    /// The right stick's horizontal axis, positive to the right.
    RightStickX,
    /// The right stick's vertical axis, positive up.
    RightStickY,
}

impl GamepadAxis {
    /// Every axis, in declaration order.
    pub const ALL: [GamepadAxis; 4] = [
        GamepadAxis::LeftStickX,
        GamepadAxis::LeftStickY,
        GamepadAxis::RightStickX,
        GamepadAxis::RightStickY,
    ];
}

/// One of a gamepad's two sticks.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum GamepadStick {
    /// The left stick.
    Left,
    /// The right stick.
    Right,
}

/// How raw gamepad values become readings, shared by every pad.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct GamepadSettings {
    /// A stick shorter than this reads as zero.
    pub dead_zone: f32,
    /// A stick at least this long reads as length 1.
    pub live_zone: f32,
    /// A trigger at or above this value is down.
    pub trigger_threshold: f32,
}

impl Default for GamepadSettings {
    fn default() -> Self {
        Self {
            dead_zone: 0.15,
            live_zone: 0.95,
            trigger_threshold: 0.5,
        }
    }
}

impl GamepadSettings {
    /// Sets [`Self::dead_zone`].
    pub fn with_dead_zone(mut self, dead_zone: f32) -> Self {
        self.dead_zone = dead_zone;
        self
    }

    /// Sets [`Self::live_zone`].
    pub fn with_live_zone(mut self, live_zone: f32) -> Self {
        self.live_zone = live_zone;
        self
    }

    /// Sets [`Self::trigger_threshold`].
    pub fn with_trigger_threshold(mut self, trigger_threshold: f32) -> Self {
        self.trigger_threshold = trigger_threshold;
        self
    }

    fn shape(self, raw: Vec2) -> Vec2 {
        let length = raw.length();
        if length < self.dead_zone || length == 0.0 {
            return Vec2::ZERO;
        }
        if length >= self.live_zone {
            return raw / length;
        }
        let scaled = (length - self.dead_zone) / (self.live_zone - self.dead_zone);
        raw / length * scaled
    }

    fn is_down(self, button: GamepadButton, value: f32) -> bool {
        if button.is_trigger() {
            value >= self.trigger_threshold
        } else {
            value > 0.5
        }
    }
}

#[derive(Clone, Copy, Default)]
struct PadButton {
    value: f32,
    state: ButtonState,
}

/// One connected gamepad.
pub struct Gamepad {
    id: GamepadId,
    name: String,
    settings: GamepadSettings,
    buttons: HashMap<GamepadButton, PadButton>,
    left: Vec2,
    right: Vec2,
}

impl Gamepad {
    pub(crate) fn new(id: GamepadId, name: String, settings: GamepadSettings) -> Self {
        Self {
            id,
            name,
            settings,
            buttons: HashMap::new(),
            left: Vec2::ZERO,
            right: Vec2::ZERO,
        }
    }

    /// Returns the pad's id.
    pub fn id(&self) -> GamepadId {
        self.id
    }

    /// Returns the name the pad's driver reports.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns where `button` is in its press cycle this frame.
    pub fn button_state(&self, button: GamepadButton) -> InputState {
        self.buttons
            .get(&button)
            .map_or(InputState::Up, |pad| pad.state.state())
    }

    /// Whether `button` is held now, whether it went down this frame or earlier.
    pub fn is_held(&self, button: GamepadButton) -> bool {
        self.buttons
            .get(&button)
            .is_some_and(|pad| pad.state.held())
    }

    /// Whether `button` went down this frame.
    pub fn is_just_pressed(&self, button: GamepadButton) -> bool {
        self.buttons
            .get(&button)
            .is_some_and(|pad| pad.state.pressed)
    }

    /// Whether `button` came up this frame.
    pub fn is_just_released(&self, button: GamepadButton) -> bool {
        self.buttons
            .get(&button)
            .is_some_and(|pad| pad.state.released)
    }

    /// 0 or 1 for a digital button, 0..1 for a trigger.
    pub fn button_value(&self, button: GamepadButton) -> f32 {
        self.buttons.get(&button).map_or(0.0, |pad| pad.value)
    }

    /// The stick's position with the dead zone applied, length at most 1.
    pub fn stick(&self, stick: GamepadStick) -> Vec2 {
        let raw = match stick {
            GamepadStick::Left => self.left,
            GamepadStick::Right => self.right,
        };
        self.settings.shape(raw)
    }

    /// One component of the dead-zoned stick.
    pub fn axis(&self, axis: GamepadAxis) -> f32 {
        match axis {
            GamepadAxis::LeftStickX => self.stick(GamepadStick::Left).x,
            GamepadAxis::LeftStickY => self.stick(GamepadStick::Left).y,
            GamepadAxis::RightStickX => self.stick(GamepadStick::Right).x,
            GamepadAxis::RightStickY => self.stick(GamepadStick::Right).y,
        }
    }

    /// The axis's last raw value, before the dead zone.
    pub fn raw_axis(&self, axis: GamepadAxis) -> f32 {
        match axis {
            GamepadAxis::LeftStickX => self.left.x,
            GamepadAxis::LeftStickY => self.left.y,
            GamepadAxis::RightStickX => self.right.x,
            GamepadAxis::RightStickY => self.right.y,
        }
    }

    pub(crate) fn set_settings(&mut self, settings: GamepadSettings) {
        self.settings = settings;
    }

    pub(crate) fn update_button(&mut self, button: GamepadButton, value: f32) {
        let down = self.settings.is_down(button, value);
        let pad = self.buttons.entry(button).or_default();
        pad.value = value;
        pad.state.apply(down);
    }

    pub(crate) fn update_axis(&mut self, axis: GamepadAxis, value: f32) {
        match axis {
            GamepadAxis::LeftStickX => self.left.x = value,
            GamepadAxis::LeftStickY => self.left.y = value,
            GamepadAxis::RightStickX => self.right.x = value,
            GamepadAxis::RightStickY => self.right.y = value,
        }
    }

    pub(crate) fn end_frame(&mut self) {
        self.buttons
            .values_mut()
            .for_each(|pad| pad.state.end_frame());
    }

    pub(crate) fn release(&mut self) {
        self.buttons.values_mut().for_each(|pad| {
            pad.value = 0.0;
            pad.state.release();
        });
        self.left = Vec2::ZERO;
        self.right = Vec2::ZERO;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::Input;

    const PAD: GamepadId = GamepadId(0);

    fn input_with_pad() -> Input {
        let mut input = Input::new();
        input.connect_gamepad(PAD, "Test pad");
        input
    }

    fn the_pad(input: &Input) -> &Gamepad {
        input.gamepad(PAD).unwrap()
    }

    fn left_stick(input: &mut Input, raw: Vec2) -> Vec2 {
        input.update_gamepad_axis(PAD, GamepadAxis::LeftStickX, raw.x);
        input.update_gamepad_axis(PAD, GamepadAxis::LeftStickY, raw.y);
        the_pad(input).stick(GamepadStick::Left)
    }

    fn close(a: Vec2, b: Vec2) -> bool {
        (a - b).length() < 1e-5
    }

    struct Shaped {
        raw: Vec2,
        shaped: Vec2,
    }

    #[test]
    fn a_stick_reads_zero_in_the_dead_zone_one_past_the_live_zone_and_rescales_its_length_between()
    {
        let cases = [
            Shaped {
                raw: Vec2::new(0.1, 0.05),
                shaped: Vec2::ZERO,
            },
            Shaped {
                raw: Vec2::new(-1.0, 0.0),
                shaped: Vec2::new(-1.0, 0.0),
            },
            Shaped {
                raw: Vec2::new(0.55, 0.0),
                shaped: Vec2::new(0.5, 0.0),
            },
            Shaped {
                raw: Vec2::new(0.3, 0.4),
                shaped: Vec2::new(0.6, 0.8) * 0.4375,
            },
        ];
        let mut input = input_with_pad();
        for case in cases {
            let shaped = left_stick(&mut input, case.raw);
            assert!(
                close(shaped, case.shaped),
                "{:?} read as {shaped:?}",
                case.raw
            );
        }
        assert!((the_pad(&input).axis(GamepadAxis::LeftStickY) - 0.35).abs() < 1e-5);
    }

    #[test]
    fn a_trigger_goes_down_at_its_threshold() {
        let mut input = input_with_pad();
        input.update_gamepad_button(PAD, GamepadButton::LeftTrigger, 0.49);
        assert!(!the_pad(&input).is_held(GamepadButton::LeftTrigger));
        input.update_gamepad_button(PAD, GamepadButton::LeftTrigger, 0.5);
        let pad = the_pad(&input);
        assert!(pad.is_just_pressed(GamepadButton::LeftTrigger));
        assert_eq!(pad.button_value(GamepadButton::LeftTrigger), 0.5);
        input.update();

        input.set_gamepad_settings(GamepadSettings::default().with_trigger_threshold(0.8));
        input.update_gamepad_button(PAD, GamepadButton::LeftTrigger, 0.7);
        assert!(!the_pad(&input).is_held(GamepadButton::LeftTrigger));
    }

    #[test]
    fn release_all_zeroes_pads_and_keeps_them_connected() {
        let mut input = input_with_pad();
        input.update_gamepad_button(PAD, GamepadButton::RightTrigger, 1.0);
        left_stick(&mut input, Vec2::new(1.0, 0.0));
        input.update();

        input.release_all();

        let pad = the_pad(&input);
        assert!(!pad.is_held(GamepadButton::RightTrigger));
        assert!(pad.is_just_released(GamepadButton::RightTrigger));
        assert_eq!(pad.button_value(GamepadButton::RightTrigger), 0.0);
        assert_eq!(pad.stick(GamepadStick::Left), Vec2::ZERO);
    }
}
