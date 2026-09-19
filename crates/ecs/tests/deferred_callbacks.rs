use ecs::entity::hierarchy::{ChildOf, Children};
use ecs::{Component, Entity, World, component::ComponentLifecycleCallback, resource::Resource};

#[derive(Component)]
struct Marker;

struct First;
struct Second;

impl Component for First {
    fn on_add() -> Option<ComponentLifecycleCallback> {
        Some(|mut world, context| {
            world.remove_component::<Second>(context.entity, true);
            world.insert(Marker, context.entity, true);
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
            world.insert(Marker, entity, true);
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

#[derive(Resource, Default)]
struct Order(Vec<u8>);
struct Chain(u8);
impl Component for Chain {
    fn on_add() -> Option<ComponentLifecycleCallback> {
        Some(|mut world, context| {
            let step = world
                .get_component_for_entity::<Self>(context.entity)
                .unwrap()
                .0;
            world.get_resource_mut::<Order>().unwrap().0.push(step);
            if step == 0 {
                world.insert(Self(1), context.entity, true);
                world.insert(Self(3), context.entity, true);
            } else if step == 1 {
                world.insert(Self(2), context.entity, true);
            }
        })
    }
}

#[test]
fn nested_callback_commands_finish_before_the_next_sibling() {
    let mut world = World::new();
    world.register_component::<Chain>();
    world.insert_resource(Order::default());
    world.spawn(Chain(0));
    assert_eq!(world.get_resource::<Order>().unwrap().0, [0, 1, 2, 3]);
}

struct Cleanup;
impl Component for Cleanup {
    fn on_remove() -> Option<ComponentLifecycleCallback> {
        Some(|mut world, context| {
            world.remove_component::<Self>(context.entity, true);
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
            world.insert(Marker, context.entity, true);
            world.remove_component::<Self>(context.entity, true);
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

#[test]
fn callback_despawns_handle_deep_hierarchies_without_recursive_flushing() {
    let mut world = World::new();
    world.register_component::<Children>();
    world.register_component::<ChildOf>();
    let root = world.spawn(());
    let mut entities = vec![root];
    for _ in 0..2_048 {
        let child = world.spawn(());
        world.add_child(*entities.last().unwrap(), child);
        entities.push(child);
    }
    world.despawn(root);
    assert!(
        entities
            .into_iter()
            .all(|entity| !world.entity_is_valid(entity))
    );
}
