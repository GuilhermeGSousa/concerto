use concerto_ecs::{
    World,
    component::{Component, ComponentLifecycleCallback},
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
