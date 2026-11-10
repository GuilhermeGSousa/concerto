use std::collections::HashSet;

use concerto_window::{
    events::WindowEvent,
    input::{GamepadAxis, GamepadButton, GamepadId},
};
use gilrs::{
    ev::filter::{axis_dpad_to_button, Jitter},
    Axis, Button, Event, EventType, Filter, Gilrs, GilrsBuilder,
};

const AXES: [GamepadAxis; 4] = [
    GamepadAxis::LeftStickX,
    GamepadAxis::LeftStickY,
    GamepadAxis::RightStickX,
    GamepadAxis::RightStickY,
];

pub(crate) struct Pads {
    gilrs: Gilrs,
    jitter: Jitter,
    told: HashSet<usize>,
    started: bool,
}

impl Pads {
    pub(crate) fn new() -> Option<Self> {
        let built = GilrsBuilder::new()
            .with_default_filters(false)
            .set_update_state(false)
            .with_force_feedback(false)
            .build();
        match built {
            Ok(gilrs) => Some(Self {
                gilrs,
                jitter: Jitter::new(),
                told: HashSet::new(),
                started: false,
            }),
            Err(error) => {
                log::warn!("Gamepads unavailable, running without them: {error}");
                None
            }
        }
    }

    pub(crate) fn drain(&mut self, focused: bool, pending: &mut Vec<WindowEvent>) {
        if !self.started {
            self.started = true;
            for (id, pad) in self.gilrs.gamepads() {
                self.told.insert(usize::from(id));
                pending.push(WindowEvent::GamepadConnected {
                    id: engine_id(id),
                    name: pad.name().to_owned(),
                });
            }
        }

        while let Some(event) = self.next_event() {
            let index = usize::from(event.id);
            match event.event {
                EventType::Connected => {
                    if self.told.insert(index) {
                        pending.push(WindowEvent::GamepadConnected {
                            id: engine_id(event.id),
                            name: self.gilrs.gamepad(event.id).name().to_owned(),
                        });
                    }
                }
                EventType::Disconnected => {
                    if self.told.remove(&index) {
                        pending.push(WindowEvent::GamepadDisconnected {
                            id: engine_id(event.id),
                        });
                    }
                }
                kind if focused && self.told.contains(&index) => {
                    pending.extend(translate(engine_id(event.id), kind));
                }
                _ => (),
            }
        }
    }

    pub(crate) fn send_state(&self, pending: &mut Vec<WindowEvent>) {
        for (id, pad) in self.gilrs.gamepads() {
            if !self.told.contains(&usize::from(id)) {
                continue;
            }
            let engine = engine_id(id);
            for button in GamepadButton::ALL {
                let value = pad
                    .button_data(gilrs_button(button))
                    .map_or(0.0, |data| data.value());
                pending.push(WindowEvent::GamepadButton {
                    id: engine,
                    button,
                    value,
                });
            }
            for axis in AXES {
                pending.push(WindowEvent::GamepadAxis {
                    id: engine,
                    axis,
                    value: pad.value(gilrs_axis(axis)),
                });
            }
        }
    }

    fn next_event(&mut self) -> Option<Event> {
        loop {
            let event = self
                .gilrs
                .next_event()
                .filter_ev(&axis_dpad_to_button, &mut self.gilrs)
                .filter_ev(&self.jitter, &mut self.gilrs)?;
            self.gilrs.update(&event);
            if !event.is_dropped() {
                return Some(event);
            }
        }
    }
}

fn engine_id(id: gilrs::GamepadId) -> GamepadId {
    GamepadId::new(usize::from(id) as u32)
}

pub(crate) fn translate(id: GamepadId, event: EventType) -> Option<WindowEvent> {
    match event {
        EventType::ButtonChanged(button, value, _) => {
            engine_button(button).map(|button| WindowEvent::GamepadButton { id, button, value })
        }
        EventType::AxisChanged(axis, value, _) => {
            engine_axis(axis).map(|axis| WindowEvent::GamepadAxis { id, axis, value })
        }
        _ => None,
    }
}

pub(crate) fn engine_button(button: Button) -> Option<GamepadButton> {
    Some(match button {
        Button::South => GamepadButton::South,
        Button::East => GamepadButton::East,
        Button::North => GamepadButton::North,
        Button::West => GamepadButton::West,
        Button::LeftTrigger => GamepadButton::LeftBumper,
        Button::RightTrigger => GamepadButton::RightBumper,
        Button::LeftTrigger2 => GamepadButton::LeftTrigger,
        Button::RightTrigger2 => GamepadButton::RightTrigger,
        Button::Select => GamepadButton::Select,
        Button::Start => GamepadButton::Start,
        Button::Mode => GamepadButton::Mode,
        Button::LeftThumb => GamepadButton::LeftThumb,
        Button::RightThumb => GamepadButton::RightThumb,
        Button::DPadUp => GamepadButton::DPadUp,
        Button::DPadDown => GamepadButton::DPadDown,
        Button::DPadLeft => GamepadButton::DPadLeft,
        Button::DPadRight => GamepadButton::DPadRight,
        Button::C | Button::Z | Button::Unknown => return None,
    })
}

