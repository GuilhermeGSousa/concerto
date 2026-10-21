use crate::{
    Added, Changed, CommandQueue, Component, Entity, IntoSystem, IntoSystemConfig, Query, Ref, Res,
    ResMut, Resource, Schedule, System, World,
    component::Tick,
    query::change_detection::DetectChanges,
    signal::{EntitySignal, On, Signal, listener::IntoListener},
    system::{
        executor::{SystemExecutor, single_thread::SingleThreadedExecutor},
        input::test_support::{Gate, GateOpen},
        schedule::{ScheduleLabel, Schedules},
    },
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

fn ordering_case<E: SystemExecutor + 'static>(writer_first: bool) -> (Vec<bool>, Vec<usize>) {
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
    (
        world.remove_resource::<Flags>().unwrap().0,
        world.remove_resource::<Seen>().unwrap().0,
    )
}

fn assert_each_order_sees_the_write_once<E: SystemExecutor + 'static>() {
    for writer_first in [true, false] {
        let (flags, seen) = ordering_case::<E>(writer_first);
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

fn record_level_change(level: Res<Level>, mut flags: ResMut<Flags>) {
    flags.0.push(level.is_changed());
}

#[test]
fn get_resource_mut_stamps_a_change() {
    let mut world = World::new();
    world.insert_resource(Level::default());
    world.insert_resource(Flags::default());
    let mut reader = record_level_change.into_system();
    reader.initialize(&mut world);

    reader.run_and_apply((), &mut world);
    reader.run_and_apply((), &mut world);
    world.get_resource_mut::<Level>().unwrap().0 = 1;
    reader.run_and_apply((), &mut world);
    reader.run_and_apply((), &mut world);

    assert_eq!(
        world.get_resource::<Flags>().unwrap().0,
        [true, false, true, false]
    );
}

#[derive(Resource, Default)]
struct Looked(Vec<usize>);

#[derive(Resource, Default)]
struct Idle(Vec<usize>);

struct Peek;
impl Signal for Peek {}

struct Prod;
impl Signal for Prod {}
impl EntitySignal for Prod {}

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

fn look_into_looked(
    _: On<Look>,
    values: Query<Entity, Changed<Value>>,
    mut looked: ResMut<Looked>,
) {
    looked.0.push(values.iter().count());
}

fn idle_reader(values: Query<Entity, Changed<Value>>, mut idle: ResMut<Idle>) {
    idle.0.push(values.iter().count());
}

fn idle_listener(_: On<Peek>, values: Query<Entity, Changed<Value>>, mut idle: ResMut<Idle>) {
    idle.0.push(values.iter().count());
}

fn idle_entity_listener(
    _: On<Prod>,
    values: Query<Entity, Changed<Value>>,
    mut idle: ResMut<Idle>,
) {
    idle.0.push(values.iter().count());
}

fn level_reader(level: Res<Level>, mut flags: ResMut<Flags>) {
    flags.0.push(level.is_changed());
}

fn frame(world: &mut World) {
    world.run_schedule(Frame);
    world.trigger(Look);
    world.tick();
}

fn rare_look(world: &mut World, holder: Entity) -> Vec<usize> {
    world.run_schedule(Rarely);
    world.trigger(Peek);
    world.trigger_on(holder, Prod);
    std::mem::take(&mut world.get_resource_mut::<Idle>().unwrap().0)
}

fn take_flags(world: &mut World) -> Vec<bool> {
    std::mem::take(&mut world.get_resource_mut::<Flags>().unwrap().0)
}

fn take_counts(world: &mut World) -> (Vec<usize>, Vec<usize>) {
    (
        std::mem::take(&mut world.get_resource_mut::<Seen>().unwrap().0),
        std::mem::take(&mut world.get_resource_mut::<Looked>().unwrap().0),
    )
}

#[test]
fn ticks_survive_wrap_around() {
    let mut world = World::new();
    let first = world.spawn(Value(0));
    world.insert_resource(Seen::default());
    world.insert_resource(Looked::default());
    world.insert_resource(Idle::default());
    world.insert_resource(Level::default());
    world.insert_resource(Flags::default());
    let holder = world.spawn(idle_entity_listener.into_listener());
    let mut schedules = Schedules::default();
    schedules.add_system(Frame, count_changed_values);
    schedules.add_system(Frame, level_reader);
    schedules.add_system(Rarely, idle_reader);
    let compiled = schedules.compile::<SingleThreadedExecutor>(&mut world);
    world.insert_resource(compiled);
    world.add_listener(look_into_looked);
    world.add_listener(idle_listener);

    frame(&mut world);
    frame(&mut world);
    assert_eq!(take_counts(&mut world), (vec![1, 0], vec![1, 0]));
    assert_eq!(take_flags(&mut world), [true, false]);
    assert_eq!(rare_look(&mut world, holder), [1, 1, 1]);

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
    assert_eq!(take_counts(&mut world), (once.clone(), once));
    assert_eq!(take_flags(&mut world), [false; 9]);
    assert_eq!(rare_look(&mut world, holder), [1, 1, 1]);
    assert_eq!(rare_look(&mut world, holder), [0, 0, 0]);

    world
        .get_component_for_entity_mut::<Value>(first)
        .unwrap()
        .0 = 2;
    world.get_resource_mut::<Level>().unwrap().0 = 1;
    frame(&mut world);
    frame(&mut world);
    assert_eq!(take_counts(&mut world), (vec![1, 0], vec![1, 0]));
    assert_eq!(take_flags(&mut world), [true, false]);

    world.spawn(Value(1));
    let mut fresh = count_changed_values.into_system();
    fresh.initialize(&mut world);
    fresh.run_and_apply((), &mut world);
    world.add_listener(look_into_looked);
    world.trigger(Look);
    assert_eq!(take_counts(&mut world), (vec![2], vec![1, 2]));
}

#[derive(Resource, Default)]
struct RefLog(Vec<(bool, bool)>);

fn log_ref(values: Query<Ref<Value>>, mut log: ResMut<RefLog>) {
    for value in values.iter() {
        log.0.push((value.is_added(), value.is_changed()));
    }
}

#[test]
fn ref_reports_added_then_changed() {
    let mut world = World::new();
    let entity = world.spawn(Value(0));
    world.insert_resource(RefLog::default());
    let mut system = log_ref.into_system();
    system.initialize(&mut world);

    system.run_and_apply((), &mut world);
    system.run_and_apply((), &mut world);
    world
        .get_component_for_entity_mut::<Value>(entity)
        .unwrap()
        .0 = 1;
    system.run_and_apply((), &mut world);
    system.run_and_apply((), &mut world);

    assert_eq!(
        world.get_resource::<RefLog>().unwrap().0,
        [(true, true), (false, false), (false, true), (false, false)]
    );
}

fn deref_ref(values: Query<Ref<Value>>, mut seen: ResMut<Seen>) {
    for value in values.iter() {
        let plain: &Value = &value;
        seen.0.push(plain.0 as usize);
    }
}

#[test]
fn ref_derefs_to_the_component() {
    let mut world = World::new();
    world.spawn(Value(7));
    world.insert_resource(Seen::default());
    let mut system = deref_ref.into_system();
    system.initialize(&mut world);
    system.run_and_apply((), &mut world);
    assert_eq!(world.get_resource::<Seen>().unwrap().0, [7]);
}

#[derive(Resource, Default)]
struct Wrote(Vec<bool>);

#[derive(Resource, Default)]
struct Stamps(Vec<bool>);

fn set_component(values: Query<&mut Value>, mut wrote: ResMut<Wrote>, mut stamps: ResMut<Stamps>) {
    for mut value in values.iter() {
        wrote.0.push(value.set_if_neq(Value(5)));
        stamps.0.push(value.is_changed());
    }
}

#[test]
fn set_if_neq_writes_only_a_different_value() {
    let mut world = World::new();
    world.spawn(Value(0));
    world.insert_resource(Wrote::default());
    world.insert_resource(Stamps::default());
    let mut system = set_component.into_system();
    system.initialize(&mut world);
    system.run_and_apply((), &mut world);
    system.run_and_apply((), &mut world);
    assert_eq!(world.get_resource::<Wrote>().unwrap().0, [true, false]);
    assert_eq!(world.get_resource::<Stamps>().unwrap().0, [true, false]);
}

fn set_resource(mut level: ResMut<Level>, mut wrote: ResMut<Wrote>, mut stamps: ResMut<Stamps>) {
    wrote.0.push(level.set_if_neq(Level(3)));
    stamps.0.push(level.is_changed());
}

#[test]
fn res_mut_set_if_neq_writes_only_a_different_value() {
    let mut world = World::new();
    world.insert_resource(Level(0));
    world.insert_resource(Wrote::default());
    world.insert_resource(Stamps::default());
    let mut system = set_resource.into_system();
    system.initialize(&mut world);
    system.run_and_apply((), &mut world);
    let first = world.resource_changed_tick::<Level>().unwrap();
    system.run_and_apply((), &mut world);
    assert_eq!(world.get_resource::<Wrote>().unwrap().0, [true, false]);
    assert_eq!(world.get_resource::<Stamps>().unwrap().0, [true, false]);
    assert_eq!(world.resource_changed_tick::<Level>().unwrap(), first);
    assert_eq!(world.get_resource::<Level>().unwrap().0, 3);
}

fn log_res(level: Res<Level>, mut flags: ResMut<Flags>) {
    flags.0.push(level.is_added());
}

#[test]
fn res_is_added_once() {
    let mut world = World::new();
    world.insert_resource(Level::default());
    world.insert_resource(Flags::default());
    let mut system = log_res.into_system();
    system.initialize(&mut world);
    for _ in 0..3 {
        system.run_and_apply((), &mut world);
    }
    assert_eq!(
        world.get_resource::<Flags>().unwrap().0,
        [true, false, false]
    );
}
