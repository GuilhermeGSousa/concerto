use std::panic::{AssertUnwindSafe, catch_unwind};

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
fn a_listener_can_despawn_itself_through_commands() {
    let mut world = World::new();
    world.insert_resource(Calls::default());
    world.register_component::<Listener<TestSignal>>();
    let entity = world.spawn(());
    world.insert(
        (move |_: On<TestSignal>, mut commands: CommandQueue, mut calls: ResMut<Calls>| {
            calls.0.push(1);
            commands.despawn(entity);
        })
        .into_listener(),
        entity,
    );
    world.add_listener(|_: On<TestSignal>, mut calls: ResMut<Calls>| calls.0.push(2));

    world.trigger_default::<TestSignal>();
    world.trigger_default::<TestSignal>();

    assert!(!world.entity_is_valid(entity));
    assert_eq!(world.get_resource::<Calls>().unwrap().0, [1, 2, 2]);
}

#[test]
fn a_listener_removed_before_its_turn_is_skipped() {
    let mut world = World::new();
    world.register_component::<Listener<TestSignal>>();
    let target = world.spawn(());
    world.add_listener(move |_: On<TestSignal>, mut commands: CommandQueue| {
        commands.despawn(target);
    });
    world.insert(
        (|_: On<TestSignal>| panic!("despawned listener ran")).into_listener(),
        target,
    );

    world.trigger_default::<TestSignal>();

    assert!(!world.entity_is_valid(target));
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
fn listeners_added_during_dispatch_start_on_the_next_trigger() {
    let mut world = World::new();
    world.insert_resource(Calls::default());
    let mut added = false;
    world.add_listener(move |_: On<TestSignal>, world: &mut World| {
        record(world, 1);
        if !added {
            added = true;
            world.add_listener(|_: On<TestSignal>, mut calls: ResMut<Calls>| calls.0.push(2));
        }
    });

    world.trigger_default::<TestSignal>();
    assert_eq!(world.get_resource::<Calls>().unwrap().0, [1]);
    world.trigger_default::<TestSignal>();
    assert_eq!(world.get_resource::<Calls>().unwrap().0, [1, 1, 2]);
}

#[test]
fn panic_restores_the_listener_and_its_local_state() {
    let mut world = World::new();
    world.insert_resource(Calls::default());
    world.add_listener(
        |_: On<TestSignal>, mut count: SystemLocal<Counter>, mut calls: ResMut<Calls>| {
            count.0 += 1;
            assert_ne!(count.0, 1, "first run panics");
            calls.0.push(count.0);
        },
    );

    assert!(
        catch_unwind(AssertUnwindSafe(|| {
            world.trigger_default::<TestSignal>();
        }))
        .is_err()
    );
    world.trigger_default::<TestSignal>();

    assert_eq!(world.get_resource::<Calls>().unwrap().0, [2]);
}

#[test]
fn listeners_can_trigger_other_signals_synchronously() {
    let mut world = World::new();
    world.insert_resource(Calls::default());
    world.add_listener(|_: On<TestSignal>, world: &mut World| {
        record(world, 1);
        world.trigger(OtherSignal);
        record(world, 3);
    });
    world.add_listener(|_: On<OtherSignal>, mut calls: ResMut<Calls>| calls.0.push(2));

    world.trigger_default::<TestSignal>();

    assert_eq!(world.get_resource::<Calls>().unwrap().0, [1, 2, 3]);
}

#[test]
fn recursively_running_an_active_listener_panics_and_restores_its_state() {
    let mut world = World::new();
    world.insert_resource(Calls::default());
    world.add_listener(|on: On<TestSignal>, world: &mut World| {
        if on.signal().0 == 0 {
            world.trigger(TestSignal(1));
        }
        record(world, on.signal().0);
    });

    let panic = catch_unwind(AssertUnwindSafe(|| {
        world.trigger_default::<TestSignal>();
    }))
    .unwrap_err();
    let message = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied())
        .unwrap();
    assert!(message.contains("Cannot recursively run an active listener"));
    world.trigger(TestSignal(2));

    assert_eq!(world.get_resource::<Calls>().unwrap().0, [2]);
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
    let mut system = (|mut commands: CommandQueue| {
        commands.spawn(TestComponent);
        commands.trigger(TestSignal(10));
        commands.spawn(TestComponent);
        commands.trigger(TestSignal(20));
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
fn listeners_can_queue_other_signals() {
    let mut world = World::new();
    world.insert_resource(Calls::default());
    world.add_listener(
        |on: On<TestSignal>, mut commands: CommandQueue, mut calls: ResMut<Calls>| {
            calls.0.push(on.signal().0);
            commands.trigger(OtherSignal);
        },
    );
    world.add_listener(|_: On<OtherSignal>, mut calls: ResMut<Calls>| calls.0.push(2));
    world.add_listener(|_: On<TestSignal>, mut calls: ResMut<Calls>| calls.0.push(3));

    world.trigger(TestSignal(1));

    assert_eq!(world.get_resource::<Calls>().unwrap().0, [1, 2, 3]);
}

#[test]
fn trigger_on_runs_only_the_target_listener() {
    let mut world = World::new();
    world.insert_resource(Poked::default());
    let first = world.spawn(record_poked.into_listener());
    let second = world.spawn(record_poked.into_listener());
    world.add_listener(|_: On<Poke>| panic!("listener on another entity ran"));

    world.trigger_on(second, Poke(0));
    world.trigger_on(first, Poke(0));

    assert_eq!(world.get_resource::<Poked>().unwrap().0, [second, first]);
}

#[test]
fn broadcast_listeners_see_their_own_entity() {
    let mut world = World::new();
    world.insert_resource(Poked::default());
    world.register_component::<Listener<Poke>>();
    let first = world.spawn(record_poked.into_listener());
    let second = world.spawn(record_poked.into_listener());

    world.trigger(Poke(0));

    assert_eq!(world.get_resource::<Poked>().unwrap().0, [first, second]);
}

#[test]
fn trigger_on_skips_entities_without_a_live_listener() {
    let mut world = World::new();
    world.insert_resource(Poked::default());
    let bare = world.spawn(TestComponent);
    let despawned = world.spawn(record_poked.into_listener());
    world.despawn(despawned);

    world.trigger_on(bare, Poke(0));
    world.trigger_on(despawned, Poke(0));

    assert!(world.get_resource::<Poked>().unwrap().0.is_empty());
}

#[test]
fn queued_entity_signals_are_deferred_and_observe_command_order() {
    let mut world = World::new();
    world.insert_resource(Calls::default());
    let target = world.spawn(
        (|on: On<Poke>, query: Query<&TestComponent>, mut calls: ResMut<Calls>| {
            calls.0.push(on.signal().0 + query.iter().count() as u32);
        })
        .into_listener(),
    );
    let mut system = (move |mut commands: CommandQueue| {
        commands.spawn(TestComponent);
        commands.entity(target).trigger(Poke(10));
        commands.spawn(TestComponent);
        commands.entity(target).trigger(Poke(20));
    })
    .into_system();
    system.initialize(&mut world);

    system.run((), &mut world);
    assert!(world.get_resource::<Calls>().unwrap().0.is_empty());

    system.apply(&mut world);
    assert_eq!(world.get_resource::<Calls>().unwrap().0, [11, 22]);
}
