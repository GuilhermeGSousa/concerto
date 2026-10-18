use concerto_ecs::{
    CommandQueue, Entity, Query, Res, ResMut, Resource, World,
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

#[derive(Resource, Default)]
struct Seen(Vec<Entity>);

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

#[test]
fn spawning_runs_the_add_hook_then_the_add_signal_with_the_new_value() {
    let mut world = world();

    world.spawn(Tracked(1));

    assert_eq!(take(&mut world), ["add hook Some(1)", "add signal Some(1)"]);
}

#[test]
fn inserting_a_new_type_fires_add() {
    let mut world = world();
    let entity = world.spawn(());

    world.insert(Tracked(4), entity);

    assert_eq!(take(&mut world), ["add hook Some(4)", "add signal Some(4)"]);
}

#[test]
fn an_overwrite_fires_replace_with_the_old_value_then_add_with_the_new_one() {
    let mut world = world();
    let entity = world.spawn(Tracked(1));
    take(&mut world);

    world.insert(Tracked(2), entity);

    assert_eq!(
        take(&mut world),
        [
            "replace hook Some(1)",
            "replace signal Some(1)",
            "add hook Some(2)",
            "add signal Some(2)"
        ]
    );
}

#[test]
fn a_removal_fires_remove_while_the_value_is_still_readable() {
    let mut world = world();
    let entity = world.spawn(Tracked(3));
    take(&mut world);

    world.remove_component::<Tracked>(entity);

    assert_eq!(
        take(&mut world),
        ["remove hook Some(3)", "remove signal Some(3)"]
    );
}

#[test]
fn a_despawn_fires_remove_then_despawn_while_the_value_is_still_readable() {
    let mut world = world();
    let entity = world.spawn(Tracked(5));
    take(&mut world);

    world.despawn(entity);

    assert_eq!(
        take(&mut world),
        [
            "remove hook Some(5)",
            "remove signal Some(5)",
            "despawn hook Some(5)",
            "despawn signal Some(5)"
        ]
    );
}

fn spawn_through_commands(mut cmd: CommandQueue) {
    cmd.spawn(Tracked(7));
}

#[test]
fn signals_fire_for_operations_applied_from_commands() {
    let mut world = world();
    let mut system = concerto_ecs::IntoSystem::into_system(spawn_through_commands);
    concerto_ecs::System::initialize(&mut system, &mut world);

    concerto_ecs::System::run_and_apply(&mut system, (), &mut world);

    assert_eq!(take(&mut world), ["add hook Some(7)", "add signal Some(7)"]);
}

#[derive(Resource, Default)]
struct Order(Vec<u32>);

fn first(_: On<Add<Unqueried>>, mut order: ResMut<Order>) {
    order.0.push(1);
}

fn second(_: On<Add<Unqueried>>, mut order: ResMut<Order>) {
    order.0.push(2);
}

#[test]
fn every_listener_runs_in_registration_order_even_for_a_type_nothing_queries() {
    let mut world = World::new();
    world.insert_resource(Order::default());
    world.add_listener(first);
    world.add_listener(second);

    world.spawn(Unqueried);

    assert_eq!(world.get_resource::<Order>().unwrap().0, [1, 2]);
}

#[derive(concerto_ecs::Component)]
struct Companion;

#[derive(Resource, Default)]
struct DuringListener(Vec<usize>);

fn spawn_a_companion(
    _: On<Add<Unqueried>>,
    companions: Query<&Companion>,
    mut during: ResMut<DuringListener>,
    mut cmd: CommandQueue,
) {
    during.0.push(companions.iter().count());
    cmd.spawn(Companion);
}

#[test]
fn a_listeners_structural_changes_apply_after_the_operation() {
    let mut world = World::new();
    world.insert_resource(DuringListener::default());
    world.add_listener(spawn_a_companion);

    world.spawn(Unqueried);

    assert_eq!(world.get_resource::<DuringListener>().unwrap().0, [0]);
    assert_eq!(world.query::<&Companion, ()>().iter(&mut world).count(), 1);
}

fn reads_the_world(_: On<Add<Unqueried>>, _: &World) {}

fn writes_the_world(_: On<Remove<Unqueried>>, _: &mut World) {}

#[test]
#[should_panic(expected = "reads_the_world")]
fn a_lifecycle_listener_that_reads_the_whole_world_is_refused() {
    World::new().add_listener(reads_the_world);
}

#[test]
#[should_panic(expected = "writes_the_world")]
fn a_lifecycle_listener_that_writes_the_whole_world_is_refused() {
    World::new().add_listener(writes_the_world);
}

#[derive(Resource)]
struct Absent;

fn needs_absent(on: On<Add<Unqueried>>, _: Res<Absent>, mut seen: ResMut<Seen>) {
    seen.0.push(on.signal().entity);
}

#[test]
fn a_lifecycle_listener_whose_resource_is_missing_is_skipped() {
    let mut world = World::new();
    world.insert_resource(Seen::default());
    world.add_listener(needs_absent);

    world.spawn(Unqueried);

    assert!(world.get_resource::<Seen>().unwrap().0.is_empty());
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
