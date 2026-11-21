use concerto_ecs::{
    IntoSystem, Res, ResMut, Resource, System, World,
    events::{Event, event_writer::EventWriter},
    signal::{On, Signal},
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

fn writes_unregistered(_: EventWriter<Unregistered>, mut runs: ResMut<Runs>) {
    runs.0.push("writes_unregistered");
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

struct SkipCase {
    name: &'static str,
    run: fn() -> Vec<&'static str>,
}

#[test]
fn a_system_whose_resource_or_event_is_missing_is_skipped() {
    let cases = [
        SkipCase {
            name: "reads a missing resource",
            run: || run_alone(reads_absent),
        },
        SkipCase {
            name: "writes an unregistered event",
            run: || run_alone(writes_unregistered),
        },
    ];
    for case in cases {
        assert!((case.run)().is_empty(), "{}", case.name);
    }
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
