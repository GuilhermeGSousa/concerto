use concerto_ecs::{Component, Resource, World};

#[derive(Resource)]
struct Gravity(f32);

#[derive(Component)]
struct Velocity(f32);

#[test]
fn a_scope_reads_the_resource_while_writing_the_world_and_puts_it_back_unchanged() {
    let mut world = World::new();
    world.insert_resource(Gravity(9.8));
    let entity = world.spawn(Velocity(0.0));
    world.tick();
    let changed = world.resource_changed_tick::<Gravity>();
    world.tick();

    world.resource_scope::<Gravity, _>(|world, gravity| {
        assert!(world.get_resource::<Gravity>().is_none());
        world
            .get_component_for_entity_mut::<Velocity>(entity)
            .unwrap()
            .0 = gravity.unwrap().0;
    });

    assert_eq!(
        world
            .get_component_for_entity::<Velocity>(entity)
            .unwrap()
            .0,
        9.8
    );
    assert_eq!(world.get_resource::<Gravity>().unwrap().0, 9.8);
    assert_eq!(world.resource_changed_tick::<Gravity>(), changed);
}

#[test]
fn a_scope_over_a_missing_resource_runs_without_it() {
    let mut world = World::new();

    let ran = world.resource_scope::<Gravity, _>(|_, gravity| gravity.is_none());

    assert!(ran);
    assert!(world.get_resource::<Gravity>().is_none());
}
