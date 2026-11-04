use std::{
    mem,
    sync::{Arc, Mutex},
    thread::{self, ThreadId},
};

use crate::Resource;

use concerto_tasks::{
    compute_pool::ComputeTaskPool,
    task_pool::{ScopedTaskPool, TaskPool},
    thread_executor::ThreadExecutor,
};
use concurrent_queue::ConcurrentQueue;
use fixedbitset::FixedBitSet;

use crate::{
    World,
    system::{
        BoxedSystem,
        access::SystemAccess,
        executor::SystemExecutor,
        meta::SystemMetadata,
        schedule::{CompiledScheduleData, is_sync_point},
    },
    utilities::SyncUnsafeCell,
    world::UnsafeWorldCell,
};

#[derive(Resource, Clone)]
pub struct MainThreadExecutor(pub Arc<ThreadExecutor<'static>>);

impl Default for MainThreadExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl MainThreadExecutor {
    /// Creates a new executor that can be used to run systems on the main thread.
    pub fn new() -> Self {
        MainThreadExecutor(TaskPool::get_thread_executor())
    }
}

/// Runs systems in parallel on the [`ComputeTaskPool`].
pub struct MultiThreadedExecutor {
    state: Mutex<ExecutorState>,
    system_completion: ConcurrentQueue<usize>,
}

struct ExecutorState {
    ready_systems: FixedBitSet,
    ready_systems_copy: FixedBitSet,
    running_systems: FixedBitSet,
    unapplied_systems: FixedBitSet,
    dependency_count: Vec<usize>,
    dependants: Vec<Vec<usize>>,
    local_thread_running: bool,
}

struct Environment<'env> {
    executor: &'env MultiThreadedExecutor,
    systems: &'env [SyncUnsafeCell<BoxedSystem>],
    system_meta: &'env [SystemMetadata],
    system_access: &'env [SystemAccess],
    world_cell: UnsafeWorldCell<'env>,
    scope_thread: ThreadId,
    thread_executor: Option<&'env ThreadExecutor<'static>>,
}

#[derive(Clone, Copy)]
struct Context<'scope, 'env> {
    environment: &'env Environment<'env>,
    scope: &'scope ScopedTaskPool<'scope, 'env, ()>,
}

impl SystemExecutor for MultiThreadedExecutor {
    fn init(compiled_data: &CompiledScheduleData) -> Self
    where
        Self: Sized,
    {
        let sys_count = compiled_data.systems.len();

        Self {
            state: Mutex::new(ExecutorState {
                ready_systems: FixedBitSet::with_capacity(sys_count),
                ready_systems_copy: FixedBitSet::with_capacity(sys_count),
                running_systems: FixedBitSet::with_capacity(sys_count),
                unapplied_systems: FixedBitSet::with_capacity(sys_count),
                dependency_count: compiled_data.dependency_count.clone(),
                dependants: compiled_data.dependants.clone(),
                local_thread_running: false,
            }),
            system_completion: ConcurrentQueue::bounded(sys_count.max(1)),
        }
    }

    fn run(&mut self, compiled_data: &mut CompiledScheduleData, world: &mut World) {
        if compiled_data.systems.is_empty() {
            return;
        }

        debug_assert_eq!(
            compiled_data.system_access.len(),
            compiled_data.systems.len()
        );

        self.state
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .reset(compiled_data);

        let thread_executor = world
            .get_resource::<MainThreadExecutor>()
            .map(|e| e.0.clone());
        let thread_executor = thread_executor.as_deref();

        let environment = &Environment {
            executor: self,
            systems: SyncUnsafeCell::from_mut(compiled_data.systems.as_mut_slice())
                .as_slice_of_cells(),
            system_meta: compiled_data.system_meta.as_slice(),
            system_access: compiled_data.system_access.as_slice(),
            world_cell: world.as_unsafe_world_cell_mut(),
            scope_thread: thread::current().id(),
            thread_executor,
        };

        ComputeTaskPool::get_or_init(|| TaskPool::with_name("compute")).scope_with_executor(
            thread_executor,
            |scope| {
                let context = Context { environment, scope };
                context.tick_executor();
            },
        );
    }
}

