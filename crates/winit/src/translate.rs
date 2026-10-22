use glam::Vec2;
use winit::{
    event::{ElementState, MouseScrollDelta, WindowEvent as WinitEvent},
    keyboard::PhysicalKey,
};

use concerto_window::events::{ScrollDelta, WindowEvent};

pub(crate) fn translate(event: &WinitEvent) -> Option<WindowEvent> {
    match event {
        WinitEvent::Resized(size) => Some(WindowEvent::Resized {
            width: size.width,
            height: size.height,
        }),
        WinitEvent::ScaleFactorChanged { scale_factor, .. } => {
            Some(WindowEvent::ScaleFactorChanged(*scale_factor))
        }
        WinitEvent::Focused(focused) => Some(WindowEvent::Focused(*focused)),
        WinitEvent::CloseRequested => Some(WindowEvent::CloseRequested),
        WinitEvent::KeyboardInput { event, .. } => {
            Some(key(event.physical_key, event.state, event.text.as_deref()))
        }
        WinitEvent::MouseInput { state, button, .. } => Some(WindowEvent::MouseButton {
            button: *button,
            pressed: *state == ElementState::Pressed,
        }),
        WinitEvent::CursorMoved { position, .. } => Some(WindowEvent::CursorMoved {
            position: Vec2::new(position.x as f32, position.y as f32),
        }),
        WinitEvent::MouseWheel { delta, .. } => Some(WindowEvent::MouseWheel {
            delta: match delta {
                MouseScrollDelta::LineDelta(x, y) => ScrollDelta::Lines(Vec2::new(*x, *y)),
                MouseScrollDelta::PixelDelta(pixels) => {
                    ScrollDelta::Pixels(Vec2::new(pixels.x as f32, pixels.y as f32))
                }
            },
        }),
        _ => None,
    }
}

pub(crate) fn key(key: PhysicalKey, state: ElementState, text: Option<&str>) -> WindowEvent {
    let pressed = state == ElementState::Pressed;
    let text = text
        .filter(|_| pressed)
        .map(|text| {
            text.chars()
                .filter(|character| !character.is_control())
                .collect::<String>()
        })
        .filter(|text| !text.is_empty());
    WindowEvent::Key { key, pressed, text }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Waiting {
    Redraw,
    Exit,
}

pub(crate) fn while_waiting(event: &WinitEvent) -> Option<Waiting> {
    match event {
        WinitEvent::RedrawRequested => Some(Waiting::Redraw),
        WinitEvent::CloseRequested => Some(Waiting::Exit),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::{
        dpi::{PhysicalPosition, PhysicalSize},
        event::{DeviceId, MouseButton, TouchPhase},
        keyboard::KeyCode,
    };

    const KEY: PhysicalKey = PhysicalKey::Code(KeyCode::KeyA);

    #[test]
    fn a_press_carries_its_text() {
        assert_eq!(
            key(KEY, ElementState::Pressed, Some("a")),
            WindowEvent::Key {
                key: KEY,
                pressed: true,
                text: Some("a".into()),
            }
        );
    }

    #[test]
    fn a_release_carries_no_text() {
        assert_eq!(
            key(KEY, ElementState::Released, Some("a")),
            WindowEvent::Key {
                key: KEY,
                pressed: false,
                text: None,
            }
        );
    }

    #[test]
    fn control_characters_are_dropped_from_text() {
        assert_eq!(
            key(KEY, ElementState::Pressed, Some("\r")),
            WindowEvent::Key {
                key: KEY,
                pressed: true,
                text: None,
            }
        );
        assert_eq!(
            key(KEY, ElementState::Pressed, Some("a\u{8}b")),
            WindowEvent::Key {
                key: KEY,
                pressed: true,
                text: Some("ab".into()),
            }
        );
    }

    #[test]
    fn window_facts_translate() {
        assert_eq!(
            translate(&WinitEvent::Resized(PhysicalSize::new(800, 600))),
            Some(WindowEvent::Resized {
                width: 800,
                height: 600,
            })
        );
        assert_eq!(
            translate(&WinitEvent::Focused(false)),
            Some(WindowEvent::Focused(false))
        );
        assert_eq!(
            translate(&WinitEvent::CloseRequested),
            Some(WindowEvent::CloseRequested)
        );
    }

    #[test]
    fn pointer_events_translate() {
        assert_eq!(
            translate(&WinitEvent::MouseInput {
                device_id: DeviceId::dummy(),
                state: ElementState::Released,
                button: MouseButton::Right,
            }),
            Some(WindowEvent::MouseButton {
                button: MouseButton::Right,
                pressed: false,
            })
        );
        assert_eq!(
            translate(&WinitEvent::CursorMoved {
                device_id: DeviceId::dummy(),
                position: PhysicalPosition::new(3.0, 4.0),
            }),
            Some(WindowEvent::CursorMoved {
                position: Vec2::new(3.0, 4.0),
            })
        );
    }

    #[test]
    fn both_wheel_units_translate() {
        assert_eq!(
            translate(&WinitEvent::MouseWheel {
                device_id: DeviceId::dummy(),
                delta: MouseScrollDelta::LineDelta(1.0, -2.0),
                phase: TouchPhase::Moved,
            }),
            Some(WindowEvent::MouseWheel {
                delta: ScrollDelta::Lines(Vec2::new(1.0, -2.0)),
            })
        );
        assert_eq!(
            translate(&WinitEvent::MouseWheel {
                device_id: DeviceId::dummy(),
                delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(5.0, 6.0)),
                phase: TouchPhase::Moved,
            }),
            Some(WindowEvent::MouseWheel {
                delta: ScrollDelta::Pixels(Vec2::new(5.0, 6.0)),
            })
        );
    }

    #[test]
    fn events_with_no_engine_meaning_are_dropped() {
        assert_eq!(translate(&WinitEvent::RedrawRequested), None);
    }

    #[test]
    fn a_redraw_is_asked_for_again_while_there_is_no_app() {
        assert_eq!(
            while_waiting(&WinitEvent::RedrawRequested),
            Some(Waiting::Redraw)
        );
    }

    #[test]
    fn the_close_button_exits_while_there_is_no_app() {
        assert_eq!(
            while_waiting(&WinitEvent::CloseRequested),
            Some(Waiting::Exit)
        );
    }

    #[test]
    fn input_and_window_facts_are_dropped_while_there_is_no_app() {
        assert_eq!(
            while_waiting(&WinitEvent::Resized(PhysicalSize::new(800, 600))),
            None
        );
        assert_eq!(while_waiting(&WinitEvent::Focused(true)), None);
        assert_eq!(
            while_waiting(&WinitEvent::MouseInput {
                device_id: DeviceId::dummy(),
                state: ElementState::Pressed,
                button: MouseButton::Left,
            }),
            None
        );
        assert_eq!(
            while_waiting(&WinitEvent::CursorMoved {
                device_id: DeviceId::dummy(),
                position: PhysicalPosition::new(3.0, 4.0),
            }),
            None
        );
    }
}
