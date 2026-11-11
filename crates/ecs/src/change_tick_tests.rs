use crate::{
    Added, Changed, CommandQueue, Component, Entity, IntoSystem, IntoSystemConfig, Query, Ref, Res,
    ResMut, Resource, Schedule, System, World,
    component::{CHECK_TICK_THRESHOLD, CheckChangeTicks, MAX_CHANGE_AGE, Tick},
    query::change_detection::DetectChanges,
    signal::{On, Signal},
    system::{
        executor::{SystemExecutor, single_thread::SingleThreadedExecutor},
        input::ForeignTicks,
        schedule::{ScheduleLabel, Schedules},
    },
    testing::{Gate, GateOpen},
};

#[derive(Component, PartialEq)]
struct Value(u32);

#[derive(Component)]
struct Marker;

#[derive(Resource, Default, PartialEq)]
struct Level(u32);

#[derive(Resource, Default)]
struct Seen(Vec<usize>);

#[derive(Resource, Default)]
struct Flags(Vec<bool>);

#[derive(Resource)]
struct WriteNow(bool);

#[derive(Resource)]
struct Target(Entity);

struct Look;
impl Signal for Look {}

fn count_changed_markers(markers: Query<Entity, Changed<Marker>>, mut seen: ResMut<Seen>) {
    seen.0.push(markers.iter().count());
}

fn insert_marker_once(mut write: ResMut<WriteNow>, target: Res<Target>, mut cmd: CommandQueue) {
    if write.0 {
        write.0 = false;
        cmd.insert(Marker, target.0);
    }
}

fn sync_point_case<E: SystemExecutor + 'static>() -> Vec<usize> {
    let mut world = World::new();
    let target = world.spawn(Value(0));
    world.insert_resource(Target(target));
    world.insert_resource(WriteNow(true));
    world.insert_resource(Seen::default());

    let mut schedule = Schedule::new();
    schedule.add_system(count_changed_markers);
    schedule.add_system(insert_marker_once.after(count_changed_markers));
    let mut schedule = schedule.compile::<E>(&mut world);

    for _ in 0..3 {
        schedule.run(&mut world);
        world.tick();
    }
    world.remove_resource::<Seen>().unwrap().0
}

#[test]
fn a_command_applied_after_the_reader_is_seen_on_its_next_run() {
    assert_eq!(sync_point_case::<SingleThreadedExecutor>(), [0, 1, 0]);
    #[cfg(all(feature = "multithreaded", not(target_arch = "wasm32")))]
    assert_eq!(
        sync_point_case::<crate::system::executor::multi_thread::MultiThreadedExecutor>(),
        [0, 1, 0]
    );
}

fn write_once(mut write: ResMut<WriteNow>, mut level: ResMut<Level>, values: Query<&mut Value>) {
    if write.0 {
        write.0 = false;
        level.0 += 1;
        for mut value in values.iter() {
            value.0 += 1;
        }
    }
}

fn read_changes(
    level: Res<Level>,
    values: Query<Entity, Changed<Value>>,
    mut seen: ResMut<Seen>,
    mut flags: ResMut<Flags>,
) {
    flags.0.push(level.is_changed());
    seen.0.push(values.iter().count());
}

struct Observed {
    resource_changed: Vec<bool>,
    components_changed: Vec<usize>,
}

fn ordering_case<E: SystemExecutor + 'static>(writer_first: bool) -> Observed {
    let mut world = World::new();
    world.spawn(Value(0));
    world.insert_resource(Level::default());
    world.insert_resource(WriteNow(false));
    world.insert_resource(Seen::default());
    world.insert_resource(Flags::default());

    let mut schedule = Schedule::new();
    if writer_first {
        schedule.add_system(write_once);
        schedule.add_system(read_changes.after(write_once));
    } else {
        schedule.add_system(read_changes);
        schedule.add_system(write_once.after(read_changes));
    }
    let mut schedule = schedule.compile::<E>(&mut world);

    schedule.run(&mut world);
    world.tick();
    world.insert_resource(WriteNow(true));
    world.insert_resource(Seen::default());
    world.insert_resource(Flags::default());
    for _ in 0..2 {
        schedule.run(&mut world);
        world.tick();
    }
    Observed {
        resource_changed: world.remove_resource::<Flags>().unwrap().0,
        components_changed: world.remove_resource::<Seen>().unwrap().0,
    }
}

