use concerto_ecs::{
    CommandQueue, Entity, Query, ResMut, Resource, World,
    component::{Component, ComponentLifecycleCallback},
    signal::{
        On,
        lifecycle::{Add, Despawn, Remove, Replace},
    },
};

#[derive(Resource, Default)]
struct Log(Vec<String>);

fn log(world: &mut concerto_ecs::world::RestrictedWorld, line: String) {
    if let Some(log) = world.get_resource_mut::<Log>() {
        log.0.push(line);
    }
}

struct Tracked(u32);

impl Component for Tracked {
    fn on_add() -> Option<ComponentLifecycleCallback> {
        Some(|mut world, context| {
            let value = world
                .get_component_for_entity::<Tracked>(context.entity)
                .map(|t| t.0);
            log(&mut world, format!("add hook {value:?}"));
        })
    }

    fn on_replace() -> Option<ComponentLifecycleCallback> {
        Some(|mut world, context| {
            let value = world
                .get_component_for_entity::<Tracked>(context.entity)
                .map(|t| t.0);
            log(&mut world, format!("replace hook {value:?}"));
        })
    }

    fn on_remove() -> Option<ComponentLifecycleCallback> {
        Some(|mut world, context| {
            let value = world
                .get_component_for_entity::<Tracked>(context.entity)
                .map(|t| t.0);
            log(&mut world, format!("remove hook {value:?}"));
        })
    }

    fn on_despawn() -> Option<ComponentLifecycleCallback> {
        Some(|mut world, context| {
            let value = world
                .get_component_for_entity::<Tracked>(context.entity)
                .map(|t| t.0);
            log(&mut world, format!("despawn hook {value:?}"));
        })
    }
}

#[derive(concerto_ecs::Component)]
struct Unqueried;

fn value(tracked: &Query<&Tracked>, entity: Entity) -> Option<u32> {
    tracked.get_entity(entity).map(|t| t.0)
}

fn on_add(on: On<Add<Tracked>>, tracked: Query<&Tracked>, mut log: ResMut<Log>) {
    let entity = on.signal().entity;
    log.0
        .push(format!("add signal {:?}", value(&tracked, entity)));
}

fn on_replace(on: On<Replace<Tracked>>, tracked: Query<&Tracked>, mut log: ResMut<Log>) {
    let entity = on.signal().entity;
    log.0
        .push(format!("replace signal {:?}", value(&tracked, entity)));
}

fn on_remove(on: On<Remove<Tracked>>, tracked: Query<&Tracked>, mut log: ResMut<Log>) {
    let entity = on.signal().entity;
    log.0
        .push(format!("remove signal {:?}", value(&tracked, entity)));
}

fn on_despawn(on: On<Despawn<Tracked>>, tracked: Query<&Tracked>, mut log: ResMut<Log>) {
    let entity = on.signal().entity;
    log.0
        .push(format!("despawn signal {:?}", value(&tracked, entity)));
}

fn world() -> World {
    let mut world = World::new();
    world.insert_resource(Log::default());
    world.add_listener(on_add);
    world.add_listener(on_replace);
    world.add_listener(on_remove);
    world.add_listener(on_despawn);
    world
}

fn take(world: &mut World) -> Vec<String> {
    std::mem::take(&mut world.get_resource_mut::<Log>().unwrap().0)
}

fn spawn_through_commands(mut cmd: CommandQueue) {
    cmd.spawn(Tracked(7));
}

struct SignalCase {
    name: &'static str,
    setup: fn(&mut World) -> Entity,
    act: fn(&mut World, Entity),
    log: &'static [&'static str],
}

#[test]
fn each_operation_runs_its_hook_then_its_signal_while_the_value_is_readable() {
    let cases = [
        SignalCase {
            name: "an overwrite fires replace with the old value then add with the new one",
            setup: |world| world.spawn(Tracked(1)),
            act: |world, entity| world.insert(Tracked(2), entity),
            log: &[
                "replace hook Some(1)",
                "replace signal Some(1)",
                "add hook Some(2)",
                "add signal Some(2)",
            ],
        },
        SignalCase {
            name: "a despawn fires remove then despawn",
            setup: |world| world.spawn(Tracked(5)),
            act: |world, entity| world.despawn(entity),
            log: &[
                "remove hook Some(5)",
                "remove signal Some(5)",
                "despawn hook Some(5)",
                "despawn signal Some(5)",
            ],
        },
        SignalCase {
            name: "an operation applied from commands fires too",
            setup: |world| world.spawn(()),
            act: |world, _| {
                let mut system = concerto_ecs::IntoSystem::into_system(spawn_through_commands);
                concerto_ecs::System::initialize(&mut system, world);
                concerto_ecs::System::run_and_apply(&mut system, (), world);
            },
            log: &["add hook Some(7)", "add signal Some(7)"],
        },
    ];
    for case in cases {
        let mut world = world();
        let entity = (case.setup)(&mut world);
        take(&mut world);

        (case.act)(&mut world, entity);

        assert_eq!(take(&mut world), case.log, "{}", case.name);
    }
}

#[derive(concerto_ecs::Component)]
struct Companion;

fn spawn_a_companion(_: On<Add<Unqueried>>, mut cmd: CommandQueue) {
    cmd.spawn(Companion);
}

#[test]
fn a_lifecycle_listeners_commands_are_applied() {
    let mut world = World::new();
    world.add_listener(spawn_a_companion);

    world.spawn(Unqueried);

    assert_eq!(world.query::<&Companion, ()>().iter(&mut world).count(), 1);
}

struct Spawner;

impl Component for Spawner {
    fn on_add() -> Option<ComponentLifecycleCallback> {
        Some(|mut world, context| {
            world.insert(Unqueried, context.entity);
        })
    }
}

#[derive(Resource, Default)]
struct Added(Vec<Entity>);

fn note_added(on: On<Add<Unqueried>>, mut added: ResMut<Added>) {
    added.0.push(on.signal().entity);
}

#[test]
fn signals_fire_for_changes_a_hook_makes_through_its_restricted_world() {
    let mut world = World::new();
    world.insert_resource(Added::default());
    world.register_component::<Spawner>();
    world.add_listener(note_added);

    let entity = world.spawn(Spawner);

    assert_eq!(world.get_resource::<Added>().unwrap().0, [entity]);
}
