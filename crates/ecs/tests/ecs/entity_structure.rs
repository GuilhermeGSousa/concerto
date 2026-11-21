use concerto_ecs::{Component, World};

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