impl Environment<'_> {
    fn apply_deferred(&self, systems: &FixedBitSet) {
        profiling::scope!("apply_deferred");
        for system_index in systems.ones() {
            let sys = unsafe { &mut *self.systems[system_index].get() };
            // SAFETY: no system runs while a sync point applies.
            sys.apply(unsafe { self.world_cell.world_mut() });
        }
    }
}

impl<'scope, 'env> Context<'scope, 'env> {
    fn tick_executor(self) {
        let executor = self.environment.executor;
        loop {
            let Ok(mut state) = executor.state.try_lock() else {
                return;
            };
            let candidate = state.tick(self);
            drop(state);

            if let Some(system_index) = candidate {
                self.run_system(system_index);
                executor
                    .system_completion
                    .push(system_index)
                    .expect("Error registering finished system");
                continue;
            }

            if executor.system_completion.is_empty() {
                return;
            }
        }
    }

    fn run_system(self, system_index: usize) {
        let sys = unsafe { &mut *self.environment.systems[system_index].get() };
        profiling::scope!(sys.name());
        // SAFETY: the executor starts a system only once no running system conflicts with its access.
        unsafe { sys.run_unsafe((), self.environment.world_cell) };
    }

    fn may_run_here(self, system_index: usize) -> bool {
        let environment = self.environment;
        let on_scope_thread = thread::current().id() == environment.scope_thread;
        if !environment.system_meta[system_index].is_send() {
            environment
                .thread_executor
                .map_or(on_scope_thread, ThreadExecutor::is_current_thread)
        } else if environment.system_access[system_index].is_exclusive() {
            on_scope_thread
        } else {
            true
        }
    }

    fn system_completed(self, system_index: usize) {
        self.environment
            .executor
            .system_completion
            .push(system_index)
            .expect("Error registering finished system");
        self.tick_executor();
    }
}

impl ExecutorState {
    fn reset(&mut self, compiled_data: &CompiledScheduleData) {
        self.ready_systems.clear();
        self.running_systems.clear();
        self.unapplied_systems.clear();
        self.local_thread_running = false;
        self.dependency_count
            .clone_from(&compiled_data.dependency_count);
        for (index, &count) in self.dependency_count.iter().enumerate() {
            if count == 0 {
                self.ready_systems.insert(index);
            }
        }
    }

    fn tick(&mut self, context: Context) -> Option<usize> {
        let environment = context.environment;

        while let Ok(finished_system) = environment.executor.system_completion.pop() {
            self.finish_system(environment, finished_system);
        }

        self.start_sync_points(context);

        let candidate = self.lone_candidate(context);
        if let Some(system_index) = candidate {
            self.start_system(context.environment, system_index);
        }
        self.spawn_system_tasks(context);
        candidate
    }

    fn start_sync_points(&mut self, context: Context) {
        let environment = context.environment;
        while let Some(sync_index) = self
            .ready_systems
            .ones()
            .find(|&index| is_sync_point(unsafe { &*environment.systems[index].get() }))
        {
            self.start_system(environment, sync_index);
            if thread::current().id() == environment.scope_thread {
                environment.apply_deferred(&self.unapplied_systems);
                self.unapplied_systems.clear();
                self.finish_system(environment, sync_index);
                continue;
            }

            let unapplied = self.unapplied_systems.clone();
            self.unapplied_systems.clear();
            context.scope.spawn_on_scope(async move {
                environment.apply_deferred(&unapplied);
                context.system_completed(sync_index);
            });
            return;
        }
    }

    fn lone_candidate(&self, context: Context) -> Option<usize> {
        if !self.running_systems.is_clear() || self.ready_systems.count_ones(..) != 1 {
            return None;
        }
        let system_index = self.ready_systems.ones().next()?;
        context.may_run_here(system_index).then_some(system_index)
    }