fn assert_each_order_sees_the_write_once<E: SystemExecutor + 'static>() {
    for writer_first in [true, false] {
        let Observed {
            resource_changed: flags,
            components_changed: seen,
        } = ordering_case::<E>(writer_first);
        assert_eq!(
            flags.iter().filter(|changed| **changed).count(),
            1,
            "ResMut, writer first: {writer_first}, {flags:?}"
        );
        assert_eq!(
            seen.iter().sum::<usize>(),
            1,
            "Query<&mut T>, writer first: {writer_first}, {seen:?}"
        );
    }
}

#[test]
fn each_order_of_writer_and_reader_sees_the_write_once() {
    assert_each_order_sees_the_write_once::<SingleThreadedExecutor>();
    #[cfg(all(feature = "multithreaded", not(target_arch = "wasm32")))]
    assert_each_order_sees_the_write_once::<
        crate::system::executor::multi_thread::MultiThreadedExecutor,
    >();
}

fn bump_on_change(mut level: ResMut<Level>, mut flags: ResMut<Flags>) {
    let changed = level.is_changed();
    flags.0.push(changed);
    if changed {
        level.0 += 1;
    }
}

#[test]
fn a_system_does_not_see_its_own_write_next_run() {
    let mut world = World::new();
    world.insert_resource(Level::default());
    world.insert_resource(Flags::default());
    let mut system = bump_on_change.into_system();
    system.initialize(&mut world);

    for _ in 0..3 {
        system.run_and_apply((), &mut world);
    }

    assert_eq!(
        world.get_resource::<Flags>().unwrap().0,
        [true, false, false]
    );
    assert_eq!(world.get_resource::<Level>().unwrap().0, 1);
}

fn record_first_look(
    added: Query<Entity, Added<Value>>,
    changed: Query<Entity, Changed<Value>>,
    level: Res<Level>,
    mut seen: ResMut<Seen>,
    mut flags: ResMut<Flags>,
) {
    seen.0.push(added.iter().count());
    seen.0.push(changed.iter().count());
    flags.0.push(level.is_changed());
}

#[test]
fn a_first_run_sees_everything_as_added() {
    let mut world = World::new();
    world.spawn(Value(0));
    world.spawn(Value(1));
    world.insert_resource(Level::default());
    world.insert_resource(Seen::default());
    world.insert_resource(Flags::default());
    for _ in 0..3 {
        world.tick();
    }

    let mut system = record_first_look.into_system();
    system.initialize(&mut world);
    system.run_and_apply((), &mut world);
    system.run_and_apply((), &mut world);

    assert_eq!(world.get_resource::<Seen>().unwrap().0, [2, 2, 0, 0]);
    assert_eq!(world.get_resource::<Flags>().unwrap().0, [true, false]);
}

fn gated_reader(_: Gate, values: Query<Entity, Changed<Value>>, mut seen: ResMut<Seen>) {
    seen.0.push(values.iter().count());
}

fn bump_values(values: Query<&mut Value>) {
    for mut value in values.iter() {
        value.0 += 1;
    }
}

#[test]
fn a_skipped_run_keeps_the_change() {
    let mut world = World::new();
    world.spawn(Value(0));
    world.insert_resource(Seen::default());
    world.insert_resource(GateOpen(true));
    let mut reader = gated_reader.into_system();
    reader.initialize(&mut world);
    let mut writer = bump_values.into_system();
    writer.initialize(&mut world);

    reader.run_and_apply((), &mut world);
    world.tick();
    world.insert_resource(GateOpen(false));
    writer.run_and_apply((), &mut world);
    reader.run_and_apply((), &mut world);
    world.tick();
    world.insert_resource(GateOpen(true));
    reader.run_and_apply((), &mut world);
    reader.run_and_apply((), &mut world);

    assert_eq!(world.get_resource::<Seen>().unwrap().0, [1, 1, 0]);
}

