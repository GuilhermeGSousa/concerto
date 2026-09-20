//! Covers binding resolution: which action a key press means depends on the
//! context stack, and typing into a widget must never fire one.
use window::input::actions::{ActionLabel, ActionMap, Modifiers, Shortcut};
use window::input::KeyCode;
use window::{define_action, define_context};

define_action!(Save);
define_action!(FrameSelected);
define_action!(DeleteEntity);
define_action!(DeleteFile);

define_context!(Viewport);
define_context!(Browser);

fn map() -> ActionMap {
    ActionMap::default()
}

#[test]
fn a_global_binding_fires_with_no_context_pushed() {
    let mut map = map();
    map.bind_global(Save, Shortcut::ctrl(KeyCode::KeyS));

    assert_eq!(
        map.resolve(Shortcut::ctrl(KeyCode::KeyS)),
        Some(Save.intern()),
        "a global binding must not need a context to be active"
    );
}

#[test]
fn modifiers_must_match_exactly() {
    let mut map = map();
    map.bind_global(Save, Shortcut::ctrl(KeyCode::KeyS));

    assert_eq!(
        map.resolve(Shortcut::key(KeyCode::KeyS)),
        None,
        "a bare S must not fire a binding that asked for Ctrl+S"
    );
    assert_eq!(
        map.resolve(Shortcut::ctrl(KeyCode::KeyS).with_shift()),
        None,
        "an extra modifier makes it a different shortcut"
    );
}

#[test]
fn the_innermost_context_wins() {
    let mut map = map();
    map.bind(DeleteFile, Shortcut::key(KeyCode::Delete), Browser);
    map.bind(DeleteEntity, Shortcut::key(KeyCode::Delete), Viewport);

    map.push_context(Browser);
    map.push_context(Viewport);

    assert_eq!(
        map.resolve(Shortcut::key(KeyCode::Delete)),
        Some(DeleteEntity.intern()),
        "the most recently pushed context claims a shared key"
    );
}

#[test]
fn a_context_binding_is_inert_while_its_context_is_not_active() {
    let mut map = map();
    map.bind(FrameSelected, Shortcut::key(KeyCode::KeyF), Viewport);

    assert_eq!(
        map.resolve(Shortcut::key(KeyCode::KeyF)),
        None,
        "F means nothing until the viewport is the active context"
    );

    map.push_context(Viewport);
    assert_eq!(
        map.resolve(Shortcut::key(KeyCode::KeyF)),
        Some(FrameSelected.intern())
    );

    map.pop_context(Viewport);
    assert_eq!(
        map.resolve(Shortcut::key(KeyCode::KeyF)),
        None,
        "popping the context must retire its bindings again"
    );
}

#[test]
fn a_context_binding_beats_a_global_one() {
    let mut map = map();
    map.bind_global(DeleteFile, Shortcut::key(KeyCode::Delete));
    map.bind(DeleteEntity, Shortcut::key(KeyCode::Delete), Viewport);
    map.push_context(Viewport);

    assert_eq!(
        map.resolve(Shortcut::key(KeyCode::Delete)),
        Some(DeleteEntity.intern()),
        "an active context must be able to override a global default"
    );
}

#[test]
fn typing_swallows_unmodified_keys_but_not_shortcuts() {
    let mut map = map();
    map.bind_global(Save, Shortcut::ctrl(KeyCode::KeyS));
    map.bind_global(DeleteFile, Shortcut::key(KeyCode::KeyS));
    map.set_capturing_text(true);

    assert_eq!(
        map.resolve(Shortcut::key(KeyCode::KeyS)),
        None,
        "a bare letter belongs to the text field being typed into"
    );
    assert_eq!(
        map.resolve(Shortcut::ctrl(KeyCode::KeyS)),
        Some(Save.intern()),
        "a modified shortcut must still reach the application while typing"
    );
}

#[test]
fn typing_does_not_swallow_navigation_keys() {
    let mut map = map();
    map.bind_global(Save, Shortcut::key(KeyCode::Escape));
    map.set_capturing_text(true);

    assert_eq!(
        map.resolve(Shortcut::key(KeyCode::Escape)),
        Some(Save.intern()),
        "Escape is not text; a field must not eat the way out of itself"
    );
}

#[test]
fn rebinding_an_action_replaces_the_old_shortcut() {
    let mut map = map();
    map.bind_global(Save, Shortcut::ctrl(KeyCode::KeyS));
    map.bind_global(Save, Shortcut::ctrl(KeyCode::KeyW));

    assert_eq!(map.resolve(Shortcut::ctrl(KeyCode::KeyS)), None);
    assert_eq!(
        map.resolve(Shortcut::ctrl(KeyCode::KeyW)),
        Some(Save.intern()),
        "an action has one shortcut per context, so rebinding moves it"
    );
}

#[test]
fn pushing_a_context_twice_still_pops_clean() {
    let mut map = map();
    map.bind(FrameSelected, Shortcut::key(KeyCode::KeyF), Viewport);

    map.push_context(Viewport);
    map.push_context(Viewport);
    map.pop_context(Viewport);

    assert_eq!(
        map.resolve(Shortcut::key(KeyCode::KeyF)),
        None,
        "a context must not linger because it was pushed twice"
    );
}

#[test]
fn modifiers_read_off_a_shortcut_builder() {
    let bare = Shortcut::key(KeyCode::KeyA);
    assert_eq!(bare.modifiers, Modifiers::NONE);
    assert!(Shortcut::ctrl(KeyCode::KeyA).modifiers.ctrl);
    assert!(Shortcut::key(KeyCode::KeyA).with_alt().modifiers.alt);
}
