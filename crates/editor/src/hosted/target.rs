use concerto_ecs::{
    CommandQueue, Component, Entity, Query, Res, With,
    signal::{On, Signal},
};

use super::HostedApp;
use crate::asset_editor::ActiveEditor;

/// Marks the document whose `App` the editor's panels read and send signals to.
#[derive(Component)]
pub struct EditTarget;

/// Makes `document` the edit target, if it is the active tab and holds an `App`, or leaves none.
pub struct SetEditTarget(pub Option<Entity>);
impl Signal for SetEditTarget {}

pub(crate) fn move_edit_target(
    on: On<SetEditTarget>,
    targets: Query<Entity, With<EditTarget>>,
    hosted: Query<&HostedApp>,
    active: Res<ActiveEditor>,
    mut cmd: CommandQueue,
) {
    for target in targets.iter() {
        cmd.remove::<EditTarget>(target);
    }
    if let Some(document) = on.signal().0
        && active.0 == Some(document)
        && hosted.get_entity(document).is_some()
    {
        cmd.insert(EditTarget, document);
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
