//! Covers binding resolution: which action a key press means depends on the
//! context stack, and typing into a widget must never fire one.
use concerto_window::input::actions::{
    ActionFired, ActionLabel, ActionMap, Axis2dBinding, InternedActionLabel, Shortcut,
};
use concerto_window::input::{
    GamepadAxis, GamepadButton, GamepadId, GamepadStick, Input, KeyCode, MouseButton, PhysicalKey,
    PlayerDevice, PlayerIndex, Players,
};
use concerto_window::{define_action, define_context};
use glam::Vec2;

define_action!(Save);
define_action!(FrameSelected);
define_action!(DeleteEntity);
define_action!(DeleteFile);
define_action!(Jump);
define_action!(Move);
define_action!(Crouch);

define_context!(Viewport);
define_context!(Browser);
define_context!(Driving);

fn map() -> ActionMap {
    ActionMap::default()
}

struct Resolve {
    name: &'static str,
    setup: fn(&mut ActionMap),
    shortcut: Shortcut,
    action: Option<InternedActionLabel>,
}

fn context_bindings(map: &mut ActionMap) {
    map.bind(DeleteFile, Shortcut::key(KeyCode::Delete), Browser);
    map.bind(DeleteEntity, Shortcut::key(KeyCode::Delete), Viewport);
    map.bind(FrameSelected, Shortcut::key(KeyCode::KeyF), Viewport);
}

fn typing(map: &mut ActionMap) {
    map.bind_global(Save, Shortcut::ctrl(KeyCode::KeyS));
    map.bind_global(DeleteFile, Shortcut::key(KeyCode::KeyS));
    map.bind_global(Jump, Shortcut::key(KeyCode::Escape));
    map.set_capturing_text(true);
}

#[test]
fn a_shortcut_resolves_to_the_innermost_active_binding_and_typing_swallows_only_text() {
    let cases = [
        Resolve {
            name: "a global binding needs no context",
            setup: |map| map.bind_global(Save, Shortcut::ctrl(KeyCode::KeyS)),
            shortcut: Shortcut::ctrl(KeyCode::KeyS),
            action: Some(Save.intern()),
        },
        Resolve {
            name: "a missing modifier",
            setup: |map| map.bind_global(Save, Shortcut::ctrl(KeyCode::KeyS)),
            shortcut: Shortcut::key(KeyCode::KeyS),
            action: None,
        },
        Resolve {
            name: "an extra modifier",
            setup: |map| map.bind_global(Save, Shortcut::ctrl(KeyCode::KeyS)),
            shortcut: Shortcut::ctrl(KeyCode::KeyS).with_shift(),
            action: None,
        },
        Resolve {
            name: "the most recently pushed context claims a shared key",
            setup: |map| {
                context_bindings(map);
                map.push_context(Browser);
                map.push_context(Viewport);
            },
            shortcut: Shortcut::key(KeyCode::Delete),
            action: Some(DeleteEntity.intern()),
        },
        Resolve {
            name: "a context binding after its context is popped",
            setup: |map| {
                context_bindings(map);
                map.push_context(Viewport);
                map.pop_context(Viewport);
            },
            shortcut: Shortcut::key(KeyCode::KeyF),
            action: None,
        },
        Resolve {
            name: "an active context overrides a global default",
            setup: |map| {
                map.bind_global(DeleteFile, Shortcut::key(KeyCode::Delete));
                map.bind(DeleteEntity, Shortcut::key(KeyCode::Delete), Viewport);
                map.push_context(Viewport);
            },
            shortcut: Shortcut::key(KeyCode::Delete),
            action: Some(DeleteEntity.intern()),
        },
        Resolve {
            name: "a bare letter while typing",
            setup: typing,
            shortcut: Shortcut::key(KeyCode::KeyS),
            action: None,
        },
        Resolve {
            name: "a modified shortcut while typing",
            setup: typing,
            shortcut: Shortcut::ctrl(KeyCode::KeyS),
            action: Some(Save.intern()),
        },
        Resolve {
            name: "Escape while typing",
            setup: typing,
            shortcut: Shortcut::key(KeyCode::Escape),
            action: Some(Jump.intern()),
        },
    ];
    for case in cases {
        let mut map = map();
        (case.setup)(&mut map);
        assert_eq!(map.resolve(case.shortcut), case.action, "{}", case.name);
    }
}

const PAD: GamepadId = GamepadId::new(0);
const OTHER_PAD: GamepadId = GamepadId::new(1);