    fn start_system(&mut self, environment: &Environment, system_index: usize) {
        if !environment.system_meta[system_index].is_send() {
            self.local_thread_running = true;
        }
        self.ready_systems.remove(system_index);
        self.running_systems.insert(system_index);
    }

    fn finish_system(&mut self, environment: &Environment, system_index: usize) {
        self.running_systems.remove(system_index);
        self.unapplied_systems.insert(system_index);
        if !environment.system_meta[system_index].is_send() {
            self.local_thread_running = false;
        }

        for &dependant in &self.dependants[system_index] {
            self.dependency_count[dependant] -= 1;
            if self.dependency_count[dependant] == 0 {
                self.ready_systems.insert(dependant);
            }
        }
    }

    fn spawn_system_tasks(&mut self, context: Context) {
        let mut ready_systems = mem::take(&mut self.ready_systems_copy);
        ready_systems.clone_from(&self.ready_systems);

        for system_index in ready_systems.ones() {
            let is_send = context.environment.system_meta[system_index].is_send();
            let is_exclusive = context.environment.system_access[system_index].is_exclusive();
            if !is_send && self.local_thread_running {
                continue;
            }

            self.start_system(context.environment, system_index);

            let task = async move {
                context.run_system(system_index);
                context.system_completed(system_index);
            };

            if !is_send {
                context.scope.spawn_on_external(task);
            } else if is_exclusive {
                context.scope.spawn_on_scope(task);
            } else {
                context.scope.spawn(task);
            }
        }

        self.ready_systems_copy = ready_systems;
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
            mpsc::{Receiver, Sender, channel},
        },
        thread::ThreadId,
        time::Duration,
    };

    use crate::{
        Changed, Resource, System, World,
        command::CommandQueue,
        component::Component,
        entity::Entity,
        query::Query,
        resource::ResMut,
        system::{
            NonSendMarker,
            access::SystemAccess,
            executor::multi_thread::{MainThreadExecutor, MultiThreadedExecutor},
            meta::SystemMetadata,
            schedule::{Schedule, ScheduleLabel, Schedules},
        },
        world::UnsafeWorldCell,
    };

    #[derive(Component)]
    struct TagComponent;

    #[derive(Resource)]
    struct Counter(usize);

    fn spawn_tag_entity(mut cmd: CommandQueue) {
        cmd.spawn(TagComponent);
    }

    fn check_changed_entities(
        query: Query<Entity, Changed<TagComponent>>,
        mut counter: ResMut<Counter>,
    ) {
        if query.iter().count() != 0 {
            counter.0 += 1;
        }
    }

    #[test]
    fn check_changed_after_spawned() {
        let mut world = World::new();
        world.insert_resource(Counter(0));

        let mut schedule = Schedule::new();
        schedule.add_system(spawn_tag_entity);
        schedule.add_system(check_changed_entities);

        println!("{:?}", schedule);

        schedule
            .compile::<MultiThreadedExecutor>(&mut world)
            .run(&mut world);

        assert_eq!(world.get_resource::<Counter>().unwrap().0, 1);
    }

    struct NonSendProbe {
        running: Arc<AtomicBool>,
        overlapped: Arc<AtomicBool>,
        ran_on: Arc<Mutex<Vec<ThreadId>>>,
    }

    impl System for NonSendProbe {
        type In = ();

        fn name(&self) -> &'static str {
            "NonSendProbe"
        }

        fn initialize(&mut self, _world: &mut World) {}

        fn fill_access(&self, meta: &mut SystemMetadata, _access: &mut SystemAccess) {
            meta.set_non_send();
        }

        unsafe fn run_unsafe(&mut self, _args: Self::In, _world: UnsafeWorldCell) {
            if self.running.swap(true, Ordering::SeqCst) {
                self.overlapped.store(true, Ordering::SeqCst);
            }
            self.ran_on
                .lock()
                .unwrap()
                .push(std::thread::current().id());
            std::thread::sleep(Duration::from_millis(5));
            self.running.store(false, Ordering::SeqCst);
        }

        fn apply(&mut self, _world: &mut World) {}

        fn check_change_tick(&mut self, _check: crate::component::CheckChangeTicks) {}
    }

    #[test]
    fn non_send_systems_run_one_at_a_time_on_calling_thread() {
        let mut world = World::new();
        world.insert_resource(MainThreadExecutor::new());

        let running = Arc::new(AtomicBool::new(false));
        let overlapped = Arc::new(AtomicBool::new(false));
        let ran_on = Arc::new(Mutex::new(Vec::new()));

        let mut schedule = Schedule::new();
        for _ in 0..4 {
            schedule.add_system(NonSendProbe {
                running: running.clone(),
                overlapped: overlapped.clone(),
                ran_on: ran_on.clone(),
            });
        }

        schedule
            .compile::<MultiThreadedExecutor>(&mut world)
            .run(&mut world);

        let ran_on = ran_on.lock().unwrap();
        assert_eq!(ran_on.len(), 4);
        assert!(ran_on.iter().all(|&id| id == std::thread::current().id()));
        assert!(!overlapped.load(Ordering::SeqCst));
    }

    #[derive(Clone, PartialEq, Eq, Hash, Debug)]
    struct Outer;
    impl ScheduleLabel for Outer {
        fn dyn_clone(&self) -> Box<dyn ScheduleLabel> {
            Box::new(self.clone())
        }
    }

    #[derive(Clone, PartialEq, Eq, Hash, Debug)]
    struct Inner;
    impl ScheduleLabel for Inner {
        fn dyn_clone(&self) -> Box<dyn ScheduleLabel> {
            Box::new(self.clone())
        }
    }

    #[derive(Resource, Default)]
    struct RanOn(Option<ThreadId>);

    fn record_thread(_: NonSendMarker, mut ran_on: ResMut<RanOn>) {
        ran_on.0 = Some(std::thread::current().id());
    }

    #[test]
    fn non_send_system_in_a_schedule_run_by_an_exclusive_system_runs_on_calling_thread() {
        let mut world = World::new();
        world.insert_resource(MainThreadExecutor::new());
        world.insert_resource(RanOn::default());

        let mut schedules = Schedules::default();
        schedules.add_system(Inner, record_thread);
        schedules.add_system(Outer, |world: &mut World| world.run_schedule(Inner));
        let compiled = schedules.compile::<MultiThreadedExecutor>(&mut world);
        world.insert_resource(compiled);

        world.run_schedule(Outer);

        assert_eq!(
            world.get_resource::<RanOn>().unwrap().0,
            Some(std::thread::current().id())
        );
    }

    #[derive(Resource, Default)]
    struct ExclusiveRanOn(Option<ThreadId>);

    #[test]
    fn an_exclusive_system_runs_on_the_thread_that_runs_its_schedule() {
        let mut world = World::new();
        world.insert_resource(MainThreadExecutor::new());
        world.insert_resource(ExclusiveRanOn::default());

        let mut schedules = Schedules::default();
        schedules.add_system(Outer, |world: &mut World| {
            world.get_resource_mut::<ExclusiveRanOn>().unwrap().0 =
                Some(std::thread::current().id());
        });
        let compiled = schedules.compile::<MultiThreadedExecutor>(&mut world);
        world.insert_resource(compiled);

        world.run_schedule(Outer);

        assert_eq!(
            world.get_resource::<ExclusiveRanOn>().unwrap().0,
            Some(std::thread::current().id())
        );
    }

    use crate::system::input::test_support::{Gate, GateOpen};

    #[derive(Resource, Default)]
    struct Ran {
        gated: usize,
        neighbour: usize,
    }

    fn gated(_: Gate, mut ran: ResMut<Ran>) {
        ran.gated += 1;
    }

    fn neighbour(mut ran: ResMut<Ran>) {
        ran.neighbour += 1;
    }

    #[test]
    fn a_declined_system_is_skipped_and_its_neighbours_run() {
        let mut world = World::new();
        world.insert_resource(MainThreadExecutor::new());
        world.insert_resource(Ran::default());
        world.insert_resource(GateOpen(false));

        let mut schedules = Schedules::default();
        schedules.add_system(Outer, gated);
        schedules.add_system(Outer, neighbour);
        let compiled = schedules.compile::<MultiThreadedExecutor>(&mut world);
        world.insert_resource(compiled);

        world.run_schedule(Outer);
        let ran = world.get_resource::<Ran>().unwrap();
        assert_eq!((ran.gated, ran.neighbour), (0, 1));

        world.insert_resource(GateOpen(true));
        world.run_schedule(Outer);
        let ran = world.get_resource::<Ran>().unwrap();
        assert_eq!((ran.gated, ran.neighbour), (1, 2));
    }

    #[derive(Resource, Default)]
    struct Finished {
        slow: AtomicBool,
        dependant: AtomicBool,
    }

    fn breaks(_: crate::resource::Res<Finished>) {
        panic!("the system broke");
    }

    fn after_breaks(finished: crate::resource::Res<Finished>) {
        finished.dependant.store(true, Ordering::SeqCst);
    }

    fn slow(finished: crate::resource::Res<Finished>) {
        std::thread::sleep(Duration::from_millis(100));
        finished.slow.store(true, Ordering::SeqCst);
    }

    #[test]
    fn a_panicking_system_is_raised_after_its_running_neighbours_finish_and_its_dependants_never_run()
     {
        use crate::IntoSystemConfig;
        for _ in 0..5 {
            let mut world = World::new();
            world.insert_resource(MainThreadExecutor::new());
            world.insert_resource(Finished::default());
            let mut schedule = Schedule::new();
            schedule
                .add_system(breaks)
                .add_system(after_breaks.after(breaks))
                .add_system(slow);
            let mut compiled = schedule.compile::<MultiThreadedExecutor>(&mut world);

            let outcome =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| compiled.run(&mut world)));

            let payload = outcome.expect_err("the panic is raised again");
            assert_eq!(payload.downcast_ref::<&str>(), Some(&"the system broke"));
            let finished = world.get_resource::<Finished>().unwrap();
            assert!(finished.slow.load(Ordering::SeqCst));
            assert!(!finished.dependant.load(Ordering::SeqCst));
        }
    }

    #[derive(Resource, Default)]
    struct ChainThreads(Vec<ThreadId>);

    fn chain_first(mut threads: ResMut<ChainThreads>) {
        threads.0.push(std::thread::current().id());
    }

    fn chain_second(mut threads: ResMut<ChainThreads>) {
        threads.0.push(std::thread::current().id());
    }

    fn chain_third(mut threads: ResMut<ChainThreads>) {
        threads.0.push(std::thread::current().id());
    }

    #[test]
    fn a_chain_of_send_systems_runs_on_the_calling_thread() {
        use crate::IntoSystemConfig;
        let mut world = World::new();
        world.insert_resource(ChainThreads::default());
        let mut schedule = Schedule::new();
        schedule
            .add_system(chain_first)
            .add_system(chain_second.after(chain_first))
            .add_system(chain_third.after(chain_second));
        schedule
            .compile::<MultiThreadedExecutor>(&mut world)
            .run(&mut world);

        let threads = &world.get_resource::<ChainThreads>().unwrap().0;
        assert_eq!(threads.len(), 3);
        assert!(threads.iter().all(|&id| id == std::thread::current().id()));
    }

    #[test]
    fn a_lone_panicking_system_at_the_start_is_raised_and_its_dependant_never_runs() {
        use crate::IntoSystemConfig;
        let mut world = World::new();
        world.insert_resource(Finished::default());
        let mut schedule = Schedule::new();
        schedule
            .add_system(breaks)
            .add_system(after_breaks.after(breaks));
        let mut compiled = schedule.compile::<MultiThreadedExecutor>(&mut world);

        let outcome =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| compiled.run(&mut world)));

        let payload = outcome.expect_err("the panic is raised again");
        assert_eq!(payload.downcast_ref::<&str>(), Some(&"the system broke"));
        let finished = world.get_resource::<Finished>().unwrap();
        assert!(!finished.dependant.load(Ordering::SeqCst));
    }

    struct Side {
        send: Mutex<Sender<()>>,
        recv: Mutex<Receiver<()>>,
        met: AtomicBool,
    }

    #[derive(Resource)]
    struct Rendezvous {
        a: Side,
        b: Side,
    }

    impl Rendezvous {
        fn new() -> Self {
            let (to_b, from_a) = channel();
            let (to_a, from_b) = channel();
            Self {
                a: Side {
                    send: Mutex::new(to_b),
                    recv: Mutex::new(from_b),
                    met: AtomicBool::new(false),
                },
                b: Side {
                    send: Mutex::new(to_a),
                    recv: Mutex::new(from_a),
                    met: AtomicBool::new(false),
                },
            }
        }
    }

    fn meet(side: &Side) {
        let _ = side.send.lock().unwrap().send(());
        let met = side
            .recv
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(5))
            .is_ok();
        side.met.store(met, Ordering::SeqCst);
    }

    fn meet_a(rendezvous: crate::resource::Res<Rendezvous>) {
        meet(&rendezvous.a);
    }

    fn meet_b(rendezvous: crate::resource::Res<Rendezvous>) {
        meet(&rendezvous.b);
    }

    fn fewer_than_two_threads() -> bool {
        let fewer = !std::thread::available_parallelism().is_ok_and(|n| n.get() >= 2);
        if fewer {
            eprintln!("SKIPPED: fewer than two compute threads are available");
        }
        fewer
    }

    fn assert_pair_met(world: &World) {
        let rendezvous = world.get_resource::<Rendezvous>().unwrap();
        assert!(rendezvous.a.met.load(Ordering::SeqCst));
        assert!(rendezvous.b.met.load(Ordering::SeqCst));
    }

    #[test]
    fn an_exclusive_system_after_a_parallel_pair_runs_on_the_calling_thread() {
        use crate::IntoSystemConfig;
        if fewer_than_two_threads() {
            return;
        }
        let mut world = World::new();
        world.insert_resource(Rendezvous::new());
        world.insert_resource(ExclusiveRanOn::default());
        let mut schedule = Schedule::new();
        schedule.add_system(meet_a).add_system(meet_b).add_system(
            (|world: &mut World| {
                world.get_resource_mut::<ExclusiveRanOn>().unwrap().0 =
                    Some(std::thread::current().id());
            })
            .after(meet_a)
            .after(meet_b),
        );
        schedule
            .compile::<MultiThreadedExecutor>(&mut world)
            .run(&mut world);

        assert_pair_met(&world);
        assert_eq!(
            world.get_resource::<ExclusiveRanOn>().unwrap().0,
            Some(std::thread::current().id())
        );
    }

    #[test]
    fn a_non_send_system_after_a_parallel_pair_runs_on_the_calling_thread() {
        use crate::IntoSystemConfig;
        if fewer_than_two_threads() {
            return;
        }
        let mut world = World::new();
        world.insert_resource(MainThreadExecutor::new());
        world.insert_resource(Rendezvous::new());
        world.insert_resource(RanOn::default());
        let mut schedule = Schedule::new();
        schedule
            .add_system(meet_a)
            .add_system(meet_b)
            .add_system(record_thread.after(meet_a).after(meet_b));
        schedule
            .compile::<MultiThreadedExecutor>(&mut world)
            .run(&mut world);

        assert_pair_met(&world);
        assert_eq!(
            world.get_resource::<RanOn>().unwrap().0,
            Some(std::thread::current().id())
        );
    }

    #[test]
    fn a_lone_panicking_system_after_a_parallel_pair_is_raised_and_its_dependant_never_runs() {
        use crate::IntoSystemConfig;
        if fewer_than_two_threads() {
            return;
        }
        let mut world = World::new();
        world.insert_resource(Finished::default());
        world.insert_resource(Rendezvous::new());
        let mut schedule = Schedule::new();
        schedule
            .add_system(meet_a)
            .add_system(meet_b)
            .add_system(breaks.after(meet_a).after(meet_b))
            .add_system(after_breaks.after(breaks));
        let mut compiled = schedule.compile::<MultiThreadedExecutor>(&mut world);

        let outcome =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| compiled.run(&mut world)));

        let payload = outcome.expect_err("the panic is raised again");
        assert_eq!(payload.downcast_ref::<&str>(), Some(&"the system broke"));
        assert_pair_met(&world);
        let finished = world.get_resource::<Finished>().unwrap();
        assert!(!finished.dependant.load(Ordering::SeqCst));
    }

    #[test]
    fn two_systems_ready_at_the_start_run_at_the_same_time() {
        if fewer_than_two_threads() {
            return;
        }
        let mut world = World::new();
        world.insert_resource(Rendezvous::new());
        let mut schedule = Schedule::new();
        schedule.add_system(meet_a).add_system(meet_b);
        schedule
            .compile::<MultiThreadedExecutor>(&mut world)
            .run(&mut world);

        assert_pair_met(&world);
    }

    struct Applied;
    impl crate::signal::Signal for Applied {}

    #[derive(Resource, Default)]
    struct AppliedOn(Option<ThreadId>);

    fn record_applied(_: crate::signal::On<Applied>, mut applied_on: ResMut<AppliedOn>) {
        applied_on.0 = Some(std::thread::current().id());
    }

    fn meet_and_trigger(rendezvous: crate::resource::Res<Rendezvous>, mut cmd: CommandQueue) {
        meet(&rendezvous.b);
        cmd.trigger(Applied);
    }

    fn trigger_applied(mut cmd: CommandQueue) {
        cmd.trigger(Applied);
    }

    fn after_trigger(mut counter: ResMut<Counter>) {
        counter.0 += 1;
    }

    #[test]
    fn commands_reached_from_pool_threads_apply_on_the_calling_thread() {
        if fewer_than_two_threads() {
            return;
        }
        for _ in 0..5 {
            let mut world = World::new();
            world.insert_resource(Rendezvous::new());
            world.insert_resource(AppliedOn::default());
            world.add_listener(record_applied);
            let mut schedule = Schedule::new();
            schedule.add_system(meet_a).add_system(meet_and_trigger);
            schedule
                .compile::<MultiThreadedExecutor>(&mut world)
                .run(&mut world);

            assert_pair_met(&world);
            assert_eq!(
                world.get_resource::<AppliedOn>().unwrap().0,
                Some(std::thread::current().id())
            );
        }
    }

    #[test]
    fn commands_of_a_chain_apply_on_the_calling_thread() {
        use crate::IntoSystemConfig;
        let mut world = World::new();
        world.insert_resource(Counter(0));
        world.insert_resource(AppliedOn::default());
        world.add_listener(record_applied);
        let mut schedule = Schedule::new();
        schedule
            .add_system(trigger_applied)
            .add_system(after_trigger.after(trigger_applied));
        schedule
            .compile::<MultiThreadedExecutor>(&mut world)
            .run(&mut world);

        assert_eq!(world.get_resource::<Counter>().unwrap().0, 1);
        assert_eq!(
            world.get_resource::<AppliedOn>().unwrap().0,
            Some(std::thread::current().id())
        );
    }

    #[derive(Resource, Default)]
    struct FirstCount(usize);

    #[derive(Resource, Default)]
    struct SecondCount(usize);

    fn count_first(mut count: ResMut<FirstCount>) {
        count.0 += 1;
    }

    fn count_second(mut count: ResMut<SecondCount>) {
        count.0 += 1;
    }

    #[test]
    fn every_dependant_ready_after_a_sync_point_runs() {
        let mut world = World::new();
        world.insert_resource(FirstCount::default());
        world.insert_resource(SecondCount::default());
        let mut schedule = Schedule::new();
        schedule
            .add_system(trigger_applied)
            .add_system(count_first)
            .add_system(count_second);
        schedule
            .compile::<MultiThreadedExecutor>(&mut world)
            .run(&mut world);

        assert_eq!(world.get_resource::<FirstCount>().unwrap().0, 1);
        assert_eq!(world.get_resource::<SecondCount>().unwrap().0, 1);
    }
}
