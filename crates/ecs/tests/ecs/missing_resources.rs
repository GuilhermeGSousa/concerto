use concerto_ecs::{
    IntoSystem, Res, ResMut, Resource, Schedule, System, World,
    events::{Event, event_reader::EventReader, event_writer::EventWriter},
    signal::{On, Signal},
    system::executor::{
        multi_thread::MultiThreadedExecutor, single_thread::SingleThreadedExecutor,
    },
};

#[derive(Resource)]
struct Absent;

#[derive(Resource, Default)]
struct Runs(Vec<&'static str>);

#[derive(Event)]
struct Unregistered;

fn reads_absent(_: Res<Absent>, mut runs: ResMut<Runs>) {
    runs.0.push("reads_absent");
}

fn writes_absent(_: ResMut<Absent>, mut runs: ResMut<Runs>) {
    runs.0.push("writes_absent");
}

fn reads_unregistered(_: EventReader<Unregistered>, mut runs: ResMut<Runs>) {
    runs.0.push("reads_unregistered");
}

fn writes_unregistered(_: EventWriter<Unregistered>, mut runs: ResMut<Runs>) {
    runs.0.push("writes_unregistered");
}

fn present(mut runs: ResMut<Runs>) {
    runs.0.push("present");
}

fn world() -> World {
    let mut world = World::new();
    world.insert_resource(Runs::default());
    world
}

fn runs(world: &World) -> Vec<&'static str> {
    world.get_resource::<Runs>().unwrap().0.clone()
}

fn run_alone<M>(system: impl IntoSystem<(), M>) -> Vec<&'static str> {
    let mut world = world();
    let mut system = system.into_system();
    system.initialize(&mut world);
    system.run_and_apply((), &mut world);
    runs(&world)
}

#[test]
fn a_system_reading_a_missing_resource_is_skipped() {
    assert!(run_alone(reads_absent).is_empty());
}

#[test]
fn a_system_writing_a_missing_resource_is_skipped() {
    assert!(run_alone(writes_absent).is_empty());
}

#[test]
fn a_system_reading_an_unregistered_event_is_skipped() {
    assert!(run_alone(reads_unregistered).is_empty());
}

#[test]
fn a_system_writing_an_unregistered_event_is_skipped() {
    assert!(run_alone(writes_unregistered).is_empty());
}

fn schedule() -> Schedule {
    let mut schedule = Schedule::new();
    schedule.add_system(reads_absent);
    schedule.add_system(writes_absent);
    schedule.add_system(present);
    schedule
}

#[test]
fn the_single_threaded_executor_skips_a_system_with_a_missing_resource_and_runs_the_rest() {
    let mut world = world();
    schedule()
        .compile::<SingleThreadedExecutor>(&mut world)
        .run(&mut world);

    assert_eq!(runs(&world), vec!["present"]);
}

#[test]
fn the_multithreaded_executor_skips_a_system_with_a_missing_resource_and_runs_the_rest() {
    let mut world = world();
    schedule()
        .compile::<MultiThreadedExecutor>(&mut world)
        .run(&mut world);

    assert_eq!(runs(&world), vec!["present"]);
}

struct Ping;
impl Signal for Ping {}

fn ping_reads_absent(_: On<Ping>, _: Res<Absent>, mut runs: ResMut<Runs>) {
    runs.0.push("ping_reads_absent");
}

fn ping_present(_: On<Ping>, mut runs: ResMut<Runs>) {
    runs.0.push("ping_present");
}

#[test]
fn a_listener_reading_a_missing_resource_is_skipped_and_the_others_run() {
    let mut world = world();
    world.add_listener(ping_reads_absent);
    world.add_listener(ping_present);

    world.trigger(Ping);

    assert_eq!(runs(&world), vec!["ping_present"]);
}
