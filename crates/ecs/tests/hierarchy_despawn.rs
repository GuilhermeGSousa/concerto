use ecs::{
    World,
    entity::hierarchy::{ChildOf, Children},
};

/// Relationship upkeep lives in these components' lifecycle callbacks, which an
/// app installs by registering them.
fn world() -> World {
    let mut world = World::new();
    world.register_component::<ChildOf>();
    world.register_component::<Children>();
    world
}

#[test]
fn despawn_takes_descendants_and_detaches_the_surviving_parent() {
    let mut world = world();
    let grandparent = world.spawn(());
    let parent = world.spawn(());
    let child = world.spawn(());
    world.add_child(grandparent, parent);
    world.add_child(parent, child);

    world.despawn(parent);

    assert!(world.entity_is_valid(grandparent));
    assert!(
        !world.entity_is_valid(child),
        "despawn must cascade to descendants"
    );
    assert!(
        world
            .get_component_for_entity::<Children>(grandparent)
            .is_none(),
        "removing the last child must remove the empty relationship component"
    );
}

#[test]
fn reparenting_repairs_the_previous_parents_children() {
    let mut world = world();
    let first_parent = world.spawn(());
    let second_parent = world.spawn(());
    let child = world.spawn(());
    world.add_child(first_parent, child);
    world.add_child(second_parent, child);

    assert_eq!(
        world
            .get_component_for_entity::<ChildOf>(child)
            .expect("child must have its new parent")
            .parent(),
        second_parent
    );
    assert!(
        world
            .get_component_for_entity::<Children>(first_parent)
            .is_none(),
        "reparenting the only child must remove the old empty relationship component"
    );
}

#[test]
fn recursive_despawn_handles_deep_hierarchies_iteratively() {
    let mut world = world();
    let root = world.spawn(());
    let mut entities = vec![root];
    let mut parent = root;
    for _ in 0..2_048 {
        let child = world.spawn(());
        world.add_child(parent, child);
        entities.push(child);
        parent = child;
    }

    world.despawn(root);

    assert!(
        entities
            .into_iter()
            .all(|entity| !world.entity_is_valid(entity))
    );
}