fn look_at_values(_: On<Look>, values: Query<Entity, Changed<Value>>, mut seen: ResMut<Seen>) {
    seen.0.push(values.iter().count());
}

#[test]
fn a_listener_sees_changes_since_its_own_previous_run() {
    let mut world = World::new();
    let entity = world.spawn(Value(0));
    world.insert_resource(Seen::default());
    world.add_listener(look_at_values);

    world.trigger(Look);
    world.trigger(Look);
    world
        .get_component_for_entity_mut::<Value>(entity)
        .unwrap()
        .0 = 1;
    world.tick();
    world.tick();
    world.trigger(Look);
    world.trigger(Look);

    assert_eq!(world.get_resource::<Seen>().unwrap().0, [1, 0, 1, 0]);
}

#[test]
fn world_query_sees_changes_since_the_last_world_tick() {
    let mut world = World::new();
    let entity = world.spawn(Value(0));
    let mut changed = world.query::<Entity, Changed<Value>>();
    assert_eq!(changed.iter(&mut world).count(), 1);

    world.tick();
    assert_eq!(changed.iter(&mut world).count(), 0);

    world
        .get_component_for_entity_mut::<Value>(entity)
        .unwrap()
        .0 = 1;
    assert_eq!(changed.iter(&mut world).count(), 1);
    assert_eq!(changed.iter(&mut world).count(), 1);

    world.tick();
    assert_eq!(changed.iter(&mut world).count(), 0);

    let mut writer = bump_values.into_system();
    writer.initialize(&mut world);
    writer.run_and_apply((), &mut world);
    assert_eq!(changed.iter(&mut world).count(), 1);

    world.tick();
    assert_eq!(changed.iter(&mut world).count(), 0);
}

#[derive(Resource, Default)]
struct Idle(Vec<usize>);