fn press(input: &mut Input, key: KeyCode) {
    input.update_key_input(PhysicalKey::Code(key), true);
}

fn input_with_pads() -> Input {
    let mut input = Input::new();
    input.connect_gamepad(PAD, "Test pad");
    input.connect_gamepad(OTHER_PAD, "Other pad");
    input
}

fn close(a: Vec2, b: Vec2) -> bool {
    (a - b).length() < 1e-5
}

#[test]
fn binding_adds_rebind_replaces_by_kind_and_unbind_removes_all() {
    let mut map = map();
    map.bind_global(Jump, Shortcut::key(KeyCode::Space));
    map.bind_global(Jump, Shortcut::key(KeyCode::KeyJ));
    map.bind_global(Jump, GamepadButton::South);
    assert_eq!(
        map.resolve(Shortcut::key(KeyCode::Space)),
        Some(Jump.intern())
    );
    assert_eq!(
        map.resolve(Shortcut::key(KeyCode::KeyJ)),
        Some(Jump.intern()),
        "binding a second key adds it beside the first"
    );

    map.rebind_global(Jump, Shortcut::key(KeyCode::KeyK));
    assert_eq!(map.resolve(Shortcut::key(KeyCode::Space)), None);
    assert_eq!(map.resolve(Shortcut::key(KeyCode::KeyJ)), None);
    assert_eq!(
        map.resolve(Shortcut::key(KeyCode::KeyK)),
        Some(Jump.intern())
    );
    let mut input = input_with_pads();
    input.update_gamepad_button(PAD, GamepadButton::South, 1.0);
    assert!(
        map.is_held(Jump, &input),
        "rebinding a key must leave the pad binding alone"
    );

    map.bind_axis2d_global(Jump, Axis2dBinding::wasd());
    map.unbind_global(Jump);
    press(&mut input, KeyCode::KeyW);
    assert_eq!(map.resolve(Shortcut::key(KeyCode::KeyK)), None);
    assert!(!map.is_held(Jump, &input));
    assert_eq!(
        map.axis2d(Jump, &input),
        Vec2::ZERO,
        "unbinding removes the 2D bindings too"
    );
}

fn movement_map() -> ActionMap {
    let mut map = map();
    map.bind_axis2d_global(Move, Axis2dBinding::wasd());
    map.bind_axis2d_global(Move, Axis2dBinding::arrows());
    map.bind_axis2d_global(Move, Axis2dBinding::Stick(GamepadStick::Left));
    map
}

struct Direction {
    name: &'static str,
    map: fn() -> ActionMap,
    input: fn(&mut Input),
    reads: Vec2,
}

fn viewport_movement() -> ActionMap {
    let mut map = map();
    map.bind_axis2d(Move, Axis2dBinding::wasd(), Viewport);
    map
}

fn typing_movement() -> ActionMap {
    let mut map = movement_map();
    map.set_capturing_text(true);
    map
}

fn half_pushed_stick(input: &mut Input) {
    input.update_gamepad_axis(PAD, GamepadAxis::LeftStickX, 0.55);
}

#[test]
fn a_two_dimensional_action_reads_the_longer_of_its_merged_keys_and_its_sticks() {
    let cases = [
        Direction {
            name: "keys across bindings merge into one normalized direction",
            map: movement_map,
            input: |input| {
                press(input, KeyCode::KeyW);
                press(input, KeyCode::ArrowLeft);
            },
            reads: Vec2::new(-1.0, 1.0).normalize(),
        },
        Direction {
            name: "the same direction from two bindings counts once",
            map: movement_map,
            input: |input| {
                press(input, KeyCode::KeyW);
                press(input, KeyCode::ArrowUp);
            },
            reads: Vec2::Y,
        },
        Direction {
            name: "a half-pushed stick with no key held",
            map: movement_map,
            input: half_pushed_stick,
            reads: Vec2::new(0.5, 0.0),
        },
        Direction {
            name: "on a tie the keys win and the stick is not added to them",
            map: movement_map,
            input: |input| {
                half_pushed_stick(input);
                press(input, KeyCode::KeyW);
                input.update_gamepad_axis(OTHER_PAD, GamepadAxis::LeftStickY, -1.0);
            },
            reads: Vec2::Y,
        },
        Direction {
            name: "a binding whose context is inactive",
            map: viewport_movement,
            input: |input| press(input, KeyCode::KeyD),
            reads: Vec2::ZERO,
        },
        Direction {
            name: "a text-like key while typing",
            map: typing_movement,
            input: |input| press(input, KeyCode::KeyW),
            reads: Vec2::ZERO,
        },
        Direction {
            name: "an arrow while typing",
            map: typing_movement,
            input: |input| {
                press(input, KeyCode::KeyW);
                press(input, KeyCode::ArrowRight);
            },
            reads: Vec2::X,
        },
    ];
    for case in cases {
        let map = (case.map)();
        let mut input = input_with_pads();
        (case.input)(&mut input);
        let read = map.axis2d(Move, &input);
        assert!(close(read, case.reads), "{}: {read:?}", case.name);
    }
}

