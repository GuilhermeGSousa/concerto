//! The editor's named actions and their default bindings.
use concerto_app::{App, Plugin, schedule_groups::Startup};
use concerto_ecs::{ResMut, SystemSet};
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
    /// Write the active document to disk.
    Save
);

define_action!(
    /// Start the game in the Scene tab, or stop it.
    TogglePlay
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

define_action!(
    /// Queue the staged imports and close the import dialog.
    ConfirmImport
);
define_action!(
    /// Close the import dialog without importing anything.
    CancelImport
);

define_context!(
    /// Active while the unsaved-changes prompt is open.
    PromptContext
);
define_context!(
    /// Active while the import dialog is open.
    ImportDialogContext
);
define_context!(
    /// Active while the pointer is over the 3D viewport.
    ViewportContext
);
define_context!(
    /// Active while keyboard focus is inside the entity tree.
    TreeContext
);

/// The `LateUpdate` systems that push and pop the viewport's and the tree's contexts.
///
/// A context pushed after this set wins their keys.
#[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
pub struct EditorContextSet;

pub struct ActionsPlugin;

impl Plugin for ActionsPlugin {
    fn build(&self, app: &mut App) {
        app.add_system(Startup, install_default_bindings);
    }
}

fn install_default_bindings(mut actions: ResMut<ActionMap>) {
    actions.bind_global(Save, Shortcut::ctrl(KeyCode::KeyS));
    actions.bind_global(TogglePlay, Shortcut::ctrl(KeyCode::KeyP));
    actions.bind(DismissPrompt, Shortcut::key(KeyCode::Escape), PromptContext);
    actions.bind(
        ConfirmImport,
        Shortcut::key(KeyCode::Enter),
        ImportDialogContext,
    );
    actions.bind(
        CancelImport,
        Shortcut::key(KeyCode::Escape),
        ImportDialogContext,
    );
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use concerto_ecs::{IntoSystem, System, World};
    use concerto_window::input::actions::ActionLabel;

    fn bound() -> World {
        let mut world = World::new();
        world.insert_resource(ActionMap::default());
        let mut system = install_default_bindings.into_system();
        system.initialize(&mut world);
        system.run_and_apply((), &mut world);
        world
    }

    #[test]
    fn the_default_bindings_resolve_in_the_context_that_holds_them() {
        let mut world = bound();
        let actions = world.get_resource_mut::<ActionMap>().unwrap();
        actions.push_context(TreeContext);
        assert_eq!(
            actions.resolve(Shortcut::ctrl(KeyCode::KeyP)),
            Some(TogglePlay.intern())
        );
        assert_eq!(
            actions.resolve(Shortcut::key(KeyCode::Delete)),
            Some(DeleteEntity.intern())
        );
        assert_eq!(actions.resolve(Shortcut::key(KeyCode::ArrowDown)), None);

        actions.push_context(ImportDialogContext);
        actions.set_capturing_text(true);
        assert_eq!(
            actions.resolve(Shortcut::key(KeyCode::Enter)),
            Some(ConfirmImport.intern())
        );
        assert_eq!(
            actions.resolve(Shortcut::key(KeyCode::Escape)),
            Some(CancelImport.intern())
        );

        actions.pop_context(ImportDialogContext);
        assert_eq!(actions.resolve(Shortcut::key(KeyCode::Enter)), None);
    }
}