pub(crate) fn gilrs_button(button: GamepadButton) -> Button {
    match button {
        GamepadButton::South => Button::South,
        GamepadButton::East => Button::East,
        GamepadButton::North => Button::North,
        GamepadButton::West => Button::West,
        GamepadButton::LeftBumper => Button::LeftTrigger,
        GamepadButton::RightBumper => Button::RightTrigger,
        GamepadButton::LeftTrigger => Button::LeftTrigger2,
        GamepadButton::RightTrigger => Button::RightTrigger2,
        GamepadButton::Select => Button::Select,
        GamepadButton::Start => Button::Start,
        GamepadButton::Mode => Button::Mode,
        GamepadButton::LeftThumb => Button::LeftThumb,
        GamepadButton::RightThumb => Button::RightThumb,
        GamepadButton::DPadUp => Button::DPadUp,
        GamepadButton::DPadDown => Button::DPadDown,
        GamepadButton::DPadLeft => Button::DPadLeft,
        GamepadButton::DPadRight => Button::DPadRight,
    }
}

pub(crate) fn engine_axis(axis: Axis) -> Option<GamepadAxis> {
    match axis {
        Axis::LeftStickX => Some(GamepadAxis::LeftStickX),
        Axis::LeftStickY => Some(GamepadAxis::LeftStickY),
        Axis::RightStickX => Some(GamepadAxis::RightStickX),
        Axis::RightStickY => Some(GamepadAxis::RightStickY),
        Axis::LeftZ | Axis::RightZ | Axis::DPadX | Axis::DPadY | Axis::Unknown => None,
    }
}

pub(crate) fn gilrs_axis(axis: GamepadAxis) -> Axis {
    match axis {
        GamepadAxis::LeftStickX => Axis::LeftStickX,
        GamepadAxis::LeftStickY => Axis::LeftStickY,
        GamepadAxis::RightStickX => Axis::RightStickX,
        GamepadAxis::RightStickY => Axis::RightStickY,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAD: GamepadId = GamepadId::new(2);

    fn code() -> gilrs::ev::Code {
        Button::South.to_nec().unwrap()
    }

    #[test]
    fn every_engine_input_maps_to_gilrs_and_back_and_the_rest_is_dropped() {
        for button in GamepadButton::ALL {
            assert_eq!(engine_button(gilrs_button(button)), Some(button));
        }
        for axis in AXES {
            assert_eq!(engine_axis(gilrs_axis(axis)), Some(axis));
        }
        assert_eq!(
            engine_button(Button::LeftTrigger),
            Some(GamepadButton::LeftBumper)
        );
        assert_eq!(
            engine_button(Button::LeftTrigger2),
            Some(GamepadButton::LeftTrigger)
        );
        for button in [Button::C, Button::Z, Button::Unknown] {
            assert_eq!(engine_button(button), None);
        }
        for axis in [
            Axis::LeftZ,
            Axis::RightZ,
            Axis::DPadX,
            Axis::DPadY,
            Axis::Unknown,
        ] {
            assert_eq!(engine_axis(axis), None);
        }
    }

    struct Change {
        event: EventType,
        engine: Option<WindowEvent>,
    }

    #[test]
    fn only_a_named_button_or_axis_change_becomes_an_engine_event_with_its_value() {
        let cases = [
            Change {
                event: EventType::ButtonChanged(Button::RightTrigger2, 0.4, code()),
                engine: Some(WindowEvent::GamepadButton {
                    id: PAD,
                    button: GamepadButton::RightTrigger,
                    value: 0.4,
                }),
            },
            Change {
                event: EventType::AxisChanged(Axis::LeftStickY, -0.7, code()),
                engine: Some(WindowEvent::GamepadAxis {
                    id: PAD,
                    axis: GamepadAxis::LeftStickY,
                    value: -0.7,
                }),
            },
            Change {
                event: EventType::ButtonPressed(Button::South, code()),
                engine: None,
            },
            Change {
                event: EventType::ButtonReleased(Button::South, code()),
                engine: None,
            },
            Change {
                event: EventType::ButtonChanged(Button::C, 1.0, code()),
                engine: None,
            },
            Change {
                event: EventType::AxisChanged(Axis::LeftZ, 0.5, code()),
                engine: None,
            },
            Change {
                event: EventType::Connected,
                engine: None,
            },
        ];
        for case in cases {
            let described = format!("{:?}", case.event);
            assert_eq!(translate(PAD, case.event), case.engine, "{described}");
        }
    }
}
