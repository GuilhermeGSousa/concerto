use concerto_ecs::{
    World,
    component::{Component, ComponentLifecycleCallback},
    entity::hierarchy::{ChildOf, Children},
    resource::Resource,
};

#[derive(Resource, Default)]
struct Events(Vec<&'static str>);

struct Tracked;

impl Component for Tracked {
    fn on_despawn() -> Option<ComponentLifecycleCallback> {
        Some(|mut world, context| {
            assert!(
                world
                    .get_component_for_entity::<Tracked>(context.entity)
                    .is_some()
            );
            world
                .get_resource_mut::<Events>()
                .unwrap()
                .0
                .push("despawn");
        })
    }

    fn on_remove() -> Option<ComponentLifecycleCallback> {
        Some(|mut world, _| {
            world.get_resource_mut::<Events>().unwrap().0.push("remove");
        })
    }
}

#[test]
fn despawn_runs_removal_cleanup_before_its_own_hook() {
    let mut world = World::new();
    world.register_component::<Tracked>();
    world.insert_resource(Events::default());
    let entity = world.spawn(Tracked);
    world.despawn(entity);
    assert_eq!(
        world.get_resource::<Events>().unwrap().0,
        ["remove", "despawn"],
        "a despawn removes the component too, so both hooks run"
    );

    world.get_resource_mut::<Events>().unwrap().0.clear();
    let entity = world.spawn(Tracked);
    world.remove_component::<Tracked>(entity);
    assert_eq!(
        world.get_resource::<Events>().unwrap().0,
        ["remove"],
        "removing a component must not run despawn-only teardown"
    );
    assert!(world.entity_is_valid(entity));
}

#[test]
fn despawning_a_leaf_detaches_it_and_preserves_siblings() {
    let mut world = hierarchy_world();
    let parent = world.spawn(());
    let leaf = world.spawn(());
    let sibling = world.spawn(());
    world.entity_mut(parent).add_child(leaf);
    world.entity_mut(parent).add_child(sibling);

    world.despawn(leaf);
    let children = world.get_component_for_entity::<Children>(parent).unwrap();
    assert_eq!(children.iter().copied().collect::<Vec<_>>(), [sibling]);
    assert!(world.entity_is_valid(sibling));

    world.despawn(parent);
    assert!(!world.entity_is_valid(parent));
    assert!(!world.entity_is_valid(sibling));
}

#[test]
fn despawning_the_last_leaf_removes_the_children_component() {
    let mut world = hierarchy_world();
    let parent = world.spawn(());
    let leaf = world.spawn(());
    world.entity_mut(parent).add_child(leaf);

    world.despawn(leaf);
    assert!(world.entity_is_valid(parent));
    assert!(world.get_component_for_entity::<Children>(parent).is_none());
    world.despawn(parent);
}

#[test]
fn despawning_a_subtree_detaches_it_from_its_surviving_parent() {
    let mut world = hierarchy_world();
    let grandparent = world.spawn(());
    let parent = world.spawn(());
    let child = world.spawn(());
    world.entity_mut(grandparent).add_child(parent);
    world.entity_mut(parent).add_child(child);

    world.despawn(parent);
    assert!(!world.entity_is_valid(parent));
    assert!(!world.entity_is_valid(child));
    assert!(world.entity_is_valid(grandparent));
    assert!(
        world
            .get_component_for_entity::<Children>(grandparent)
            .is_none()
    );
}

fn hierarchy_world() -> World {
    let mut world = World::new();
    world.register_component::<Children>();
    world.register_component::<ChildOf>();
    world
}
