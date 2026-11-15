use concerto_ecs::{
    CommandQueue, Component, Entity, IntoSystem, Query, ResMut, Resource, System, World,
    signal::{
        EntitySignal, On, Signal,
        listener::{IntoListener, Listener},
    },
    system::input::SystemLocal,
    world::FromWorld,
};

#[derive(Default)]
struct TestSignal(u32);
impl Signal for TestSignal {}

struct OtherSignal;
impl Signal for OtherSignal {}

#[derive(Default)]
struct Poke(u32);
impl Signal for Poke {}
impl EntitySignal for Poke {}

#[derive(Resource, Default)]
struct Calls(Vec<u32>);

#[derive(Resource, Default)]
struct Poked(Vec<Entity>);

fn record_poked(on: On<Poke>, mut poked: ResMut<Poked>) {
    poked.0.push(on.entity());
}

#[derive(Component)]
struct TestComponent;

struct Counter(u32);

impl FromWorld for Counter {
    fn from_world(_: &World) -> Self {
        Self(0)
    }
}

fn record(world: &mut World, value: u32) {
    world.get_resource_mut::<Calls>().unwrap().0.push(value);
}

#[test]
fn trigger_runs_every_matching_listener_in_registration_order() {
    let mut world = World::new();
    world.trigger_default::<TestSignal>();
    world.insert_resource(Calls::default());
    world.add_listener(|on: On<TestSignal>, mut calls: ResMut<Calls>| {
        calls.0.push(on.signal().0 + 1);
    });
    world.add_listener(|_: On<OtherSignal>| panic!("wrong signal type"));
    world.add_listener(|on: On<TestSignal>, mut calls: ResMut<Calls>| {
        calls.0.push(on.signal().0 + 2);
    });

    world.trigger(TestSignal(10));
    world.trigger_default::<TestSignal>();

    assert_eq!(world.get_resource::<Calls>().unwrap().0, [11, 12, 1, 2]);
}

#[test]
fn listener_initialization_preserves_local_state_between_triggers() {
    let mut world = World::new();
    world.insert_resource(Calls::default());
    world.add_listener(
        |_: On<TestSignal>, mut count: SystemLocal<Counter>, mut calls: ResMut<Calls>| {
            count.0 += 1;
            calls.0.push(count.0);
        },
    );

    world.trigger_default::<TestSignal>();
    world.trigger_default::<TestSignal>();

    assert_eq!(world.get_resource::<Calls>().unwrap().0, [1, 2]);
}

#[test]
fn commands_are_applied_before_the_next_listener_queries_the_world() {
    let mut world = World::new();
    world.insert_resource(Calls::default());
    world.add_listener(|_: On<TestSignal>, mut commands: CommandQueue| {
        commands.spawn(TestComponent);
    });
    world.add_listener(
        |_: On<TestSignal>, query: Query<&TestComponent>, mut calls: ResMut<Calls>| {
            calls.0.push(query.iter().count() as u32);
        },
    );

    world.trigger_default::<TestSignal>();
    world.trigger_default::<TestSignal>();

    assert_eq!(world.get_resource::<Calls>().unwrap().0, [1, 2]);
}

#[test]
fn a_listener_can_despawn_itself_and_a_later_listener_through_commands() {
    let mut world = World::new();
    world.insert_resource(Calls::default());
    world.register_component::<Listener<TestSignal>>();
    let entity = world.spawn(());
    let target = world.spawn(());
    world.insert(
        (move |_: On<TestSignal>, mut commands: CommandQueue, mut calls: ResMut<Calls>| {
            calls.0.push(1);
            commands.despawn(entity);
            commands.despawn(target);
        })
        .into_listener(),
        entity,
    );
    world.insert(
        (|_: On<TestSignal>| panic!("despawned listener ran")).into_listener(),
        target,
    );
    world.add_listener(|_: On<TestSignal>, mut calls: ResMut<Calls>| calls.0.push(2));

    world.trigger_default::<TestSignal>();
    world.trigger_default::<TestSignal>();

    assert!(!world.entity_is_valid(entity));
    assert!(!world.entity_is_valid(target));
    assert_eq!(world.get_resource::<Calls>().unwrap().0, [1, 2, 2]);
}

#[test]
fn a_listener_can_replace_itself_without_losing_the_replacement() {
    let mut world = World::new();
    world.insert_resource(Calls::default());
    world.register_component::<Listener<TestSignal>>();
    let entity = world.spawn(());
    world.insert(
        (move |_: On<TestSignal>, world: &mut World| {
            record(world, 1);
            world.insert(
                (|_: On<TestSignal>, mut count: SystemLocal<Counter>, mut calls: ResMut<Calls>| {
                    count.0 += 10;
                    calls.0.push(count.0);
                })
                .into_listener(),
                entity,
            );
        })
        .into_listener(),
        entity,
    );

    world.trigger_default::<TestSignal>();
    world.trigger_default::<TestSignal>();
    world.trigger_default::<TestSignal>();

    assert_eq!(world.get_resource::<Calls>().unwrap().0, [1, 10, 20]);
}

