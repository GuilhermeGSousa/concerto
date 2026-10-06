//! The editor's named actions and their default bindings.
use concerto_app::{App, Plugin, schedule_groups::Startup};
use concerto_ecs::ResMut;
use concerto_window::input::KeyCode;
use concerto_window::input::actions::{ActionMap, Shortcut};
use concerto_window::{define_action, define_context};

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

define_action!(
    /// Write the active document to disk.
    Save
);

define_action!(
    /// Delete the selected entity and its descendants.
    DeleteEntity
);
define_action!(
    /// Copy the selected entity and its descendants beside it.
    DuplicateEntity
);
define_action!(
    /// Edit the selected entity's name.
    RenameEntity
);
define_action!(
    /// Show the handles that move the selected entity.
    GizmoTranslate
);
define_action!(
    /// Show the handles that rotate the selected entity.
    GizmoRotate
);
define_action!(
    /// Close the unsaved-changes prompt without acting.
    DismissPrompt
);

define_context!(
    /// Active while the unsaved-changes prompt is open.
    PromptContext
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

fn install_default_bindings(mut actions: ResMut<ActionMap>) {
    actions.bind_global(Save, Shortcut::ctrl(KeyCode::KeyS));
    actions.bind(DismissPrompt, Shortcut::key(KeyCode::Escape), PromptContext);
    actions.bind(FrameSelected, Shortcut::key(KeyCode::KeyF), ViewportContext);
    actions.bind(
        FrameAll,
        Shortcut::key(KeyCode::KeyF).with_shift(),
        ViewportContext,
    );

    actions.bind(DeleteEntity, Shortcut::key(KeyCode::Delete), TreeContext);
    actions.bind(
        DeleteEntity,
        Shortcut::key(KeyCode::Delete),
        ViewportContext,
    );
    actions.bind(DuplicateEntity, Shortcut::ctrl(KeyCode::KeyD), TreeContext);
    actions.bind(
        DuplicateEntity,
        Shortcut::ctrl(KeyCode::KeyD),
        ViewportContext,
    );
    actions.bind(RenameEntity, Shortcut::key(KeyCode::F2), TreeContext);
    actions.bind(
        GizmoTranslate,
        Shortcut::key(KeyCode::KeyW),
        ViewportContext,
    );
    actions.bind(GizmoRotate, Shortcut::key(KeyCode::KeyE), ViewportContext);

    actions.bind(SelectNext, Shortcut::key(KeyCode::ArrowDown), TreeContext);
    actions.bind(SelectPrevious, Shortcut::key(KeyCode::ArrowUp), TreeContext);
    actions.bind(SelectFirst, Shortcut::key(KeyCode::Home), TreeContext);
    actions.bind(SelectLast, Shortcut::key(KeyCode::End), TreeContext);
    actions.bind(ExpandRow, Shortcut::key(KeyCode::ArrowRight), TreeContext);
    actions.bind(CollapseRow, Shortcut::key(KeyCode::ArrowLeft), TreeContext);
}
