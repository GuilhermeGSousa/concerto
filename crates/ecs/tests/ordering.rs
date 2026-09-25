use concerto_ecs::{
    IntoSetConfigs, IntoSystemConfig, IntoSystemConfigs, Resource, Schedule, SystemSet, World,
    resource::ResMut,
    system::{IntoSetConfig, executor::single_thread::SingleThreadedExecutor},
};

#[derive(Resource, Default)]
struct Log(Vec<&'static str>);

#[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
enum Phase {
    Early,
    Late,
    Unused,
}

fn first(mut log: ResMut<Log>) {
    log.0.push("first");
}

fn second(mut log: ResMut<Log>) {
    log.0.push("second");
}

fn third(mut log: ResMut<Log>) {
    log.0.push("third");
}

fn run(schedule: Schedule) -> Vec<&'static str> {
    let mut world = World::new();
    world.insert_resource(Log::default());
    schedule
        .compile::<SingleThreadedExecutor>(&mut world)
        .run(&mut world);
    world.remove_resource::<Log>().unwrap().0
}

#[test]
fn a_referenced_system_runs_once() {
    let mut schedule = Schedule::new();
    schedule.add_system(first);
    schedule.add_system(second.after(first));

    assert_eq!(run(schedule), vec!["first", "second"]);
}

#[test]
fn a_system_referenced_by_two_constraints_runs_once_per_run() {
    let mut schedule = Schedule::new();
    schedule.add_system(second.after(first));
    schedule.add_system(third.after(first));
    schedule.add_system(first);

    assert_eq!(run(schedule), vec!["first", "second", "third"]);
}

#[test]
fn a_constraint_can_name_a_system_registered_later() {
    let mut schedule = Schedule::new();
    schedule.add_system(second.after(first));
    schedule.add_system(first);

    assert_eq!(run(schedule), vec!["first", "second"]);
}

#[test]
fn an_explicit_constraint_beats_registration_order() {
    let mut schedule = Schedule::new();
    schedule.add_system(second);
    schedule.add_system(first.before(second));

    assert_eq!(run(schedule), vec!["first", "second"]);
}

#[test]
fn conflicting_systems_without_constraints_keep_registration_order() {
    let mut schedule = Schedule::new();
    schedule.add_system(second);
    schedule.add_system(first);

    assert_eq!(run(schedule), vec!["second", "first"]);
}

#[test]
fn a_target_that_is_not_in_the_schedule_is_ignored() {
    let mut schedule = Schedule::new();
    schedule.add_system(second.after(first));

    assert_eq!(run(schedule), vec!["second"]);
}

#[test]
fn a_constraint_orders_against_every_copy_of_a_twice_added_target() {
    let mut schedule = Schedule::new();
    schedule.add_system(second.after(first));
    schedule.add_system(first);
    schedule.add_system(first);

    assert_eq!(run(schedule), vec!["first", "first", "second"]);
}

#[test]
fn sets_order_their_members() {
    let mut schedule = Schedule::new();
    schedule.configure_sets((Phase::Early, Phase::Late).chain());
    schedule.add_system(second.in_set(Phase::Late));
    schedule.add_system(first.in_set(Phase::Early));

    assert_eq!(run(schedule), vec!["first", "second"]);
}

#[test]
fn configuring_a_set_twice_accumulates_constraints() {
    let mut schedule = Schedule::new();
    schedule.configure_sets(Phase::Late.after(Phase::Early));
    schedule.configure_sets(Phase::Late.before(third));
    schedule.add_system(third);
    schedule.add_system(second.in_set(Phase::Late));
    schedule.add_system(first.in_set(Phase::Early));

    assert_eq!(run(schedule), vec!["first", "second", "third"]);
}

#[test]
fn a_system_can_be_ordered_before_a_set() {
    let mut schedule = Schedule::new();
    schedule.add_system(second.in_set(Phase::Late));
    schedule.add_system(first.before(Phase::Late));

    assert_eq!(run(schedule), vec!["first", "second"]);
}

#[test]
fn a_system_can_be_ordered_after_a_set() {
    let mut schedule = Schedule::new();
    schedule.add_system(second.after(Phase::Early));
    schedule.add_systems((first, third).in_set(Phase::Early));

    let order = run(schedule);
    assert_eq!(order.last(), Some(&"second"));
    assert_eq!(order.len(), 3);
}

#[test]
fn an_empty_set_named_as_a_target_is_ignored() {
    let mut schedule = Schedule::new();
    schedule.add_system(first.before(Phase::Unused));

    assert_eq!(run(schedule), vec!["first"]);
}

#[test]
fn a_system_in_a_set_is_not_self_ordered_by_that_set() {
    let mut schedule = Schedule::new();
    schedule.configure_sets(Phase::Late.after(Phase::Early));
    schedule.add_system(first.in_set(Phase::Early).in_set(Phase::Late));
    schedule.add_system(second.in_set(Phase::Late));

    assert_eq!(run(schedule), vec!["first", "second"]);
}

#[test]
#[should_panic(expected = "Cycle in schedule ordering")]
fn a_cycle_created_by_set_expansion_panics() {
    let mut schedule = Schedule::new();
    schedule.configure_sets((Phase::Early, Phase::Late).chain());
    schedule.add_system(first.in_set(Phase::Early).after(second));
    schedule.add_system(second.in_set(Phase::Late));

    let mut world = World::new();
    world.insert_resource(Log::default());
    let _ = schedule.compile::<SingleThreadedExecutor>(&mut world);
}

#[test]
fn a_tuple_of_systems_can_join_one_set() {
    let mut schedule = Schedule::new();
    schedule.configure_sets((Phase::Early, Phase::Late).chain());
    schedule.add_systems((second, third).in_set(Phase::Late));
    schedule.add_systems(first.in_set(Phase::Early));

    let order = run(schedule);
    assert_eq!(order, vec!["first", "second", "third"]);
}

#[test]
fn chain_orders_a_tuple_against_registration_order() {
    let mut schedule = Schedule::new();
    let configs: Vec<_> = (second, first).chain().into_iter().rev().collect();
    schedule.add_systems(configs);

    assert_eq!(run(schedule), vec!["second", "first"]);
}

#[test]
fn chain_orders_sets_against_registration_order() {
    let mut schedule = Schedule::new();
    schedule.configure_sets((Phase::Late, Phase::Early).chain());
    schedule.add_system(second.in_set(Phase::Early));
    schedule.add_system(first.in_set(Phase::Late));
    schedule.add_system(third.in_set(Phase::Early));

    assert_eq!(run(schedule), vec!["first", "second", "third"]);
}