#[test]
fn a_held_action_allows_extra_modifiers() {
    let mut map = map();
    map.bind_global(Crouch, Shortcut::key(KeyCode::KeyC));
    map.bind_global(Save, Shortcut::ctrl(KeyCode::KeyS));
    map.bind_global(Jump, MouseButton::Right);
    let mut input = Input::new();
    press(&mut input, KeyCode::ShiftLeft);
    press(&mut input, KeyCode::KeyC);
    press(&mut input, KeyCode::KeyS);

    assert!(map.is_held(Crouch, &input), "Shift+C still crouches");
    assert!(map.is_just_pressed(Crouch, &input));
    assert!(
        !map.is_held(Save, &input),
        "a modifier the binding names must be held"
    );
    assert!(!map.is_held(Jump, &input));
    input.update_mouse_button(MouseButton::Right, true);
    assert!(map.is_held(Jump, &input), "a mouse button is read as held");
}

#[test]
fn a_press_on_a_second_device_does_not_press_again() {
    let mut map = map();
    map.bind_global(Jump, Shortcut::key(KeyCode::Space));
    map.bind_global(Jump, GamepadButton::South);
    let mut input = input_with_pads();
    press(&mut input, KeyCode::Space);
    assert!(map.is_just_pressed(Jump, &input));
    input.update();

    input.update_gamepad_button(PAD, GamepadButton::South, 1.0);
    assert!(
        !map.is_just_pressed(Jump, &input),
        "the action was already held on the keyboard"
    );

    input.update_key_input(PhysicalKey::Code(KeyCode::Space), false);
    assert!(
        !map.is_just_released(Jump, &input),
        "the pad still holds it"
    );
    input.update();
    input.update_gamepad_button(PAD, GamepadButton::South, 0.0);
    assert!(map.is_just_released(Jump, &input));
    assert!(!map.is_held(Jump, &input));
}

const P0: PlayerIndex = PlayerIndex::new(0);
const P1: PlayerIndex = PlayerIndex::new(1);
const LONE_PAD: GamepadId = GamepadId::new(2);

fn two_players() -> Players {
    let mut players = Players::default();
    players.assign(P0, PlayerDevice::Gamepad(PAD)).unwrap();
    players.assign(P0, PlayerDevice::KeyboardMouse).unwrap();
    players
        .assign(P1, PlayerDevice::Gamepad(OTHER_PAD))
        .unwrap();
    players
}

fn input_with_three_pads() -> Input {
    let mut input = input_with_pads();
    input.connect_gamepad(LONE_PAD, "Lone pad");
    input
}

#[test]
fn each_players_stick_reads_only_their_own_pad() {
    let map = movement_map();
    let players = two_players();
    let mut input = input_with_three_pads();
    input.update_gamepad_axis(OTHER_PAD, GamepadAxis::LeftStickX, 1.0);

    assert_eq!(map.player(P0, &input, &players).axis2d(Move), Vec2::ZERO);
    assert!(close(
        map.player(P1, &input, &players).axis2d(Move),
        Vec2::X
    ));

    let mut input = input_with_three_pads();
    input.update_gamepad_axis(LONE_PAD, GamepadAxis::LeftStickY, 1.0);
    assert_eq!(
        map.player(P0, &input, &players).axis2d(Move),
        Vec2::ZERO,
        "an unassigned pad moves no player"
    );
    assert_eq!(map.player(P1, &input, &players).axis2d(Move), Vec2::ZERO);
    assert!(
        close(map.axis2d(Move, &input), Vec2::Y),
        "an unassigned pad still moves the read without a player"
    );
}

