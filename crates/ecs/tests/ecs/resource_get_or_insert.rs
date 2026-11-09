use concerto_ecs::{Resource, World};

#[derive(Resource)]
struct Gravity(f32);

#[test]
fn a_missing_resource_is_inserted_and_returned() {
    let mut world = World::new();

    world.get_resource_or_insert_with(|| Gravity(9.8)).0 += 0.2;

    assert_eq!(world.get_resource::<Gravity>().unwrap().0, 10.0);
}

#[test]
fn a_present_resource_is_returned_without_building_another() {
    let mut world = World::new();
    world.insert_resource(Gravity(9.8));
    let mut built = false;

    let gravity = world.get_resource_or_insert_with(|| {
        built = true;
        Gravity(0.0)
    });

    assert_eq!(gravity.0, 9.8);
    assert!(!built);
}

#[test]
fn a_present_resource_is_marked_changed() {
    let mut world = World::new();
    world.insert_resource(Gravity(9.8));
    world.tick();
    let inserted = world.resource_changed_tick::<Gravity>();

    world.get_resource_or_insert_with(|| Gravity(0.0));

    assert_ne!(world.resource_changed_tick::<Gravity>(), inserted);
}
