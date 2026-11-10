//! Covers binding resolution: which action a key press means depends on the
//! context stack, and typing into a widget must never fire one.
use concerto_window::input::actions::{
    ActionFired, ActionLabel, ActionMap, Axis2dBinding, InternedActionLabel, Shortcut,
};
use concerto_window::input::{
    GamepadAxis, GamepadButton, GamepadId, GamepadStick, Input, KeyCode, MouseButton, PhysicalKey,
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
            name: "a context binding before its context is pushed",
            setup: context_bindings,
            shortcut: Shortcut::key(KeyCode::KeyF),
            action: None,
        },
        Resolve {
            name: "a context binding while its context is active",
            setup: |map| {
                context_bindings(map);
                map.push_context(Viewport);
            },
            shortcut: Shortcut::key(KeyCode::KeyF),
            action: Some(FrameSelected.intern()),
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
            name: "a context pushed twice and popped once",
            setup: |map| {
                context_bindings(map);
                map.push_context(Viewport);
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
        Resolve {
            name: "the shortcut a rebind replaced",
            setup: |map| {
                map.bind_global(Save, Shortcut::ctrl(KeyCode::KeyS));
                map.rebind_global(Save, Shortcut::ctrl(KeyCode::KeyW));
            },
            shortcut: Shortcut::ctrl(KeyCode::KeyS),
            action: None,
        },
        Resolve {
            name: "the shortcut a rebind moved to",
            setup: |map| {
                map.bind_global(Save, Shortcut::ctrl(KeyCode::KeyS));
                map.rebind_global(Save, Shortcut::ctrl(KeyCode::KeyW));
            },
            shortcut: Shortcut::ctrl(KeyCode::KeyW),
            action: Some(Save.intern()),
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
fn a_pad_button_fires_its_action_with_the_pads_id_under_context_rules() {
    let mut map = map();
    map.bind(Jump, GamepadButton::South, Viewport);
    let mut input = input_with_pads();
    input.update_gamepad_button(OTHER_PAD, GamepadButton::South, 1.0);

    assert!(
        map.fired(&input).is_empty(),
        "a context binding must not fire while its context is inactive"
    );

    map.push_context(Viewport);
    map.set_capturing_text(true);
    assert_eq!(
        map.fired(&input),
        vec![ActionFired {
            action: Jump.intern(),
            gamepad: Some(OTHER_PAD),
        }],
        "a pad press fires with the id of the pad that pressed, and is never text"
    );
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
            name: "a held key is longer than a half-pushed stick",
            map: movement_map,
            input: |input| {
                half_pushed_stick(input);
                press(input, KeyCode::KeyW);
            },
            reads: Vec2::Y,
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
            name: "a binding whose context is active",
            map: || {
                let mut map = viewport_movement();
                map.push_context(Viewport);
                map
            },
            input: |input| press(input, KeyCode::KeyD),
            reads: Vec2::X,
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
