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

/// Moves the targets to `document` if it holds an `App`, or clears them; a `document` that is not the active tab changes nothing.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        asset_editor::ActiveEditor,
        hosted::{HostedApp, testing::*},
    };
    use concerto_ecs::{
        ResMut, Resource, World,
        signal::lifecycle::{Add, Remove},
    };

    fn world_with_two_documents() -> (World, Entity, Entity) {
        let mut world = World::new();
        world.insert_resource(ActiveEditor::default());
        world.add_listener(move_edit_target);
        let first = world.spawn(hosted_now(plain_recipe, |_| {}));
        let second = world.spawn(hosted_now(plain_recipe, |_| {}));
        (world, first, second)
    }

    fn activate(world: &mut World, document: Option<Entity>) {
        world.get_resource_mut::<ActiveEditor>().unwrap().0 = document;
        world.trigger(SetEditTarget(document));
    }

    fn targets(world: &mut World) -> Vec<Entity> {
        world
            .query::<(Entity, &EditTarget), ()>()
            .iter(world)
            .map(|(entity, _)| entity)
            .collect()
    }

    #[test]
    fn the_target_moves_to_the_document_it_names() {
        let (mut world, first, second) = world_with_two_documents();

        activate(&mut world, Some(first));
        assert_eq!(targets(&mut world), [first]);

        activate(&mut world, Some(second));
        assert_eq!(targets(&mut world), [second]);
    }

    #[test]
    fn none_leaves_no_target() {
        let (mut world, first, _) = world_with_two_documents();
        activate(&mut world, Some(first));

        world.trigger(SetEditTarget(None));

        assert!(targets(&mut world).is_empty());
    }

    #[test]
    fn a_document_without_an_app_or_not_in_front_is_not_marked() {
        let (mut world, first, second) = world_with_two_documents();
        let loading = world.spawn(());

        activate(&mut world, Some(loading));
        assert!(targets(&mut world).is_empty());

        world.get_resource_mut::<ActiveEditor>().unwrap().0 = Some(first);
        world.trigger(SetEditTarget(Some(second)));
        assert!(targets(&mut world).is_empty());
    }

    fn viewport_targets(world: &mut World) -> Vec<Entity> {
        world
            .query::<(Entity, &ViewportTarget), ()>()
            .iter(world)
            .map(|(entity, _)| entity)
            .collect()
    }

    #[test]
    fn a_bare_hosted_app_gets_both_markers() {
        let (mut world, first, _) = world_with_two_documents();

        activate(&mut world, Some(first));

        assert_eq!(targets(&mut world), [first]);
        assert_eq!(viewport_targets(&mut world), [first]);
    }

    #[test]
    fn a_preview_app_gets_only_the_viewport_target() {
        let (mut world, first, _) = world_with_two_documents();
        activate(&mut world, Some(first));
        let preview = world.spawn(preview_now(plain_recipe));

        activate(&mut world, Some(preview));

        assert!(targets(&mut world).is_empty());
        assert_eq!(viewport_targets(&mut world), [preview]);
    }

    #[test]
    fn a_target_for_a_document_not_in_front_leaves_the_markers_alone() {
        let (mut world, first, second) = world_with_two_documents();
        activate(&mut world, Some(first));

        world.trigger(SetEditTarget(Some(second)));

        assert_eq!(targets(&mut world), [first]);
        assert_eq!(viewport_targets(&mut world), [first]);
    }

    #[derive(Resource, Default)]
    struct Moves(Vec<&'static str>);

    fn note_add(_: On<Add<EditTarget>>, mut moves: ResMut<Moves>) {
        moves.0.push("add");
    }

    fn note_remove(_: On<Remove<EditTarget>>, mut moves: ResMut<Moves>) {
        moves.0.push("remove");
    }

    #[test]
    fn retargeting_the_same_document_removes_the_marker_then_adds_it() {
        let (mut world, first, _) = world_with_two_documents();
        world.insert_resource(Moves::default());
        world.add_listener(note_add);
        world.add_listener(note_remove);
        activate(&mut world, Some(first));

        world.trigger(SetEditTarget(Some(first)));

        assert_eq!(
            world.get_resource::<Moves>().unwrap().0,
            ["add", "remove", "add"]
        );
        assert_eq!(targets(&mut world), [first]);
        let _: &HostedApp = world.get_component_for_entity(first).unwrap();
    }
}
