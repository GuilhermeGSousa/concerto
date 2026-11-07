use concerto_ecs::{
    events::{event_reader::EventReader, event_writer::EventWriter, Event},
    resource::{Res, ResMut, Resource},
    system::input::SystemLocal,
    world::{FromWorld, World},
    SystemSet,
};
use glam::Vec2;

use crate::{
    input::{GamepadAxis, GamepadButton, GamepadId, Input, MouseButton, PhysicalKey},
    window::Window,
};

/// How far a wheel or trackpad scrolled.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScrollDelta {
    Lines(Vec2),
    Pixels(Vec2),
}

/// Something a host tells an `App` about its window or its input devices.
#[derive(Event, Clone, Debug, PartialEq)]
pub enum WindowEvent {
    Resized {
        width: u32,
        height: u32,
    },
    ScaleFactorChanged(f64),
    Focused(bool),
    Maximised(bool),
    CloseRequested,
    Key {
        key: PhysicalKey,
        pressed: bool,
        text: Option<String>,
    },
    MouseButton {
        button: MouseButton,
        pressed: bool,
    },
    /// Physical pixels from the top-left of the drawing area.
    CursorMoved {
        position: Vec2,
    },
    MouseMotion {
        delta: Vec2,
    },
    MouseWheel {
        delta: ScrollDelta,
    },
    GamepadConnected {
        id: GamepadId,
        name: String,
    },
    GamepadDisconnected {
        id: GamepadId,
    },
    /// The button's raw value, 0..1.
    GamepadButton {
        id: GamepadId,
        button: GamepadButton,
        value: f32,
    },
    /// The axis's raw value, -1..1.
    GamepadAxis {
        id: GamepadId,
        axis: GamepadAxis,
        value: f32,
    },
}

/// A pad that `Input` gained or lost this frame.
#[derive(Event, Clone, Debug, PartialEq)]
pub enum GamepadConnection {
    Connected { id: GamepadId, name: String },
    Disconnected { id: GamepadId },
}

/// The set that folds window events into `Input`; a system ordered before it decides this frame's [`InputGate`].
#[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
pub struct FoldInput;

/// While closed, an `App` folds no key, mouse button, wheel, or raw-motion events into its `Input`; pad events always fold.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputGate {
    pub open: bool,
}

impl Default for InputGate {
    fn default() -> Self {
        Self { open: true }
    }
}

#[derive(Default)]
pub(crate) struct GateWasClosed(bool);

impl FromWorld for GateWasClosed {
    fn from_world(_: &World) -> Self {
        Self::default()
    }
}

pub(crate) fn fold_input(
    mut events: EventReader<WindowEvent>,
    mut input: ResMut<Input>,
    gate: Res<InputGate>,
    mut was_closed: SystemLocal<GateWasClosed>,
    mut connections: EventWriter<GamepadConnection>,
) {
    let closed = !gate.open;
    if closed && !was_closed.0 {
        input.release_keys_and_buttons();
    }
    was_closed.0 = closed;
    for event in events.read() {
        match event {
            WindowEvent::Key { .. }
            | WindowEvent::MouseButton { .. }
            | WindowEvent::MouseMotion { .. }
            | WindowEvent::MouseWheel { .. }
                if closed => {}
            WindowEvent::Key { key, pressed, text } => {
                if let Some(text) = text {
                    for character in text.chars().filter(|character| !character.is_control()) {
                        input.push_typed_char(character);
                    }
                }
                if matches!(key, PhysicalKey::Code(_)) {
                    input.update_key_input(*key, *pressed);
                }
            }
            WindowEvent::MouseButton { button, pressed } => {
                input.update_mouse_button(*button, *pressed);
            }
            WindowEvent::CursorMoved { position } => {
                input.update_mouse_position(f64::from(position.x), f64::from(position.y));
            }
            WindowEvent::MouseMotion { delta } => {
                input.update_mouse_delta((f64::from(delta.x), f64::from(delta.y)));
            }
            WindowEvent::Focused(false) => input.release_all(),
            WindowEvent::GamepadConnected { id, name } => {
                if input.connect_gamepad(*id, name.clone()) {
                    connections.write(GamepadConnection::Connected {
                        id: *id,
                        name: name.clone(),
                    });
                }
            }
            WindowEvent::GamepadDisconnected { id } => {
                if input.disconnect_gamepad(*id) {
                    connections.write(GamepadConnection::Disconnected { id: *id });
                }
            }
            WindowEvent::GamepadButton { id, button, value } => {
                input.update_gamepad_button(*id, *button, *value);
            }
            WindowEvent::GamepadAxis { id, axis, value } => {
                input.update_gamepad_axis(*id, *axis, *value);
            }
            _ => {}
        }
    }
}