#[test]
fn a_key_binding_and_the_mouse_read_only_for_the_keyboard_and_mouse_player() {
    let mut map = movement_map();
    map.bind_global(Jump, Shortcut::key(KeyCode::Space));
    let players = two_players();
    let mut input = input_with_pads();
    press(&mut input, KeyCode::Space);
    press(&mut input, KeyCode::KeyD);

    let keyboard = map.player(P0, &input, &players);
    assert!(keyboard.is_held(Jump));
    assert!(keyboard.is_just_pressed(Jump));
    assert!(close(keyboard.axis2d(Move), Vec2::X));

    let pad_only = map.player(P1, &input, &players);
    assert!(!pad_only.is_held(Jump));
    assert!(!pad_only.is_just_pressed(Jump));
    assert_eq!(pad_only.axis2d(Move), Vec2::ZERO);

    input.update_mouse_delta((3.0, -2.0));
    assert_eq!(
        map.player(P0, &input, &players).mouse_delta(),
        Vec2::new(3.0, -2.0)
    );
    assert_eq!(map.player(P1, &input, &players).mouse_delta(), Vec2::ZERO);

    input.update();
    input.update_key_input(PhysicalKey::Code(KeyCode::Space), false);
    assert!(map.player(P0, &input, &players).is_just_released(Jump));
    assert!(!map.player(P1, &input, &players).is_just_released(Jump));
}

#[test]
fn a_players_context_changes_only_that_player_and_a_shared_one_both() {
    let mut map = map();
    map.bind(Jump, GamepadButton::South, Driving);
    let players = two_players();
    let mut input = input_with_pads();
    input.update_gamepad_button(PAD, GamepadButton::South, 1.0);
    input.update_gamepad_button(OTHER_PAD, GamepadButton::South, 1.0);

    map.push_context_for(P0, Driving);
    assert!(map.is_active_for(P0, Driving));
    assert!(!map.is_active_for(P1, Driving));
    assert!(!map.is_active(Driving));
    assert!(map.player(P0, &input, &players).is_held(Jump));
    assert!(
        !map.player(P1, &input, &players).is_held(Jump),
        "player 0's context must not reach player 1"
    );
    assert!(
        !map.is_held(Jump, &input),
        "a player's context does not reach the read without a player"
    );

    map.pop_context_for(P0, Driving);
    map.push_context(Driving);
    assert!(map.player(P0, &input, &players).is_held(Jump));
    assert!(
        map.player(P1, &input, &players).is_held(Jump),
        "a shared context applies to every player"
    );
}

#[test]
fn fired_carries_the_player_that_pressed() {
    let mut map = map();
    map.bind_global(Jump, GamepadButton::South);
    map.bind_global(Save, Shortcut::key(KeyCode::Enter));
    let players = two_players();
    let mut input = input_with_three_pads();
    input.update_gamepad_button(OTHER_PAD, GamepadButton::South, 1.0);
    input.update_gamepad_button(LONE_PAD, GamepadButton::South, 1.0);
    press(&mut input, KeyCode::Enter);

    let fired = map.fired(&input, &players);
    assert!(fired.contains(&ActionFired {
        action: Save.intern(),
        gamepad: None,
        player: Some(P0),
    }));
    assert!(fired.contains(&ActionFired {
        action: Jump.intern(),
        gamepad: Some(OTHER_PAD),
        player: Some(P1),
    }));
    assert!(
        fired.contains(&ActionFired {
            action: Jump.intern(),
            gamepad: Some(LONE_PAD),
            player: None,
        }),
        "an unassigned pad still fires, with no player"
    );
    assert_eq!(fired.len(), 3);
}

#[test]
fn a_players_context_beats_the_shared_one_when_firing() {
    let mut map = map();
    map.bind(DeleteFile, GamepadButton::South, Browser);
    map.bind(Jump, GamepadButton::South, Driving);
    map.bind_global(Crouch, GamepadButton::South);
    map.push_context_for(P1, Driving);
    map.push_context(Browser);
    let players = two_players();
    let mut input = input_with_pads();
    input.update_gamepad_button(PAD, GamepadButton::South, 1.0);
    input.update_gamepad_button(OTHER_PAD, GamepadButton::South, 1.0);

    assert_eq!(
        map.fired(&input, &players),
        vec![
            ActionFired {
                action: DeleteFile.intern(),
                gamepad: Some(PAD),
                player: Some(P0),
            },
            ActionFired {
                action: Jump.intern(),
                gamepad: Some(OTHER_PAD),
                player: Some(P1),
            },
        ],
        "player 1's own context is searched before the shared stack"
    );
}
