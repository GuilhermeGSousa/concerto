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

    struct Typed {
        state: ElementState,
        text: &'static str,
        carries: Option<&'static str>,
    }

    #[test]
    fn a_key_press_carries_its_text_without_control_characters_and_a_release_carries_none() {
        let cases = [
            Typed {
                state: ElementState::Pressed,
                text: "a",
                carries: Some("a"),
            },
            Typed {
                state: ElementState::Released,
                text: "a",
                carries: None,
            },
            Typed {
                state: ElementState::Pressed,
                text: "a\u{8}b",
                carries: Some("ab"),
            },
        ];
        for case in cases {
            assert_eq!(
                key(KEY, case.state, Some(case.text)),
                WindowEvent::Key {
                    key: KEY,
                    pressed: case.state == ElementState::Pressed,
                    text: case.carries.map(Into::into),
                },
                "{:?}",
                case.text
            );
        }
    }

    struct Translated {
        event: WinitEvent,
        engine: Option<WindowEvent>,
        waiting: Option<Waiting>,
    }

    fn left_press() -> Option<WindowEvent> {
        Some(WindowEvent::MouseButton {
            button: MouseButton::Left,
            pressed: true,
        })
    }

    fn cursor_moved() -> Option<WindowEvent> {
        Some(WindowEvent::CursorMoved {
            position: Vec2::new(3.0, 4.0),
        })
    }

    #[test]
    fn a_winit_event_becomes_its_engine_event_or_nothing_and_while_waiting_only_redraws_and_closes()
    {
        let cases = [
            Translated {
                event: WinitEvent::SurfaceResized(PhysicalSize::new(800, 600)),
                engine: Some(WindowEvent::Resized {
                    width: 800,
                    height: 600,
                }),
                waiting: None,
            },
            Translated {
                event: WinitEvent::CloseRequested,
                engine: Some(WindowEvent::CloseRequested),
                waiting: Some(Waiting::Exit),
            },
            Translated {
                event: WinitEvent::RedrawRequested,
                engine: None,
                waiting: Some(Waiting::Redraw),
            },
            Translated {
                event: pressed(ButtonSource::Mouse(MouseButton::Left)),
                engine: left_press(),
                waiting: None,
            },
            Translated {
                event: pressed(pen(TabletToolButton::Contact)),
                engine: left_press(),
                waiting: None,
            },
            Translated {
                event: pressed(touch()),
                engine: None,
                waiting: None,
            },
            Translated {
                event: moved(PointerSource::Mouse),
                engine: cursor_moved(),
                waiting: None,
            },
            Translated {
                event: moved(PointerSource::Touch {
                    finger_id: FingerId::from_raw(0),
                    force: None,
                }),
                engine: None,
                waiting: None,
            },
            Translated {
                event: wheel(MouseScrollDelta::LineDelta(1.0, -2.0)),
                engine: Some(WindowEvent::MouseWheel {
                    delta: ScrollDelta::Lines(Vec2::new(1.0, -2.0)),
                }),
                waiting: None,
            },
            Translated {
                event: wheel(MouseScrollDelta::PixelDelta(PhysicalPosition::new(
                    5.0, 6.0,
                ))),
                engine: Some(WindowEvent::MouseWheel {
                    delta: ScrollDelta::Pixels(Vec2::new(5.0, 6.0)),
                }),
                waiting: None,
            },
        ];
        for case in cases {
            assert_eq!(translate(&case.event), case.engine, "{:?}", case.event);
            assert_eq!(while_waiting(&case.event), case.waiting, "{:?}", case.event);
        }
    }
}
