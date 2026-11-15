use concerto_ecs::{
    Component, Entity, World, component::ComponentLifecycleCallback, resource::Resource,
};

#[derive(Component)]
struct Marker;

struct First;
struct Second;

impl Component for First {
    fn on_add() -> Option<ComponentLifecycleCallback> {
        Some(|mut world, context| {
            world.remove_component::<Second>(context.entity);
            world.insert(Marker, context.entity);
            assert!(
                world
                    .get_component_for_entity::<Second>(context.entity)
                    .is_some()
            );
            assert!(
                world
                    .get_component_for_entity::<Marker>(context.entity)
                    .is_none()
            );
        })
    }
}

impl Component for Second {
    fn on_add() -> Option<ComponentLifecycleCallback> {
        Some(|world, context| {
            assert!(
                world
                    .get_component_for_entity::<First>(context.entity)
                    .is_some()
            );
            assert!(
                world
                    .get_component_for_entity::<Self>(context.entity)
                    .is_some()
            );
            assert!(
                world
                    .get_component_for_entity::<Marker>(context.entity)
                    .is_none()
            );
        })
    }
}

#[test]
fn entire_bundle_hook_pass_precedes_structural_commands() {
    let mut world = World::new();
    world.register_component::<First>();
    world.register_component::<Second>();
    let entity = world.spawn((First, Second));
    assert!(world.get_component_for_entity::<Second>(entity).is_none());
    assert!(world.get_component_for_entity::<Marker>(entity).is_some());
}

#[derive(Resource, Default)]
struct Spawned(Option<Entity>);

struct Spawner;
impl Component for Spawner {
    fn on_add() -> Option<ComponentLifecycleCallback> {
        Some(|mut world, _| {
            let entity = world.spawn(());
            world.insert(Marker, entity);
            assert!(world.get_component_for_entity::<Marker>(entity).is_none());
            world.get_resource_mut::<Spawned>().unwrap().0 = Some(entity);
        })
    }
}

#[test]
fn callback_spawn_reserves_an_entity_for_followup_commands() {
    let mut world = World::new();
    world.register_component::<Spawner>();
    world.insert_resource(Spawned::default());
    world.spawn(Spawner);
    let entity = world.get_resource::<Spawned>().unwrap().0.unwrap();
    assert!(world.entity_is_valid(entity));
    assert!(world.get_component_for_entity::<Marker>(entity).is_some());
}

struct Cleanup;
impl Component for Cleanup {
    fn on_remove() -> Option<ComponentLifecycleCallback> {
        Some(|mut world, context| {
            world.remove_component::<Self>(context.entity);
            assert!(
                world
                    .get_component_for_entity::<Self>(context.entity)
                    .is_some()
            );
        })
    }

    fn on_despawn() -> Option<ComponentLifecycleCallback> {
        Some(|mut world, context| {
            world.despawn(context.entity);
            world.insert(Marker, context.entity);
            world.remove_component::<Self>(context.entity);
            assert!(
                world
                    .get_component_for_entity::<Self>(context.entity)
                    .is_some()
            );
        })
    }
}

#[test]
fn deferred_self_cleanup_does_not_reenter_or_touch_dead_entities() {
    let mut world = World::new();
    world.register_component::<Cleanup>();
    let removed = world.spawn(Cleanup);
    world.remove_component::<Cleanup>(removed);
    assert!(world.entity_is_valid(removed));
    assert!(world.get_component_for_entity::<Cleanup>(removed).is_none());
    let despawned = world.spawn(Cleanup);
    world.despawn(despawned);
    assert!(!world.entity_is_valid(despawned));
    let reused = world.spawn(());
    assert!(world.get_component_for_entity::<Marker>(reused).is_none());
}
