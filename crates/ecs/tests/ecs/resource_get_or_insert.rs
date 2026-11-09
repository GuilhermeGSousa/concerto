use concerto_ecs::{Resource, World};

#[derive(Resource)]
struct Gravity(f32);

#[test]
fn a_missing_resource_is_built_and_a_present_one_is_returned_and_marked_changed() {
    let mut world = World::new();

    world.get_resource_or_insert_with(|| Gravity(9.8)).0 += 0.2;
    assert_eq!(world.get_resource::<Gravity>().unwrap().0, 10.0);

    world.tick();
    let inserted = world.resource_changed_tick::<Gravity>();
    let mut built = false;
    let gravity = world.get_resource_or_insert_with(|| {
        built = true;
        Gravity(0.0)
    });

    assert_eq!(gravity.0, 10.0);
    assert!(!built);
    assert_ne!(world.resource_changed_tick::<Gravity>(), inserted);
}