pub(crate) fn fold_window(mut events: EventReader<WindowEvent>, mut window: ResMut<Window>) {
    for event in events.read() {
        if Window::reports(event) {
            window.apply(event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{GamepadStick, KeyCode};
    use concerto_app::{
        main_schedule::MainSchedulePlugin,
        plugins::TimePlugin,
        schedule_groups::{First, LateUpdate, Main},
        App,
    };
    use concerto_ecs::{events::event_channel::EventChannel, Res, Resource};

    const KEY: PhysicalKey = PhysicalKey::Code(KeyCode::KeyW);
    const PAD: GamepadId = GamepadId::new(0);

    #[derive(Resource, Default)]
    struct Seen {
        key_pressed: bool,
        button_pressed: bool,
        typed: String,
        motion: Vec2,
        connections: Vec<GamepadConnection>,
    }

    fn observe(
        input: Res<Input>,
        mut connections: EventReader<GamepadConnection>,
        mut seen: ResMut<Seen>,
    ) {
        seen.key_pressed = input.is_just_pressed(KEY);
        seen.button_pressed = input.is_mouse_button_just_pressed(MouseButton::Left);
        seen.typed = input.typed_chars().iter().collect();
        seen.motion = input.mouse_delta();
        seen.connections.extend(connections.read().cloned());
    }

    fn end_frame(mut input: ResMut<Input>) {
        input.update();
    }

    fn app() -> App {
        let mut app = App::new();
        app.register_plugin(MainSchedulePlugin)
            .register_plugin(TimePlugin)
            .register_event::<WindowEvent>()
            .register_event::<GamepadConnection>()
            .insert_resource(Input::new())
            .insert_resource(InputGate::default())
            .insert_resource(Seen::default())
            .add_system(First, fold_input)
            .add_system(LateUpdate, observe)
            .add_system(Main, end_frame);
        app
    }

    fn push(app: &mut App, event: WindowEvent) {
        app.get_resource_mut::<EventChannel<WindowEvent>>()
            .unwrap()
            .push_event(event);
    }

    #[test]
    fn a_pushed_press_reads_as_just_pressed_through_late_update_then_as_held() {
        let mut app = app();
        push(
            &mut app,
            WindowEvent::Key {
                key: KEY,
                pressed: true,
                text: Some("w".into()),
            },
        );
        push(
            &mut app,
            WindowEvent::MouseButton {
                button: MouseButton::Left,
                pressed: true,
            },
        );
        app.finish_plugin_build();

        app.update();
        let seen = app.get_resource::<Seen>().unwrap();
        assert!(seen.key_pressed);
        assert!(seen.button_pressed);
        assert_eq!(seen.typed, "w");

        app.update();
        let seen = app.get_resource::<Seen>().unwrap();
        assert!(!seen.key_pressed);
        assert!(!seen.button_pressed);
        assert_eq!(seen.typed, "");
        let input = app.get_resource::<Input>().unwrap();
        assert!(input.is_held(KEY));
        assert!(input.is_mouse_button_held(MouseButton::Left));
    }

    #[test]
    fn cursor_and_motion_events_reach_input() {
        let mut app = app();
        push(
            &mut app,
            WindowEvent::CursorMoved {
                position: Vec2::new(30.0, 40.0),
            },
        );
        push(
            &mut app,
            WindowEvent::MouseMotion {
                delta: Vec2::new(2.0, -1.0),
            },
        );
        push(
            &mut app,
            WindowEvent::MouseMotion {
                delta: Vec2::new(1.0, 1.0),
            },
        );
        app.finish_plugin_build();
        app.update();

        assert_eq!(
            app.get_resource::<Input>().unwrap().mouse_position(),
            Vec2::new(30.0, 40.0)
        );
        assert_eq!(
            app.get_resource::<Seen>().unwrap().motion,
            Vec2::new(3.0, 0.0)
        );
    }

    #[test]
    fn a_release_event_releases_the_key() {
        let mut app = app();
        push(
            &mut app,
            WindowEvent::Key {
                key: KEY,
                pressed: true,
                text: None,
            },
        );
        app.finish_plugin_build();
        app.update();
        assert!(app.get_resource::<Input>().unwrap().is_held(KEY));
        push(
            &mut app,
            WindowEvent::Key {
                key: KEY,
                pressed: false,
                text: None,
            },
        );
        app.update();
        assert!(!app.get_resource::<Input>().unwrap().is_held(KEY));
    }

    #[test]
    fn a_pushed_resize_updates_the_window() {
        let mut app = app();
        app.insert_resource(Window::new((800, 600), 1.0))
            .add_system(First, fold_window);
        push(
            &mut app,
            WindowEvent::Resized {
                width: 320,
                height: 200,
            },
        );
        app.finish_plugin_build();
        app.update();
        assert_eq!(app.get_resource::<Window>().unwrap().size(), (320, 200));
    }

    #[test]
    fn losing_focus_releases_every_held_key_and_button() {
        let mut app = app();
        push(
            &mut app,
            WindowEvent::Key {
                key: KEY,
                pressed: true,
                text: None,
            },
        );
        push(
            &mut app,
            WindowEvent::MouseButton {
                button: MouseButton::Right,
                pressed: true,
            },
        );
        app.finish_plugin_build();
        app.update();
        let input = app.get_resource::<Input>().unwrap();
        assert!(input.is_held(KEY));
        assert!(input.is_mouse_button_held(MouseButton::Right));

        push(&mut app, WindowEvent::Focused(false));
        app.update();

        let input = app.get_resource::<Input>().unwrap();
        assert!(!input.is_held(KEY));
        assert!(!input.is_mouse_button_held(MouseButton::Right));
    }

    #[test]
    fn gaining_focus_leaves_input_alone() {
        let mut app = app();
        push(
            &mut app,
            WindowEvent::Key {
                key: KEY,
                pressed: true,
                text: None,
            },
        );
        app.finish_plugin_build();
        app.update();

        push(&mut app, WindowEvent::Focused(true));
        app.update();

        assert!(app.get_resource::<Input>().unwrap().is_held(KEY));
    }

    #[test]
    fn a_closed_gate_drops_keys_and_buttons_but_keeps_the_cursor() {
        let mut app = app();
        app.get_resource_mut::<InputGate>().unwrap().open = false;
        push(
            &mut app,
            WindowEvent::Key {
                key: KEY,
                pressed: true,
                text: Some("w".into()),
            },
        );
        push(
            &mut app,
            WindowEvent::MouseButton {
                button: MouseButton::Left,
                pressed: true,
            },
        );
        push(
            &mut app,
            WindowEvent::MouseMotion {
                delta: Vec2::new(2.0, 1.0),
            },
        );
        push(
            &mut app,
            WindowEvent::CursorMoved {
                position: Vec2::new(5.0, 6.0),
            },
        );
        app.finish_plugin_build();
        app.update();

        let seen = app.get_resource::<Seen>().unwrap();
        assert!(!seen.key_pressed);
        assert!(!seen.button_pressed);
        assert_eq!(seen.typed, "");
        assert_eq!(seen.motion, Vec2::ZERO);
        let input = app.get_resource::<Input>().unwrap();
        assert!(!input.is_held(KEY));
        assert_eq!(input.mouse_position(), Vec2::new(5.0, 6.0));
    }

    #[test]
    fn closing_the_gate_releases_held_input_on_the_next_fold() {
        let mut app = app();
        push(
            &mut app,
            WindowEvent::Key {
                key: KEY,
                pressed: true,
                text: None,
            },
        );
        app.finish_plugin_build();
        app.update();
        assert!(app.get_resource::<Input>().unwrap().is_held(KEY));

        app.get_resource_mut::<InputGate>().unwrap().open = false;
        app.update();
        assert!(!app.get_resource::<Input>().unwrap().is_held(KEY));
    }

    fn connect(app: &mut App) {
        push(
            app,
            WindowEvent::GamepadConnected {
                id: PAD,
                name: "Test pad".into(),
            },
        );
    }

    fn push_stick(app: &mut App, value: f32) {
        push(
            app,
            WindowEvent::GamepadAxis {
                id: PAD,
                axis: GamepadAxis::LeftStickX,
                value,
            },
        );
    }

    #[test]
    fn pad_events_fold_with_the_gate_closed() {
        let mut app = app();
        app.get_resource_mut::<InputGate>().unwrap().open = false;
        connect(&mut app);
        push(
            &mut app,
            WindowEvent::GamepadButton {
                id: PAD,
                button: GamepadButton::South,
                value: 1.0,
            },
        );
        push_stick(&mut app, 1.0);
        app.finish_plugin_build();
        app.update();

        let input = app.get_resource::<Input>().unwrap();
        let pad = input.gamepad(PAD).unwrap();
        assert!(pad.is_held(GamepadButton::South));
        assert_eq!(pad.stick(GamepadStick::Left), Vec2::new(1.0, 0.0));
    }

    #[test]
    fn closing_the_gate_keeps_a_held_sticks_value() {
        let mut app = app();
        connect(&mut app);
        push_stick(&mut app, 1.0);
        app.finish_plugin_build();
        app.update();

        app.get_resource_mut::<InputGate>().unwrap().open = false;
        app.update();

        let input = app.get_resource::<Input>().unwrap();
        assert_eq!(
            input.gamepad(PAD).unwrap().stick(GamepadStick::Left),
            Vec2::new(1.0, 0.0)
        );
    }

    #[test]
    fn connecting_and_disconnecting_write_one_connection_event_each() {
        let mut app = app();
        connect(&mut app);
        connect(&mut app);
        app.finish_plugin_build();
        app.update();
        push(&mut app, WindowEvent::GamepadDisconnected { id: PAD });
        push(&mut app, WindowEvent::GamepadDisconnected { id: PAD });
        app.update();

        assert_eq!(
            app.get_resource::<Seen>().unwrap().connections,
            vec![
                GamepadConnection::Connected {
                    id: PAD,
                    name: "Test pad".into(),
                },
                GamepadConnection::Disconnected { id: PAD },
            ]
        );
        assert!(app.get_resource::<Input>().unwrap().gamepad(PAD).is_none());
    }

    #[test]
    fn events_for_a_pad_that_is_not_connected_are_ignored() {
        let other = GamepadId::new(1);
        let mut app = app();
        connect(&mut app);
        push(
            &mut app,
            WindowEvent::GamepadButton {
                id: other,
                button: GamepadButton::South,
                value: 1.0,
            },
        );
        push(
            &mut app,
            WindowEvent::GamepadAxis {
                id: other,
                axis: GamepadAxis::LeftStickX,
                value: 1.0,
            },
        );
        app.finish_plugin_build();
        app.update();

        let input = app.get_resource::<Input>().unwrap();
        assert!(input.gamepad(other).is_none());
        assert_eq!(input.gamepads().count(), 1);
        let pad = input.gamepad(PAD).unwrap();
        assert!(!pad.is_held(GamepadButton::South));
        assert_eq!(pad.stick(GamepadStick::Left), Vec2::ZERO);
        assert!(app.get_resource::<Seen>().unwrap().connections.len() == 1);
    }

    #[test]
    fn reopening_the_gate_folds_input_again() {
        let mut app = app();
        app.get_resource_mut::<InputGate>().unwrap().open = false;
        app.finish_plugin_build();
        app.update();
        app.get_resource_mut::<InputGate>().unwrap().open = true;
        push(
            &mut app,
            WindowEvent::Key {
                key: KEY,
                pressed: true,
                text: None,
            },
        );
        app.update();
        assert!(app.get_resource::<Input>().unwrap().is_held(KEY));
    }
}
