use concerto::window::{
    define_action,
    input::{
        GamepadButton, KeyCode,
        actions::{ActionMap, Shortcut},
    },
};

define_action!(
    /// Uses what the player looks at: West, or E.
    Interact
);
define_action!(
    /// Leaves a screen or puts down what is carried: East, or Q.
    Back
);
define_action!(
    /// Opens and closes the casebook: North, or Tab.
    OpenBook
);
define_action!(
    /// Picks the highlighted row: South, or Enter.
    Confirm
);
define_action!(
    /// Moves up a screen: d-pad up, or the up arrow.
    MenuUp
);
define_action!(
    /// Moves down a screen: d-pad down, or the down arrow.
    MenuDown
);
define_action!(
    /// Steps left on a screen: d-pad left, or the left arrow.
    MenuLeft
);
define_action!(
    /// Steps right on a screen: d-pad right, or the right arrow.
    MenuRight
);
define_action!(
    /// Shows the pocket watch while held: the left bumper, or C.
    Watch
);
define_action!(
    /// Listens at a door while held: the right bumper, or R.
    Listen
);

/// Binds the game's actions for pads and for the keyboard that stands in for one player.
pub fn bind_controls(actions: &mut ActionMap) {
    let key = |key: KeyCode| Shortcut::key(key);
    actions.bind_global(Interact, GamepadButton::West);
    actions.bind_global(Interact, key(KeyCode::KeyE));
    actions.bind_global(Back, GamepadButton::East);
    actions.bind_global(Back, key(KeyCode::KeyQ));
    actions.bind_global(OpenBook, GamepadButton::North);
    actions.bind_global(OpenBook, key(KeyCode::Tab));
    actions.bind_global(Confirm, GamepadButton::South);
    actions.bind_global(Confirm, key(KeyCode::Enter));
    actions.bind_global(MenuUp, GamepadButton::DPadUp);
    actions.bind_global(MenuUp, key(KeyCode::ArrowUp));
    actions.bind_global(MenuDown, GamepadButton::DPadDown);
    actions.bind_global(MenuDown, key(KeyCode::ArrowDown));
    actions.bind_global(MenuLeft, GamepadButton::DPadLeft);
    actions.bind_global(MenuLeft, key(KeyCode::ArrowLeft));
    actions.bind_global(MenuRight, GamepadButton::DPadRight);
    actions.bind_global(MenuRight, key(KeyCode::ArrowRight));
    actions.bind_global(Watch, GamepadButton::LeftBumper);
    actions.bind_global(Watch, key(KeyCode::KeyC));
    actions.bind_global(Listen, GamepadButton::RightBumper);
    actions.bind_global(Listen, key(KeyCode::KeyR));
}
