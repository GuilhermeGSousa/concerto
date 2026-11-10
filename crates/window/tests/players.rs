use concerto_app::{
    main_schedule::MainSchedulePlugin,
    plugins::TimePlugin,
    schedule_groups::{LateUpdate, Update},
    App,
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

#[derive(Resource, Default)]
struct PendingAssign {
    gamepad: Option<GamepadId>,
}

fn observe(mut events: EventReader<PlayerEvent>, mut seen: ResMut<Seen>) {
    seen.events.extend(events.read().copied());
}

fn observe_fired(mut fired: EventReader<ActionFired>, mut seen: ResMut<Fired>) {
    seen.actions.extend(fired.read().copied());
}

fn assign_pending(mut pending: ResMut<PendingAssign>, mut players: ResMut<Players>) {
    if let Some(gamepad) = pending.gamepad.take() {
        players.assign(P0, PlayerDevice::Gamepad(gamepad)).unwrap();
    }
}

fn app(settings: JoinSettings) -> App {
    let mut app = App::new();
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(TimePlugin)
        .register_plugin(WindowPlugin)
        .insert_resource(Players::new(settings))
        .insert_resource(Seen::default())
        .insert_resource(PendingAssign::default())
        .insert_resource(Fired::default())
        .add_system(Update, assign_pending)
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

#[test]
fn with_join_off_a_pad_pressing_south_joins_no_one() {
    let mut app = app(JoinSettings::new());
    connect(&mut app, PAD_A);
    app.update();

    tap(&mut app, PAD_A, GamepadButton::South);

    assert_eq!(players(&app).joined().count(), 0);
    assert!(events(&mut app).is_empty());
}

#[test]
fn two_pads_pressing_in_separate_frames_become_players_0_and_1() {
    let mut app = app(joining());
    connect(&mut app, PAD_A);
    connect(&mut app, PAD_B);
    app.update();

    tap(&mut app, PAD_B, GamepadButton::South);
    assert_eq!(events(&mut app), vec![joined(P0, PAD_B)]);
    tap(&mut app, PAD_A, GamepadButton::South);
    assert_eq!(events(&mut app), vec![joined(P1, PAD_A)]);

    assert_eq!(
        players(&app).player_of(PlayerDevice::Gamepad(PAD_B)),
        Some(P0)
    );
    assert_eq!(
        players(&app).player_of(PlayerDevice::Gamepad(PAD_A)),
        Some(P1)
    );
}

#[test]
fn two_pads_pressing_in_one_frame_join_in_connection_order() {
    let mut app = app(joining());
    connect(&mut app, PAD_A);
    connect(&mut app, PAD_B);
    app.update();

    button(&mut app, PAD_B, GamepadButton::South, true);
    button(&mut app, PAD_A, GamepadButton::South, true);
    app.update();

    assert_eq!(events(&mut app), vec![joined(P0, PAD_A), joined(P1, PAD_B)]);
}

#[test]
fn a_third_pad_is_refused_when_max_players_is_2() {
    let mut app = app(joining().with_max_players(2));
    connect(&mut app, PAD_A);
    connect(&mut app, PAD_B);
    connect(&mut app, PAD_C);
    app.update();

    tap(&mut app, PAD_A, GamepadButton::South);
    tap(&mut app, PAD_B, GamepadButton::South);
    events(&mut app);
    tap(&mut app, PAD_C, GamepadButton::South);

    assert!(events(&mut app).is_empty());
    assert_eq!(players(&app).joined().collect::<Vec<_>>(), vec![P0, P1]);
    assert_eq!(players(&app).player_of(PlayerDevice::Gamepad(PAD_C)), None);
}

#[test]
fn a_button_that_is_not_a_join_button_does_not_join() {
    let mut app = app(joining());
    connect(&mut app, PAD_A);
    app.update();

    tap(&mut app, PAD_A, GamepadButton::North);

    assert_eq!(players(&app).joined().count(), 0);
    assert!(events(&mut app).is_empty());
}

#[test]
fn a_disconnect_keeps_the_place_and_the_same_pad_restores_it() {
    let mut app = app(JoinSettings::new());
    connect(&mut app, PAD_A);
    app.get_resource_mut::<Players>()
        .unwrap()
        .assign(P0, PlayerDevice::Gamepad(PAD_A))
        .unwrap();
    app.update();
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
        players(&app).player_of(PlayerDevice::Gamepad(PAD_A)),
        Some(P0)
    );
    assert_eq!(
        events(&mut app),
        vec![PlayerEvent::DeviceRestored {
            player: P0,
            gamepad: PAD_A
        }]
    );
}

#[test]
fn a_new_pads_join_press_fills_a_waiting_place_not_a_new_one() {
    let mut app = app(joining());
    connect(&mut app, PAD_A);
    connect(&mut app, PAD_B);
    app.update();
    tap(&mut app, PAD_A, GamepadButton::South);
    disconnect(&mut app, PAD_A);
    app.update();
    events(&mut app);

    tap(&mut app, PAD_B, GamepadButton::South);

    assert_eq!(players(&app).joined().collect::<Vec<_>>(), vec![P0]);
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

#[test]
fn the_keyboard_joins_with_enter_only_when_keyboard_mouse_joins_is_on() {
    let mut app = app(joining());
    key(&mut app, KeyCode::Enter, true);
    app.update();
    key(&mut app, KeyCode::Enter, false);
    app.update();
    assert_eq!(players(&app).joined().count(), 0);

    app.get_resource_mut::<Players>()
        .unwrap()
        .set_settings(joining().with_keyboard_mouse_joins(true));
    key(&mut app, KeyCode::Enter, true);
    app.update();

    assert_eq!(
        events(&mut app),
        vec![PlayerEvent::Joined {
            player: P0,
            device: PlayerDevice::KeyboardMouse
        }]
    );
    assert_eq!(
        players(&app).player_of(PlayerDevice::KeyboardMouse),
        Some(P0)
    );
}

#[test]
fn the_keyboard_never_fills_a_waiting_place() {
    let mut app = app(joining().with_keyboard_mouse_joins(true));
    connect(&mut app, PAD_A);
    app.update();
    tap(&mut app, PAD_A, GamepadButton::South);
    disconnect(&mut app, PAD_A);
    app.update();
    events(&mut app);

    key(&mut app, KeyCode::Enter, true);
    app.update();

    assert!(players(&app).is_waiting(P0));
    assert!(!players(&app).devices(P0).unwrap().keyboard_mouse);
    assert_eq!(
        players(&app).player_of(PlayerDevice::KeyboardMouse),
        Some(P1)
    );
    assert_eq!(
        events(&mut app),
        vec![PlayerEvent::Joined {
            player: P1,
            device: PlayerDevice::KeyboardMouse
        }]
    );
}

#[test]
fn a_connect_then_a_disconnect_in_one_frame_leaves_the_player_waiting() {
    let mut app = app(JoinSettings::new());
    connect(&mut app, PAD_A);
    app.get_resource_mut::<Players>()
        .unwrap()
        .assign(P0, PlayerDevice::Gamepad(PAD_A))
        .unwrap();
    app.update();
    disconnect(&mut app, PAD_A);
    app.update();
    events(&mut app);

    connect(&mut app, PAD_A);
    disconnect(&mut app, PAD_A);
    app.update();

    assert!(players(&app).is_waiting(P0));
    assert_eq!(players(&app).devices(P0).unwrap().gamepad, None);
    assert_eq!(
        events(&mut app),
        vec![
            PlayerEvent::DeviceRestored {
                player: P0,
                gamepad: PAD_A
            },
            PlayerEvent::DeviceLost {
                player: P0,
                gamepad: PAD_A
            },
        ]
    );
}

#[test]
fn assigning_pad_a_to_player_1_clears_player_0s_wait_on_it() {
    let mut app = app(JoinSettings::new());
    connect(&mut app, PAD_A);
    app.get_resource_mut::<Players>()
        .unwrap()
        .assign(P0, PlayerDevice::Gamepad(PAD_A))
        .unwrap();
    app.update();
    disconnect(&mut app, PAD_A);
    app.update();
    events(&mut app);

    app.get_resource_mut::<Players>()
        .unwrap()
        .assign(P1, PlayerDevice::Gamepad(PAD_A))
        .unwrap();
    connect(&mut app, PAD_A);
    app.update();

    assert!(!players(&app).is_waiting(P0));
    assert_eq!(
        players(&app).player_of(PlayerDevice::Gamepad(PAD_A)),
        Some(P1)
    );
    assert_eq!(events(&mut app), vec![joined(P1, PAD_A)]);
}

#[test]
fn assign_from_a_system_shows_up_as_joined_in_the_next_frame() {
    let mut app = app(JoinSettings::new());
    app.update();

    app.get_resource_mut::<PendingAssign>().unwrap().gamepad = Some(PAD_A);
    app.update();
    assert!(events(&mut app).is_empty());
    assert_eq!(players(&app).joined().collect::<Vec<_>>(), vec![P0]);

    app.update();
    assert_eq!(events(&mut app), vec![joined(P0, PAD_A)]);
}

#[test]
fn a_waiting_player_above_a_lowered_max_players_still_gets_their_pad_back() {
    let mut app = app(JoinSettings::new());
    connect(&mut app, PAD_A);
    app.get_resource_mut::<Players>()
        .unwrap()
        .assign(P1, PlayerDevice::Gamepad(PAD_A))
        .unwrap();
    app.update();
    disconnect(&mut app, PAD_A);
    app.update();
    events(&mut app);

    app.get_resource_mut::<Players>()
        .unwrap()
        .set_settings(JoinSettings::new().with_max_players(1));
    connect(&mut app, PAD_A);
    app.update();

    assert!(!players(&app).is_waiting(P1));
    assert_eq!(
        players(&app).player_of(PlayerDevice::Gamepad(PAD_A)),
        Some(P1)
    );
    assert_eq!(
        events(&mut app),
        vec![PlayerEvent::DeviceRestored {
            player: P1,
            gamepad: PAD_A
        }]
    );
}

#[test]
fn the_press_that_joins_fires_its_action_for_the_new_player() {
    let mut app = app(joining());
    app.get_resource_mut::<ActionMap>()
        .unwrap()
        .bind_global(Jump, GamepadButton::South);
    connect(&mut app, PAD_A);
    app.update();

    tap(&mut app, PAD_A, GamepadButton::South);
    app.update();

    let fired = std::mem::take(&mut app.get_resource_mut::<Fired>().unwrap().actions);
    assert_eq!(
        fired,
        vec![ActionFired {
            gamepad: Some(PAD_A),
            player: Some(P0),
            ..ActionFired::new(Jump)
        }]
    );
}

#[test]
fn a_join_press_skips_a_waiting_place_above_a_lowered_max_players() {
    let mut app = app(joining());
    connect(&mut app, PAD_A);
    connect(&mut app, PAD_B);
    app.get_resource_mut::<Players>()
        .unwrap()
        .assign(P1, PlayerDevice::Gamepad(PAD_A))
        .unwrap();
    app.update();
    disconnect(&mut app, PAD_A);
    app.update();
    app.get_resource_mut::<Players>()
        .unwrap()
        .set_settings(joining().with_max_players(1));
    events(&mut app);

    tap(&mut app, PAD_B, GamepadButton::South);

    assert!(players(&app).is_waiting(P1));
    assert_eq!(
        players(&app).player_of(PlayerDevice::Gamepad(PAD_B)),
        Some(P0)
    );
    assert_eq!(events(&mut app), vec![joined(P0, PAD_B)]);
}
