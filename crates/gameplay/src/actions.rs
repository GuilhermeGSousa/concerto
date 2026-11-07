use concerto_window::define_action;
use concerto_window::input::GamepadStick;
use concerto_window::input::actions::{ActionMap, Axis2dBinding};

define_action!(
    /// Moves the player: WASD, the arrow keys or the left stick.
    Move
);
define_action!(
    /// Turns the camera: the right stick.
    Look
);

/// Binds `Move` and `Look` globally.
pub fn bind_gameplay_actions(actions: &mut ActionMap) {
    actions.bind_axis2d_global(Move, Axis2dBinding::wasd());
    actions.bind_axis2d_global(Move, Axis2dBinding::arrows());
    actions.bind_axis2d_global(Move, Axis2dBinding::Stick(GamepadStick::Left));
    actions.bind_axis2d_global(Look, Axis2dBinding::Stick(GamepadStick::Right));
}
