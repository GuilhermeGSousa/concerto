//! The editor's named actions and their default bindings.
//!
//! Panels react to actions, never to keys. Rebinding is a change to the table
//! below (or to `ActionMap` at runtime) rather than a hunt through systems.
use app::{schedule_groups::Startup, App, Plugin};
use ecs::ResMut;
use window::input::actions::{ActionMap, Shortcut};
use window::input::KeyCode;
use window::{define_action, define_context};

define_action!(
    /// Frame the current selection in the viewport.
    FrameSelected
);
define_action!(
    /// Frame everything in the world.
    FrameAll
);

define_action!(
    /// Move the tree selection down one row.
    SelectNext
);
define_action!(
    /// Move the tree selection up one row.
    SelectPrevious
);
define_action!(
    /// Jump to the first row.
    SelectFirst
);
define_action!(
    /// Jump to the last row.
    SelectLast
);
define_action!(
    /// Expand the selected row, or step into it.
    ExpandRow
);
define_action!(
    /// Collapse the selected row, or step out to its parent.
    CollapseRow
);

define_context!(
    /// Active while the pointer is over the 3D viewport.
    ViewportContext
);
define_context!(
    /// Active while keyboard focus is inside the entity tree.
    TreeContext
);

pub struct ActionsPlugin;

impl Plugin for ActionsPlugin {
    fn build(&self, app: &mut App) {
        app.add_system(Startup, install_default_bindings);
    }
}

/// The default binding table. One place to read, one place to change.
fn install_default_bindings(mut actions: ResMut<ActionMap>) {
    actions.bind(FrameSelected, Shortcut::key(KeyCode::KeyF), ViewportContext);
    actions.bind(
        FrameAll,
        Shortcut::key(KeyCode::KeyF).with_shift(),
        ViewportContext,
    );

    actions.bind(SelectNext, Shortcut::key(KeyCode::ArrowDown), TreeContext);
    actions.bind(SelectPrevious, Shortcut::key(KeyCode::ArrowUp), TreeContext);
    actions.bind(SelectFirst, Shortcut::key(KeyCode::Home), TreeContext);
    actions.bind(SelectLast, Shortcut::key(KeyCode::End), TreeContext);
    actions.bind(ExpandRow, Shortcut::key(KeyCode::ArrowRight), TreeContext);
    actions.bind(CollapseRow, Shortcut::key(KeyCode::ArrowLeft), TreeContext);
}
