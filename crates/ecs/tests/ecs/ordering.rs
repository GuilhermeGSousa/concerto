use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Mutex;

use concerto_ecs::{
    CommandQueue, IntoSetConfig, IntoSystemConfig, Res, Resource, Schedule, SystemSet, SystemTag,
    TagFilter, World, resource::ResMut, system::executor::single_thread::SingleThreadedExecutor,
};

#[derive(Resource, Default)]
struct Log(Mutex<Vec<&'static str>>);

#[derive(Resource, Default)]
struct Conflict;

#[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
enum Phase {
    Early,
    Late,
}

#[derive(SystemTag, Clone, PartialEq, Eq, Hash, Debug)]
struct Kept;

#[derive(SystemTag, Clone, PartialEq, Eq, Hash, Debug)]
struct Dropped;

fn push(log: &Log, name: &'static str) {
    log.0.lock().unwrap().push(name);
}

fn first(_: ResMut<Conflict>, log: Res<Log>) {
    push(&log, "first");
}

fn second(_: ResMut<Conflict>, log: Res<Log>) {
    push(&log, "second");
}

fn third(_: ResMut<Conflict>, log: Res<Log>) {
    push(&log, "third");
}

fn queues(_: CommandQueue, _: ResMut<Conflict>, log: Res<Log>) {
    push(&log, "queues");
}

fn one(log: Res<Log>) {
    push(&log, "one");
}

fn two(log: Res<Log>) {
    push(&log, "two");
}

fn three(log: Res<Log>) {
    push(&log, "three");
}

fn run(schedule: Schedule, filter: &TagFilter) -> Vec<&'static str> {
    let mut world = World::new();
    world.insert_resource(Log::default());
    world.insert_resource(Conflict);
    schedule
        .compile_filtered::<SingleThreadedExecutor>(&mut world, filter)
        .run(&mut world);
    world
        .remove_resource::<Log>()
        .unwrap()
        .0
        .into_inner()
        .unwrap()
}

fn everything() -> TagFilter {
    TagFilter::default()
}

fn only_kept() -> TagFilter {
    TagFilter {
        include: vec![Kept.intern()],
        exclude: Vec::new(),
    }
}

fn kept_without_dropped() -> TagFilter {
    TagFilter {
        include: vec![Kept.intern()],
        exclude: vec![Dropped.intern()],
    }
}

struct OrderCase {
    name: &'static str,
    build: fn(&mut Schedule),
    filter: fn() -> TagFilter,
    order: &'static [&'static str],
}

#[test]
fn a_schedule_runs_each_system_once_in_constraint_then_registration_order() {
    let cases = [
        OrderCase {
            name: "a system referenced by two constraints runs once",
            build: |schedule| {
                schedule.add_system(second.after(first));
                schedule.add_system(third.after(first));
                schedule.add_system(first);
            },
            filter: everything,
            order: &["first", "second", "third"],
        },
        OrderCase {
            name: "an explicit constraint beats registration order",
            build: |schedule| {
                schedule.add_system(second);
                schedule.add_system(first.before(second));
            },
            filter: everything,
            order: &["first", "second"],
        },
        OrderCase {
            name: "conflicting systems without constraints keep registration order",
            build: |schedule| {
                schedule.add_system(second);
                schedule.add_system(first);
            },
            filter: everything,
            order: &["second", "first"],
        },
        OrderCase {
            name: "a constraint across a sync point holds",
            build: |schedule| {
                schedule.add_system(queues);
                schedule.add_system(first.before(queues));
            },
            filter: everything,
            order: &["first", "queues"],
        },
        OrderCase {
            name: "constraints alone order systems that share no data",
            build: |schedule| {
                schedule.add_system(three.after(two));
                schedule.add_system(two.after(one));
                schedule.add_system(one);
            },
            filter: everything,
            order: &["one", "two", "three"],
        },
        OrderCase {
            name: "chain orders a nested group as a unit",
            build: |schedule| {
                schedule.add_system((one, (first, second), two).chain());
            },
            filter: everything,
            order: &["one", "first", "second", "two"],
        },
        OrderCase {
            name: "a chained group orders against a system registered before it",
            build: |schedule| {
                schedule.add_system(third);
                schedule.add_system((first, second).chain().before(third));
            },
            filter: everything,
            order: &["first", "second", "third"],
        },
        OrderCase {
            name: "sets order their members",
            build: |schedule| {
                schedule.configure_sets((Phase::Early, Phase::Late).chain());
                schedule.add_system(second.in_set(Phase::Late));
                schedule.add_system(first.in_set(Phase::Early));
            },
            filter: everything,
            order: &["first", "second"],
        },
        OrderCase {
            name: "configuring a set twice accumulates constraints",
            build: |schedule| {
                schedule.configure_sets(Phase::Late.after(Phase::Early));
                schedule.configure_sets(Phase::Late.before(third));
                schedule.add_system(third);
                schedule.add_system(second.in_set(Phase::Late));
                schedule.add_system(first.in_set(Phase::Early));
            },
            filter: everything,
            order: &["first", "second", "third"],
        },
        OrderCase {
            name: "a system can be ordered after a set",
            build: |schedule| {
                schedule.add_system(second.after(Phase::Early));
                schedule.add_system((first, third).in_set(Phase::Early));
            },
            filter: everything,
            order: &["first", "third", "second"],
        },
        OrderCase {
            name: "an exclude filter wins over an include filter",
            build: |schedule| {
                schedule
                    .add_system((first.tagged(Kept).tagged(Dropped), second.tagged(Kept)).chain());
            },
            filter: kept_without_dropped,
            order: &["second"],
        },
        OrderCase {
            name: "an order through a filtered out system still holds",
            build: |schedule| {
                schedule
                    .add_system(first.tagged(Kept).after(third))
                    .add_system(second.tagged(Kept))
                    .add_system(third.after(second));
            },
            filter: only_kept,
            order: &["second", "first"],
        },
        OrderCase {
            name: "tagging a tuple tags every system in it",
            build: |schedule| {
                schedule.add_system((first, second).tagged(Kept));
                schedule.add_system(third);
            },
            filter: only_kept,
            order: &["first", "second"],
        },
    ];
    for case in cases {
        let mut schedule = Schedule::new();
        (case.build)(&mut schedule);

        assert_eq!(run(schedule, &(case.filter)()), case.order, "{}", case.name);
    }
}

#[test]
fn a_cycle_through_a_set_panics() {
    let mut schedule = Schedule::new();
    schedule.configure_sets((Phase::Early, Phase::Late).chain());
    schedule.add_system(first.in_set(Phase::Early).after(second));
    schedule.add_system(second.in_set(Phase::Late));
    let mut world = World::new();

    let outcome = catch_unwind(AssertUnwindSafe(|| {
        schedule.compile::<SingleThreadedExecutor>(&mut world);
    }));

    assert!(outcome.is_err());
}
