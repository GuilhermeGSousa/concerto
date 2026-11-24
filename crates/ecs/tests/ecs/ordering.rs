use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Mutex;

use concerto_ecs::{
    CommandQueue, Component, IntoSetConfig, IntoSystemConfig, Query, Res, Resource, Schedule,
    SystemSet, SystemTag, TagFilter, World,
    resource::ResMut,
    system::executor::{
        SystemExecutor, multi_thread::MultiThreadedExecutor, single_thread::SingleThreadedExecutor,
    },
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

#[derive(Component)]
struct A;

#[derive(Component)]
struct B;

#[derive(Component)]
struct C;

#[derive(Resource, Default)]
struct Seen(Vec<usize>);

fn spawn_a(existing: Query<&A>, mut cmd: CommandQueue) {
    if existing.iter().next().is_none() {
        cmd.spawn(A);
    }
}

fn spawn_b(a: Query<&A>, b: Query<&B>, mut cmd: CommandQueue) {
    if a.iter().next().is_some() && b.iter().next().is_none() {
        cmd.spawn(B);
    }
}

fn spawn_c(b: Query<&B>, c: Query<&C>, mut cmd: CommandQueue) {
    if b.iter().next().is_some() && c.iter().next().is_none() {
        cmd.spawn(C);
    }
}

fn unrelated(log: Res<Log>) {
    push(&log, "unrelated");
}

fn count_a(a: Query<&A>, mut seen: ResMut<Seen>) {
    seen.0.push(a.iter().count());
}

fn count_c(c: Query<&C>, mut seen: ResMut<Seen>) {
    seen.0.push(c.iter().count());
}

fn count_a_exclusively(world: &mut World) {
    let count = world.query::<&A, ()>().iter(world).count();
    world.get_resource_mut::<Seen>().unwrap().0.push(count);
}

fn seen_over_two_runs<E: SystemExecutor + 'static>(build: fn(&mut Schedule)) -> Vec<usize> {
    let mut world = World::new();
    world.insert_resource(Log::default());
    world.insert_resource(Seen::default());
    let mut schedule = Schedule::new();
    build(&mut schedule);
    let mut compiled = schedule.compile::<E>(&mut world);
    compiled.run(&mut world);
    compiled.run(&mut world);
    world.remove_resource::<Seen>().unwrap().0
}

struct VisibilityCase {
    name: &'static str,
    build: fn(&mut Schedule),
    seen: [usize; 2],
}

#[test]
fn commands_are_seen_by_systems_ordered_after_them_and_by_everything_next_run() {
    let cases = [
        VisibilityCase {
            name: "a system ordered after the producer sees its spawn in the same run",
            build: |schedule| {
                schedule
                    .add_system(spawn_a)
                    .add_system(count_a.after(spawn_a));
            },
            seen: [1, 1],
        },
        VisibilityCase {
            name: "an unordered system sees the spawn only on the next run",
            build: |schedule| {
                schedule.add_system(spawn_a).add_system(count_a);
            },
            seen: [0, 1],
        },
        VisibilityCase {
            name: "a producer registered after its consumer but ordered before it is seen",
            build: |schedule| {
                schedule
                    .add_system(count_a)
                    .add_system(spawn_a.before(count_a));
            },
            seen: [1, 1],
        },
        VisibilityCase {
            name: "a chain of command systems applies each before the next",
            build: |schedule| {
                schedule.add_system((spawn_a, spawn_b, spawn_c, count_c).chain());
            },
            seen: [1, 1],
        },
        VisibilityCase {
            name: "an order through a system without commands carries the commands",
            build: |schedule| {
                schedule
                    .add_system(count_a.after(unrelated))
                    .add_system(unrelated.after(spawn_a))
                    .add_system(spawn_a);
            },
            seen: [1, 1],
        },
        VisibilityCase {
            name: "an order between sets carries the commands",
            build: |schedule| {
                schedule.configure_sets((Phase::Early, Phase::Late).chain());
                schedule
                    .add_system(count_a.in_set(Phase::Late))
                    .add_system(spawn_a.in_set(Phase::Early));
            },
            seen: [1, 1],
        },
        VisibilityCase {
            name: "a system taking the world sees what ran before it",
            build: |schedule| {
                schedule.add_system(spawn_a).add_system(count_a_exclusively);
            },
            seen: [1, 1],
        },
    ];
    for case in cases {
        assert_eq!(
            seen_over_two_runs::<SingleThreadedExecutor>(case.build),
            case.seen,
            "single-threaded: {}",
            case.name
        );
        assert_eq!(
            seen_over_two_runs::<MultiThreadedExecutor>(case.build),
            case.seen,
            "multi-threaded: {}",
            case.name
        );
    }
}

#[derive(Resource, PartialEq, Debug)]
struct Winner(&'static str);

fn slow_insert(mut cmd: CommandQueue) {
    std::thread::sleep(std::time::Duration::from_millis(5));
    cmd.insert_resource(Winner("registered first"));
}

fn fast_insert(mut cmd: CommandQueue) {
    cmd.insert_resource(Winner("registered second"));
}

fn winner<E: SystemExecutor + 'static>() -> Winner {
    let mut world = World::new();
    let mut schedule = Schedule::new();
    schedule.add_system(slow_insert).add_system(fast_insert);
    schedule.compile::<E>(&mut world).run(&mut world);
    world.remove_resource::<Winner>().unwrap()
}

#[test]
fn unordered_systems_apply_their_commands_in_registration_order_whatever_finishes_first() {
    assert_eq!(
        winner::<SingleThreadedExecutor>(),
        Winner("registered second")
    );
    for _ in 0..8 {
        assert_eq!(
            winner::<MultiThreadedExecutor>(),
            Winner("registered second")
        );
    }
}
