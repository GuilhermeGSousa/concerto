use concerto_ecs::{CommandQueue, Component, IntoSystem, System, World};

#[derive(Component)]
struct A(u32);
#[derive(Component)]
struct B;

#[test]
fn structural_versions_track_membership_not_values_or_frames() {
    let mut world = World::default();
    let entity = world.spawn(A(1));
    let initial = world.structural_version(entity).unwrap();
    world.get_component_for_entity_mut::<A>(entity).unwrap().0 = 2;
    world.insert(A(3), entity);
    world.insert((), entity);
    world.remove_component::<B>(entity);
    world.tick();
    assert_eq!(world.structural_version(entity), Some(initial));
    world.insert(B, entity);
    let added = world.structural_version(entity).unwrap();
    assert_ne!(added, initial);
    world.remove_component::<B>(entity);
    assert_ne!(world.structural_version(entity), Some(added));
    assert_ne!(world.structural_version(entity), Some(initial));
    for _ in 0..3 {
        world.tick();
    }
    assert_eq!(world.get_component_for_entity::<A>(entity).unwrap().0, 3);
}

#[test]
fn row_swaps_and_recycled_handles_do_not_report_false_changes() {
    let mut world = World::default();
    let first = world.spawn(A(1));
    let second = world.spawn(A(2));
    let version = world.structural_version(second);
    world.insert(B, first);
    assert_eq!(world.structural_version(second), version);
    world.remove_component::<B>(first);
    world.despawn(first);
    assert_eq!(world.structural_version(second), version);
    assert_eq!(world.structural_version(first), None);
    let replacement = world.spawn(());
    assert_eq!(replacement.index(), first.index());
    assert!(world.structural_version(replacement).is_some());
    assert_eq!(world.structural_version(first), None);
}

#[test]
fn deferred_changes_are_visible_only_after_apply() {
    let mut world = World::default();
    let target = world.spawn(());
    let version = world.structural_version(target);
    let mut add = (move |mut cmd: CommandQueue| {
        cmd.insert(B, target);
    })
    .into_system();
    add.initialize(&mut world);
    add.run((), &mut world);
    assert_eq!(world.structural_version(target), version);
    add.apply(&mut world);
    let added = world.structural_version(target);
    assert_ne!(added, version);
    let mut remove = (move |mut cmd: CommandQueue| {
        cmd.remove::<B>(target);
    })
    .into_system();
    remove.initialize(&mut world);
    remove.run_and_apply((), &mut world);
    assert_ne!(world.structural_version(target), added);
}
