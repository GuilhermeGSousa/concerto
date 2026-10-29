use glam::Vec2;
use winit::{
    event::{
        ButtonSource, ElementState, MouseButton, MouseScrollDelta, PointerSource,
        WindowEvent as WinitEvent,
    },
    keyboard::PhysicalKey,
};

use concerto_window::events::{ScrollDelta, WindowEvent};

pub(crate) fn translate(event: &WinitEvent) -> Option<WindowEvent> {
    match event {
        WinitEvent::SurfaceResized(size) => Some(WindowEvent::Resized {
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
        WinitEvent::PointerButton { state, button, .. } => {
            mouse_button(button).map(|button| WindowEvent::MouseButton {
                button,
                pressed: *state == ElementState::Pressed,
            })
        }
        WinitEvent::PointerMoved {
            position, source, ..
        } => (!matches!(source, PointerSource::Touch { .. })).then(|| WindowEvent::CursorMoved {
            position: Vec2::new(position.x as f32, position.y as f32),
        }),
        WinitEvent::MouseWheel { delta, .. } => Some(WindowEvent::MouseWheel {
            delta: match delta {
                MouseScrollDelta::LineDelta(x, y) => ScrollDelta::Lines(Vec2::new(*x, *y)),
                MouseScrollDelta::PixelDelta(pixels) => {
                    ScrollDelta::Pixels(Vec2::new(pixels.x as f32, pixels.y as f32))
                }
                _ => return None,
            },
        }),
        _ => None,
    }
}

fn mouse_button(button: &ButtonSource) -> Option<MouseButton> {
    match button {
        ButtonSource::Touch { .. } => None,
        button => button.clone().mouse_button(),
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn is_mouse_left(button: &ButtonSource) -> bool {
    mouse_button(button) == Some(MouseButton::Left)
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
        event::{
            FingerId, ScrollSource, TabletToolButton, TabletToolData, TabletToolKind, TouchPhase,
        },
        keyboard::KeyCode,
    };

    const KEY: PhysicalKey = PhysicalKey::Code(KeyCode::KeyA);

    fn moved(source: PointerSource) -> WinitEvent {
        WinitEvent::PointerMoved {
            device_id: None,
            position: PhysicalPosition::new(3.0, 4.0),
            primary: true,
            source,
        }
    }

    fn pressed(button: ButtonSource) -> WinitEvent {
        WinitEvent::PointerButton {
            device_id: None,
            state: ElementState::Pressed,
            position: PhysicalPosition::new(3.0, 4.0),
            primary: true,
            button,
            is_macos_activation_click: false,
        }
    }

    fn touch() -> ButtonSource {
        ButtonSource::Touch {
            finger_id: FingerId::from_raw(0),
            force: None,
        }
    }

    fn pen(button: TabletToolButton) -> ButtonSource {
        ButtonSource::TabletTool {
            kind: TabletToolKind::Pen,
            button,
            data: TabletToolData::default(),
        }
    }

    fn wheel(delta: MouseScrollDelta) -> WinitEvent {
        WinitEvent::MouseWheel {
            device_id: None,
            delta,
            phase: TouchPhase::Moved,
            source: ScrollSource::Unknown,
        }
    }

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
            translate(&WinitEvent::SurfaceResized(PhysicalSize::new(800, 600))),
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
            translate(&WinitEvent::PointerButton {
                device_id: None,
                state: ElementState::Released,
                position: PhysicalPosition::new(3.0, 4.0),
                primary: true,
                button: ButtonSource::Mouse(MouseButton::Right),
                is_macos_activation_click: false,
            }),
            Some(WindowEvent::MouseButton {
                button: MouseButton::Right,
                pressed: false,
            })
        );
        assert_eq!(
            translate(&moved(PointerSource::Mouse)),
            Some(WindowEvent::CursorMoved {
                position: Vec2::new(3.0, 4.0),
            })
        );
    }

    #[test]
    fn a_pen_moves_the_cursor_as_the_mouse_does() {
        assert_eq!(
            translate(&moved(PointerSource::TabletTool {
                kind: TabletToolKind::Pen,
                data: TabletToolData::default(),
            })),
            Some(WindowEvent::CursorMoved {
                position: Vec2::new(3.0, 4.0),
            })
        );
    }

    #[test]
    fn a_touch_does_not_move_the_cursor() {
        assert_eq!(
            translate(&moved(PointerSource::Touch {
                finger_id: FingerId::from_raw(0),
                force: None,
            })),
            None
        );
    }

    #[test]
    fn a_mouse_or_pen_press_is_a_mouse_button() {
        let left = Some(WindowEvent::MouseButton {
            button: MouseButton::Left,
            pressed: true,
        });
        assert_eq!(
            translate(&pressed(ButtonSource::Mouse(MouseButton::Left))),
            left
        );
        assert_eq!(translate(&pressed(pen(TabletToolButton::Contact))), left);
    }

    #[test]
    fn a_touch_or_unknown_press_is_dropped() {
        assert_eq!(translate(&pressed(touch())), None);
        assert_eq!(translate(&pressed(ButtonSource::Unknown(7))), None);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_left_button_is_the_mouse_left_or_the_pen_tip() {
        assert!(is_mouse_left(&ButtonSource::Mouse(MouseButton::Left)));
        assert!(is_mouse_left(&pen(TabletToolButton::Contact)));
        assert!(!is_mouse_left(&touch()));
        assert!(!is_mouse_left(&ButtonSource::Mouse(MouseButton::Right)));
    }

    #[test]
    fn both_wheel_units_translate() {
        assert_eq!(
            translate(&wheel(MouseScrollDelta::LineDelta(1.0, -2.0))),
            Some(WindowEvent::MouseWheel {
                delta: ScrollDelta::Lines(Vec2::new(1.0, -2.0)),
            })
        );
        assert_eq!(
            translate(&wheel(MouseScrollDelta::PixelDelta(PhysicalPosition::new(
                5.0, 6.0
            )))),
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
            while_waiting(&WinitEvent::SurfaceResized(PhysicalSize::new(800, 600))),
            None
        );
        assert_eq!(while_waiting(&WinitEvent::Focused(true)), None);
        assert_eq!(
            while_waiting(&pressed(ButtonSource::Mouse(MouseButton::Left))),
            None
        );
        assert_eq!(while_waiting(&moved(PointerSource::Mouse)), None);
    }
}