#[test]
fn active_listener_state_survives_archetype_moves() {
    let mut world = World::new();
    world.insert_resource(Calls::default());
    world.register_component::<Listener<TestSignal>>();
    let entity = world.spawn(());
    world.insert(
        (move |_: On<TestSignal>, world: &mut World, mut count: SystemLocal<Counter>| {
            count.0 += 1;
            world.insert(TestComponent, entity);
            for _ in 0..32 {
                world.spawn(TestComponent);
            }
            record(world, count.0);
        })
        .into_listener(),
        entity,
    );

    world.trigger_default::<TestSignal>();
    world.trigger_default::<TestSignal>();

    assert_eq!(world.get_resource::<Calls>().unwrap().0, [1, 2]);
}

#[test]
fn queued_signals_are_deferred_and_observe_command_order() {
    let mut world = World::new();
    world.insert_resource(Calls::default());
    world.add_listener(
        |on: On<TestSignal>, query: Query<&TestComponent>, mut calls: ResMut<Calls>| {
            calls.0.push(on.signal().0 + query.iter().count() as u32);
        },
    );
    let target = world.spawn(
        (|on: On<Poke>, query: Query<&TestComponent>, mut calls: ResMut<Calls>| {
            calls.0.push(on.signal().0 + query.iter().count() as u32);
        })
        .into_listener(),
    );
    let mut system = (move |mut commands: CommandQueue| {
        commands.spawn(TestComponent);
        commands.trigger(TestSignal(10));
        commands.spawn(TestComponent);
        commands.entity(target).trigger(Poke(20));
    })
    .into_system();
    system.initialize(&mut world);

    system.run((), &mut world);
    assert!(world.get_resource::<Calls>().unwrap().0.is_empty());

    system.apply(&mut world);
    assert_eq!(world.get_resource::<Calls>().unwrap().0, [11, 22]);

    system.apply(&mut world);
    assert_eq!(world.get_resource::<Calls>().unwrap().0, [11, 22]);
}

#[test]
fn trigger_on_runs_only_the_target_listener_and_trigger_runs_each_with_its_entity() {
    let mut world = World::new();
    world.insert_resource(Poked::default());
    world.register_component::<Listener<Poke>>();
    let first = world.spawn(record_poked.into_listener());
    let second = world.spawn(record_poked.into_listener());

    world.trigger_on(second, Poke(0));
    world.trigger(Poke(0));

    assert_eq!(
        world.get_resource::<Poked>().unwrap().0,
        [second, first, second]
    );
}

#[derive(Resource)]
struct GateOpen(bool);

struct Gate;

impl concerto_ecs::system::input::SystemInput for Gate {
    type State = ();
    type Data<'world, 'state> = Gate;

    fn init_state(_world: &mut World) -> Self::State {}

    unsafe fn get_data<'world, 'state>(
        _state: &'state mut Self::State,
        world: concerto_ecs::world::UnsafeWorldCell<'world>,
        _ticks: concerto_ecs::component::SystemTicks,
    ) -> Result<Self::Data<'world, 'state>, concerto_ecs::system::input::ValidationError> {
        // SAFETY: reads only `GateOpen`, which `fill_access` declares.
        let open = unsafe { world.world() }
            .get_resource::<GateOpen>()
            .is_some_and(|open| open.0);
        if open {
            Ok(Gate)
        } else {
            Err(concerto_ecs::system::input::ValidationError::skip::<Self>(
                "the gate is closed",
            ))
        }
    }

    fn fill_access(
        _meta: &mut concerto_ecs::system::meta::SystemMetadata,
        access: &mut concerto_ecs::system::access::SystemAccess,
    ) {
        access.read_resource::<GateOpen>();
    }
}

#[test]
fn a_listener_whose_parameter_declines_does_not_run() {
    let mut world = World::new();
    world.insert_resource(Calls::default());
    world.insert_resource(GateOpen(false));
    world.add_listener(|on: On<TestSignal>, _: Gate, mut calls: ResMut<Calls>| {
        calls.0.push(on.signal().0);
    });

    world.trigger(TestSignal(1));
    assert!(world.get_resource::<Calls>().unwrap().0.is_empty());

    world.insert_resource(GateOpen(true));
    world.trigger(TestSignal(2));
    assert_eq!(world.get_resource::<Calls>().unwrap().0, vec![2]);
}

#[test]
fn a_global_listener_entity_is_marked_and_an_attached_listener_is_not() {
    use concerto_ecs::{query::filter::With, signal::listener::GlobalListener};

    let mut world = World::new();
    world.add_listener(|_: On<TestSignal>| {});
    let attached = world.spawn((TestComponent, (|_: On<Poke>| {}).into_listener()));

    let marked: Vec<Entity> = world
        .query::<Entity, With<GlobalListener>>()
        .iter(&mut world)
        .collect();
    let listeners: Vec<Entity> = world
        .query::<Entity, With<Listener<TestSignal>>>()
        .iter(&mut world)
        .collect();

    assert_eq!(marked, listeners);
    assert_eq!(marked.len(), 1);
    assert!(
        world
            .get_component_for_entity::<GlobalListener>(attached)
            .is_none()
    );
}
