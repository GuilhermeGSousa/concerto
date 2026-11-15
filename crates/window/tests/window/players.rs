use concerto_app::{
    main_schedule::MainSchedulePlugin, plugins::TimePlugin, schedule_groups::LateUpdate, App,
};
use concerto_ecs::{
    events::{event_channel::EventChannel, event_reader::EventReader},
    resource::{ResMut, Resource},
};
use concerto_window::define_action;
use concerto_window::events::WindowEvent;
use concerto_window::input::actions::{ActionFired, ActionMap};
use concerto_window::input::{
    GamepadButton, GamepadId, JoinSettings, KeyCode, PhysicalKey, PlayerDevice, PlayerEvent,
    PlayerIndex, Players,
};
use concerto_window::plugin::WindowPlugin;

const P0: PlayerIndex = PlayerIndex::new(0);
const P1: PlayerIndex = PlayerIndex::new(1);
const PAD_A: GamepadId = GamepadId::new(10);
const PAD_B: GamepadId = GamepadId::new(11);
const PAD_C: GamepadId = GamepadId::new(12);

#[derive(Resource, Default)]
struct Seen {
    events: Vec<PlayerEvent>,
}

define_action!(Jump);

#[derive(Resource, Default)]
struct Fired {
    actions: Vec<ActionFired>,
}

fn observe(mut events: EventReader<PlayerEvent>, mut seen: ResMut<Seen>) {
    seen.events.extend(events.read().copied());
}

fn observe_fired(mut fired: EventReader<ActionFired>, mut seen: ResMut<Fired>) {
    seen.actions.extend(fired.read().copied());
}

fn app(settings: JoinSettings) -> App {
    let mut app = App::new();
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(TimePlugin)
        .register_plugin(WindowPlugin)
        .insert_resource(Players::new(settings))
        .insert_resource(Seen::default())
        .insert_resource(Fired::default())
        .add_system(LateUpdate, observe)
        .add_system(LateUpdate, observe_fired);
    app.finish_plugin_build();
    app
}

fn joining() -> JoinSettings {
    JoinSettings::new().with_join_on_press(true)
}

fn push(app: &mut App, event: WindowEvent) {
    app.get_resource_mut::<EventChannel<WindowEvent>>()
        .unwrap()
        .push_event(event);
}

fn connect(app: &mut App, id: GamepadId) {
    push(
        app,
        WindowEvent::GamepadConnected {
            id,
            name: "Test pad".into(),
        },
    );
}

fn disconnect(app: &mut App, id: GamepadId) {
    push(app, WindowEvent::GamepadDisconnected { id });
}

fn button(app: &mut App, id: GamepadId, button: GamepadButton, pressed: bool) {
    push(
        app,
        WindowEvent::GamepadButton {
            id,
            button,
            value: if pressed { 1.0 } else { 0.0 },
        },
    );
}

fn tap(app: &mut App, id: GamepadId, pressed: GamepadButton) {
    button(app, id, pressed, true);
    app.update();
    button(app, id, pressed, false);
}

fn key(app: &mut App, code: KeyCode, pressed: bool) {
    push(
        app,
        WindowEvent::Key {
            key: PhysicalKey::Code(code),
            pressed,
            text: None,
        },
    );
}

fn events(app: &mut App) -> Vec<PlayerEvent> {
    std::mem::take(&mut app.get_resource_mut::<Seen>().unwrap().events)
}

fn players(app: &App) -> &Players {
    app.get_resource::<Players>().unwrap()
}

fn joined(player: PlayerIndex, gamepad: GamepadId) -> PlayerEvent {
    PlayerEvent::Joined {
        player,
        device: PlayerDevice::Gamepad(gamepad),
    }
}

fn fired(app: &mut App) -> Vec<ActionFired> {
    std::mem::take(&mut app.get_resource_mut::<Fired>().unwrap().actions)
}

#[test]
fn a_join_press_seats_pads_in_connection_order_up_to_max_players_and_fires_its_action() {
    let mut app = app(JoinSettings::new());
    app.get_resource_mut::<ActionMap>()
        .unwrap()
        .bind_global(Jump, GamepadButton::South);
    connect(&mut app, PAD_A);
    connect(&mut app, PAD_B);
    connect(&mut app, PAD_C);
    app.update();

    tap(&mut app, PAD_A, GamepadButton::South);
    assert_eq!(players(&app).joined().count(), 0);
    assert!(events(&mut app).is_empty());

    app.get_resource_mut::<Players>()
        .unwrap()
        .set_settings(joining().with_max_players(2));
    tap(&mut app, PAD_A, GamepadButton::North);
    assert!(events(&mut app).is_empty());
    fired(&mut app);

    button(&mut app, PAD_B, GamepadButton::South, true);
    button(&mut app, PAD_A, GamepadButton::South, true);
    app.update();
    app.update();
    assert_eq!(events(&mut app), vec![joined(P0, PAD_A), joined(P1, PAD_B)]);
    let fired = fired(&mut app);
    assert!(fired.contains(&ActionFired {
        gamepad: Some(PAD_A),
        player: Some(P0),
        ..ActionFired::new(Jump)
    }));
    assert!(fired.contains(&ActionFired {
        gamepad: Some(PAD_B),
        player: Some(P1),
        ..ActionFired::new(Jump)
    }));

    tap(&mut app, PAD_C, GamepadButton::South);
    assert!(events(&mut app).is_empty());
    assert_eq!(players(&app).joined().collect::<Vec<_>>(), vec![P0, P1]);
    assert_eq!(players(&app).player_of(PlayerDevice::Gamepad(PAD_C)), None);
}

#[test]
fn a_disconnected_pads_player_keeps_their_place_until_a_pad_returns_and_the_keyboard_never_fills_it(
) {
    let mut app = app(joining().with_keyboard_mouse_joins(true));
    connect(&mut app, PAD_A);
    connect(&mut app, PAD_B);
    app.update();
    tap(&mut app, PAD_A, GamepadButton::South);
    events(&mut app);

    disconnect(&mut app, PAD_A);
    app.update();
    assert_eq!(players(&app).joined().collect::<Vec<_>>(), vec![P0]);
    assert_eq!(players(&app).waiting().collect::<Vec<_>>(), vec![P0]);
    assert_eq!(
        events(&mut app),
        vec![PlayerEvent::DeviceLost {
            player: P0,
            gamepad: PAD_A
        }]
    );

    connect(&mut app, PAD_A);
    app.update();
    assert!(!players(&app).is_waiting(P0));
    assert_eq!(
        events(&mut app),
        vec![PlayerEvent::DeviceRestored {
            player: P0,
            gamepad: PAD_A
        }]
    );

    disconnect(&mut app, PAD_A);
    app.update();
    events(&mut app);
    key(&mut app, KeyCode::Enter, true);
    app.update();
    assert!(players(&app).is_waiting(P0));
    assert_eq!(
        events(&mut app),
        vec![PlayerEvent::Joined {
            player: P1,
            device: PlayerDevice::KeyboardMouse
        }]
    );

    tap(&mut app, PAD_B, GamepadButton::South);
    assert!(!players(&app).is_waiting(P0));
    assert_eq!(
        players(&app).player_of(PlayerDevice::Gamepad(PAD_B)),
        Some(P0)
    );
    assert_eq!(
        events(&mut app),
        vec![PlayerEvent::DeviceRestored {
            player: P0,
            gamepad: PAD_B
        }]
    );
}