#[derive(Resource, Default)]
struct Checks(Vec<Tick>);

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct Frame;
impl ScheduleLabel for Frame {
    fn dyn_clone(&self) -> Box<dyn ScheduleLabel> {
        Box::new(self.clone())
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct Rarely;
impl ScheduleLabel for Rarely {
    fn dyn_clone(&self) -> Box<dyn ScheduleLabel> {
        Box::new(self.clone())
    }
}

fn count_changed_values(values: Query<Entity, Changed<Value>>, mut seen: ResMut<Seen>) {
    seen.0.push(values.iter().count());
}

fn idle_reader(values: Query<Entity, Changed<Value>>, mut idle: ResMut<Idle>) {
    idle.0.push(values.iter().count());
}

fn level_reader(level: Res<Level>, mut flags: ResMut<Flags>) {
    flags.0.push(level.is_changed());
}

fn record_check(check: On<CheckChangeTicks>, mut checks: ResMut<Checks>) {
    checks.0.push(check.signal().present_tick());
}

fn frame(world: &mut World) {
    world.run_schedule(Frame);
    world.tick();
}

fn rare_look(world: &mut World) -> Vec<usize> {
    world.run_schedule(Rarely);
    std::mem::take(&mut world.get_resource_mut::<Idle>().unwrap().0)
}

fn take_flags(world: &mut World) -> Vec<bool> {
    std::mem::take(&mut world.get_resource_mut::<Flags>().unwrap().0)
}

fn take_seen(world: &mut World) -> Vec<usize> {
    std::mem::take(&mut world.get_resource_mut::<Seen>().unwrap().0)
}

fn value_changed_tick(world: &World, entity: Entity) -> Tick {
    let location = world.entity_store().find_location(entity).unwrap();
    world
        .get_component_for_entity_location_ticked::<Value>(location)
        .unwrap()
        .changed_tick
}

#[test]
fn check_change_ticks_runs_from_a_schedule_run() {
    let mut world = World::new();
    let entity = world.spawn(Value(0));
    world.set_change_tick(Tick::new(*world.change_tick() + MAX_CHANGE_AGE + 10));
    world.insert_resource(Seen::default());
    let present = world.change_tick();
    let mut schedule = Schedule::new();
    schedule.add_system(count_changed_values);

    schedule
        .compile::<SingleThreadedExecutor>(&mut world)
        .run(&mut world);

    let stamp = value_changed_tick(&world, entity);
    assert_eq!(*present.relative_to(stamp), MAX_CHANGE_AGE);
}

#[test]
fn a_schedule_run_clamps_its_own_systems() {
    let mut world = World::new();
    let entity = world.spawn(Value(0));
    world.insert_resource(Seen::default());
    world.insert_resource(GateOpen(true));
    let mut schedule = Schedule::new();
    schedule.add_system(gated_reader);
    let mut compiled = schedule.compile::<SingleThreadedExecutor>(&mut world);

    compiled.run(&mut world);
    world.insert_resource(GateOpen(false));
    world.set_change_tick(Tick::new(u32::MAX - 5));
    compiled.run(&mut world);
    world
        .get_component_for_entity_mut::<Value>(entity)
        .unwrap()
        .0 = 1;
    world.set_change_tick(Tick::new(20));
    world.insert_resource(GateOpen(true));
    compiled.run(&mut world);

    assert_eq!(take_seen(&mut world), [1, 1]);
}

#[test]
fn ticks_survive_wrap_around() {
    let mut world = World::new();
    let first = world.spawn(Value(0));
    world.insert_resource(Seen::default());
    world.insert_resource(Idle::default());
    world.insert_resource(Level::default());
    world.insert_resource(Flags::default());
    let mut schedules = Schedules::default();
    schedules.add_system(Frame, count_changed_values);
    schedules.add_system(Frame, level_reader);
    schedules.add_system(Rarely, idle_reader);
    let compiled = schedules.compile::<SingleThreadedExecutor>(&mut world);
    world.insert_resource(compiled);

    frame(&mut world);
    frame(&mut world);
    assert_eq!(take_seen(&mut world), [1, 0]);
    assert_eq!(take_flags(&mut world), [true, false]);
    assert_eq!(rare_look(&mut world), [1]);

    world.set_change_tick(Tick::new(u32::MAX - 5));
    frame(&mut world);
    world
        .get_component_for_entity_mut::<Value>(first)
        .unwrap()
        .0 = 1;
    for _ in 0..8 {
        frame(&mut world);
    }
    assert!(*world.change_tick() < 100);
    let mut once = vec![0; 9];
    once[1] = 1;
    assert_eq!(take_seen(&mut world), once);
    assert_eq!(take_flags(&mut world), [false; 9]);
    assert_eq!(rare_look(&mut world), [1]);
    assert_eq!(rare_look(&mut world), [0]);

    world
        .get_component_for_entity_mut::<Value>(first)
        .unwrap()
        .0 = 2;
    world.get_resource_mut::<Level>().unwrap().0 = 1;
    frame(&mut world);
    frame(&mut world);
    assert_eq!(take_seen(&mut world), [1, 0]);
    assert_eq!(take_flags(&mut world), [true, false]);

    world.spawn(Value(1));
    let mut fresh = count_changed_values.into_system();
    fresh.initialize(&mut world);
    fresh.run_and_apply((), &mut world);
    assert_eq!(take_seen(&mut world), [2]);
}

#[test]
fn foreign_ticks_commit_only_when_asked_and_clamp_an_old_last_run() {
    let world = World::new();
    let mut ticks = ForeignTicks::default();
    let first = ticks.begin(&world);
    assert_eq!(*first.this_run.relative_to(first.last_run), MAX_CHANGE_AGE);

    let uncommitted = ticks.begin(&world);
    assert_eq!(
        *uncommitted.this_run.relative_to(uncommitted.last_run),
        MAX_CHANGE_AGE
    );
    ticks.commit();
    assert_eq!(ticks.begin(&world).last_run, uncommitted.this_run);
    ticks.commit();

    let mut world = world;
    world.set_change_tick(Tick::new(*world.change_tick() + MAX_CHANGE_AGE + 10));
    let late = ticks.begin(&world);
    assert_eq!(*late.this_run.relative_to(late.last_run), MAX_CHANGE_AGE);
}

#[test]
fn a_first_run_does_not_see_a_stamp_clamped_to_max_change_age() {
    let mut world = World::new();
    world.spawn(Value(0));
    world.insert_resource(Level::default());
    world.insert_resource(Seen::default());
    world.insert_resource(Flags::default());
    world.set_change_tick(Tick::new(*world.change_tick() + MAX_CHANGE_AGE + 10));
    assert!(world.check_change_ticks().is_some());

    let mut first = record_first_look.into_system();
    first.initialize(&mut world);
    first.run_and_apply((), &mut world);
    assert_eq!(take_seen(&mut world), [0, 0]);
    assert_eq!(take_flags(&mut world), [false]);

    world.spawn(Value(1));
    let mut fresh = record_first_look.into_system();
    fresh.initialize(&mut world);
    fresh.run_and_apply((), &mut world);
    assert_eq!(take_seen(&mut world), [1, 1]);
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Stamp {
    added: bool,
    changed: bool,
}

const NEW: Stamp = Stamp {
    added: true,
    changed: true,
};
const SAME: Stamp = Stamp {
    added: false,
    changed: false,
};
const CHANGED: Stamp = Stamp {
    added: false,
    changed: true,
};

#[derive(Resource, Default)]
struct StampLog(Vec<Stamp>);

fn log_ref(values: Query<Ref<Value>>, mut log: ResMut<StampLog>) {
    for value in values.iter() {
        log.0.push(Stamp {
            added: value.is_added(),
            changed: value.is_changed(),
        });
    }
}

fn log_mut(values: Query<&mut Value>, mut log: ResMut<StampLog>) {
    for value in values.iter() {
        log.0.push(Stamp {
            added: value.is_added(),
            changed: value.is_changed(),
        });
    }
}

fn log_res(level: Res<Level>, mut log: ResMut<StampLog>) {
    log.0.push(Stamp {
        added: level.is_added(),
        changed: level.is_changed(),
    });
}

fn log_res_mut(level: ResMut<Level>, mut log: ResMut<StampLog>) {
    log.0.push(Stamp {
        added: level.is_added(),
        changed: level.is_changed(),
    });
}

#[derive(Clone, Copy, Debug)]
enum Access {
    Ref,
    Mut,
    Res,
    ResMut,
}

impl Access {
    fn reads_a_component(self) -> bool {
        matches!(self, Access::Ref | Access::Mut)
    }

    fn reader(self) -> crate::system::BoxedSystem {
        match self {
            Access::Ref => log_ref.into_boxed_system(),
            Access::Mut => log_mut.into_boxed_system(),
            Access::Res => log_res.into_boxed_system(),
            Access::ResMut => log_res_mut.into_boxed_system(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Step {
    Look,
    Write,
    Overwrite,
    OverwriteInBundle,
    Reinsert,
}

struct StampCase {
    access: Access,
    steps: &'static [Step],
    seen: &'static [Stamp],
}

fn write(world: &mut World, entity: Entity, access: Access, step: Step, value: u32) {
    match (step, access.reads_a_component()) {
        (Step::Write, true) => {
            world
                .get_component_for_entity_mut::<Value>(entity)
                .unwrap()
                .0 = value
        }
        (Step::Write, false) => world.get_resource_mut::<Level>().unwrap().0 = value,
        (Step::Overwrite, true) => world.insert(Value(value), entity),
        (Step::Overwrite, false) => world.insert_resource(Level(value)),
        (Step::OverwriteInBundle, true) => world.insert((Value(value), Marker), entity),
        (Step::Reinsert, false) => {
            world.remove_resource::<Level>();
            world.insert_resource(Level(value));
        }
        (step, _) => unreachable!("{step:?} does not apply to {access:?}"),
    }
}

#[test]
fn added_and_changed_follow_each_write() {
    use Step::*;
    let cases = [
        StampCase {
            access: Access::Ref,
            steps: &[Look, Look, Write, Look, Look],
            seen: &[NEW, SAME, CHANGED, SAME],
        },
        StampCase {
            access: Access::Mut,
            steps: &[Look, Look, Write, Look],
            seen: &[NEW, SAME, CHANGED],
        },
        StampCase {
            access: Access::Res,
            steps: &[Look, Look, Write, Look, Look],
            seen: &[NEW, SAME, CHANGED, SAME],
        },
        StampCase {
            access: Access::ResMut,
            steps: &[Look, Look, Write, Look],
            seen: &[NEW, SAME, CHANGED],
        },
        StampCase {
            access: Access::Ref,
            steps: &[Look, Look, Overwrite, Look, OverwriteInBundle, Look],
            seen: &[NEW, SAME, CHANGED, CHANGED],
        },
        StampCase {
            access: Access::Res,
            steps: &[Look, Look, Overwrite, Look, Look, Reinsert, Look],
            seen: &[NEW, SAME, CHANGED, SAME, NEW],
        },
    ];
    for case in cases {
        let mut world = World::new();
        let entity = world.spawn(Value(0));
        world.insert_resource(Level(0));
        world.insert_resource(StampLog::default());
        let mut reader = case.access.reader();
        reader.initialize(&mut world);
        let mut writes = 0;
        for step in case.steps {
            if let Look = step {
                reader.run_and_apply((), &mut world);
            } else {
                writes += 1;
                write(&mut world, entity, case.access, *step, writes);
            }
        }

        let label = format!("{:?} over {:?}", case.access, case.steps);
        assert_eq!(
            world.get_resource::<StampLog>().unwrap().0,
            case.seen,
            "{label}"
        );
        let value = if case.access.reads_a_component() {
            world.get_component_for_entity::<Value>(entity).unwrap().0
        } else {
            world.get_resource::<Level>().unwrap().0
        };
        assert_eq!(value, writes, "{label}");
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SetIfNeq {
    wrote: bool,
    changed: bool,
}

#[derive(Resource, Default)]
struct Wrote(Vec<SetIfNeq>);

fn set_both(values: Query<&mut Value>, mut level: ResMut<Level>, mut wrote: ResMut<Wrote>) {
    for mut value in values.iter() {
        let written = value.set_if_neq(Value(5));
        wrote.0.push(SetIfNeq {
            wrote: written,
            changed: value.is_changed(),
        });
    }
    let written = level.set_if_neq(Level(3));
    wrote.0.push(SetIfNeq {
        wrote: written,
        changed: level.is_changed(),
    });
}

#[test]
fn set_if_neq_writes_only_a_different_value() {
    let mut world = World::new();
    world.spawn(Value(0));
    world.insert_resource(Level(0));
    world.insert_resource(Wrote::default());
    let mut system = set_both.into_system();
    system.initialize(&mut world);

    system.run_and_apply((), &mut world);
    let first = world.resource_changed_tick::<Level>().unwrap();
    system.run_and_apply((), &mut world);

    let wrote = SetIfNeq {
        wrote: true,
        changed: true,
    };
    let kept = SetIfNeq {
        wrote: false,
        changed: false,
    };
    assert_eq!(
        world.get_resource::<Wrote>().unwrap().0,
        [wrote, wrote, kept, kept]
    );
    assert_eq!(world.resource_changed_tick::<Level>().unwrap(), first);
    assert_eq!(world.get_resource::<Level>().unwrap().0, 3);
}

#[test]
fn check_change_ticks_runs_once_per_threshold_and_tells_its_listeners() {
    let mut world = World::new();
    world.insert_resource(Checks::default());
    world.add_listener(record_check);

    world.set_change_tick(Tick::new(CHECK_TICK_THRESHOLD - 1));
    assert!(world.check_change_ticks().is_none());

    world.set_change_tick(Tick::new(CHECK_TICK_THRESHOLD));
    let check = world.check_change_ticks().unwrap();
    assert_eq!(check.present_tick(), Tick::new(CHECK_TICK_THRESHOLD));

    world.set_change_tick(Tick::new(2 * CHECK_TICK_THRESHOLD - 1));
    assert!(world.check_change_ticks().is_none());

    assert_eq!(
        world.get_resource::<Checks>().unwrap().0,
        [Tick::new(CHECK_TICK_THRESHOLD)]
    );
}
