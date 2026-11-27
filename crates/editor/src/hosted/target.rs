use concerto_ecs::{
    CommandQueue, Component, Entity, Query, Res, With,
    signal::{On, Signal},
};

use super::{HostRole, HostedApp};
use crate::asset_editor::ActiveEditor;

/// Marks the document whose `App` the editor's panels read and send signals to.
#[derive(Component)]
pub struct EditTarget;

/// Marks the active document whose hosted `App`, of any role, the viewport shows, takes input and is stepped.
#[derive(Component)]
pub struct ViewportTarget;

/// Moves the targets to `document` if it holds an `App`, or clears them.
///
/// A `document` that is not the active tab changes nothing.
pub struct SetEditTarget(pub Option<Entity>);
impl Signal for SetEditTarget {}

pub(crate) fn move_edit_target(
    on: On<SetEditTarget>,
    edit_targets: Query<Entity, With<EditTarget>>,
    viewport_targets: Query<Entity, With<ViewportTarget>>,
    hosted: Query<&HostedApp>,
    active: Res<ActiveEditor>,
    mut cmd: CommandQueue,
) {
    let wanted = on.signal().0;
    if wanted.is_some_and(|document| active.0 != Some(document)) {
        return;
    }
    for target in edit_targets.iter() {
        cmd.remove::<EditTarget>(target);
    }
    for target in viewport_targets.iter() {
        cmd.remove::<ViewportTarget>(target);
    }
    if let Some(document) = wanted
        && let Some(app) = hosted.get_entity(document)
    {
        cmd.insert(ViewportTarget, document);
        if app.role() != HostRole::Preview {
            cmd.insert(EditTarget, document);
        }
    }
}
