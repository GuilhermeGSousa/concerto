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
    Unused,
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

fn without_dropped() -> TagFilter {
    TagFilter {
        include: Vec::new(),
        exclude: vec![Dropped.intern()],
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
            name: "a referenced system runs once",
            build: |schedule| {
                schedule.add_system(first);
                schedule.add_system(second.after(first));
            },
            filter: everything,
            order: &["first", "second"],
        },
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
            name: "a constraint can name a system registered later",
            build: |schedule| {
                schedule.add_system(second.after(first));
                schedule.add_system(first);
            },
            filter: everything,
            order: &["first", "second"],
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
            name: "a target that is not in the schedule is ignored",
            build: |schedule| {
                schedule.add_system(second.after(first));
            },
            filter: everything,
            order: &["second"],
        },
        OrderCase {
            name: "a constraint orders against every copy of a twice added target",
            build: |schedule| {
                schedule.add_system(second.after(first));
                schedule.add_system(first);
                schedule.add_system(first);
            },
            filter: everything,
            order: &["first", "first", "second"],
        },
        OrderCase {
            name: "a system ordered after itself runs once",
            build: |schedule| {
                schedule.add_system(first.after(first));
            },
            filter: everything,
            order: &["first"],
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
            name: "before alone orders systems that share no data",
            build: |schedule| {
                schedule.add_system(two);
                schedule.add_system(one.before(two));
            },
            filter: everything,
            order: &["one", "two"],
        },
        OrderCase {
            name: "chain orders a vec of systems",
            build: |schedule| {
                schedule.add_system(
                    vec![
                        third.into_config(),
                        second.into_config(),
                        first.into_config(),
                    ]
                    .chain(),
                );
            },
            filter: everything,
            order: &["third", "second", "first"],
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
            name: "a system can be ordered before a set",
            build: |schedule| {
                schedule.add_system(second.in_set(Phase::Late));
                schedule.add_system(first.before(Phase::Late));
            },
            filter: everything,
            order: &["first", "second"],
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
            name: "an empty set named as a target is ignored",
            build: |schedule| {
                schedule.add_system(first.before(Phase::Unused));
            },
            filter: everything,
            order: &["first"],
        },
        OrderCase {
            name: "a system in a set is not ordered against itself by that set",
            build: |schedule| {
                schedule.configure_sets(Phase::Late.after(Phase::Early));
                schedule.add_system(first.in_set(Phase::Early).in_set(Phase::Late));
                schedule.add_system(second.in_set(Phase::Late));
            },
            filter: everything,
            order: &["first", "second"],
        },
        OrderCase {
            name: "a tuple of systems can join one set",
            build: |schedule| {
                schedule.configure_sets((Phase::Early, Phase::Late).chain());
                schedule.add_system((second, third).in_set(Phase::Late));
                schedule.add_system(first.in_set(Phase::Early));
            },
            filter: everything,
            order: &["first", "second", "third"],
        },
        OrderCase {
            name: "a tuple of sets can be ordered as one",
            build: |schedule| {
                schedule.configure_sets((Phase::Late, Phase::Unused).after(Phase::Early));
                schedule.add_system(second.in_set(Phase::Late));
                schedule.add_system(third.in_set(Phase::Unused));
                schedule.add_system(first.in_set(Phase::Early));
            },
            filter: everything,
            order: &["first", "second", "third"],
        },
        OrderCase {
            name: "chain orders sets against registration order",
            build: |schedule| {
                schedule.configure_sets((Phase::Late, Phase::Early).chain());
                schedule.add_system(second.in_set(Phase::Early));
                schedule.add_system(first.in_set(Phase::Late));
                schedule.add_system(third.in_set(Phase::Early));
            },
            filter: everything,
            order: &["first", "second", "third"],
        },
        OrderCase {
            name: "an empty filter runs tagged systems",
            build: |schedule| {
                schedule.add_system((first.tagged(Kept), second).chain());
            },
            filter: everything,
            order: &["first", "second"],
        },
        OrderCase {
            name: "an include filter runs only tagged systems",
            build: |schedule| {
                schedule.add_system((first.tagged(Kept), second).chain());
            },
            filter: only_kept,
            order: &["first"],
        },
        OrderCase {
            name: "an exclude filter drops tagged systems",
            build: |schedule| {
                schedule.add_system((first.tagged(Dropped), second).chain());
            },
            filter: without_dropped,
            order: &["second"],
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

struct CycleCase {
    name: &'static str,
    build: fn(&mut Schedule),
    names: &'static [&'static str],
}

#[test]
fn a_cycle_in_the_constraints_panics_naming_its_systems() {
    let cases = [
        CycleCase {
            name: "contradicting explicit constraints",
            build: |schedule| {
                schedule.add_system(first.before(second));
                schedule.add_system(second.before(first));
            },
            names: &["Cycle in schedule ordering", "first", "second"],
        },
        CycleCase {
            name: "a cycle created by set expansion",
            build: |schedule| {
                schedule.configure_sets((Phase::Early, Phase::Late).chain());
                schedule.add_system(first.in_set(Phase::Early).after(second));
                schedule.add_system(second.in_set(Phase::Late));
            },
            names: &["Cycle in schedule ordering"],
        },
    ];
    for case in cases {
        let mut schedule = Schedule::new();
        (case.build)(&mut schedule);
        let mut world = World::new();

        let payload = catch_unwind(AssertUnwindSafe(|| {
            schedule.compile::<SingleThreadedExecutor>(&mut world);
        }))
        .expect_err(case.name);

        let message = payload
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| payload.downcast_ref::<&str>().copied())
            .unwrap();
        for name in case.names {
            assert!(message.contains(name), "{}: {message}", case.name);
        }
    }
}
