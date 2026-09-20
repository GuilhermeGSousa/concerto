use std::{any::TypeId, collections::HashMap, fmt};

use crate::{
    Resource, System, define_label,
    intern::Interned,
    system::{
        BoxedSystem,
        access::SystemAccess,
        config::{IntoSystemConfig, SystemConfig},
        executor::SystemExecutor,
        graph::{SystemDependencyGraph, SystemNode},
        meta::SystemMetadata,
        sync_point::SyncPoint,
    },
    world::World,
};
use derive_more::{Deref, From};
use petgraph::{
    Direction,
    algo::toposort,
    dot::{Config, Dot},
    graph::NodeIndex,
};

#[derive(Clone, Copy, Eq, Hash, PartialEq, Deref, From)]
pub struct SystemNodeIndex(NodeIndex);

#[derive(Clone, Copy, Eq, Hash, PartialEq, Deref, From)]
pub struct SystemIndex(usize);

/// An ordered list of systems that are executed sequentially each time [`run`](Schedule::run) is called.
///
/// Systems are added with [`add_system`](Schedule::add_system) (appended to the end) or
/// [`add_system_first`](Schedule::add_system_first) (prepended to the front).
///
/// # Example
/// ```
/// use ecs::{Schedule, World, Component, Query};
///
/// #[derive(Component)]
/// struct Velocity(f32);
///
/// fn apply_velocity(query: Query<&Velocity>) {
///     for v in query.iter() { /* ... */ }
/// }
///
/// let mut schedule = Schedule::new();
/// schedule.add_system(apply_velocity);
/// ```
#[derive(Default)]
pub struct Schedule {
    system_ids: Vec<SystemNodeIndex>,
    systems: Vec<BoxedSystem>,
    graph: SystemDependencyGraph,
}

impl Schedule {
    /// Creates an empty schedule.
    pub fn new() -> Schedule {
        Self {
            system_ids: Vec::new(),
            systems: Vec::new(),
            graph: SystemDependencyGraph::new(),
        }
    }

    /// Appends a system (or [`SystemConfig`]) to the schedule.
    ///
    /// Accepts bare system functions as well as configured systems built with
    /// [`.after()`](IntoSystemConfig::after) / [`.before()`](IntoSystemConfig::before):
    ///
    /// ```
    /// # use ecs::{Schedule, IntoSystemConfig};
    /// # fn a() {} fn b() {} fn c() {}
    /// let mut schedule = Schedule::new();
    /// schedule
    ///     .add_system(a)
    ///     .add_system(b.after(a))
    ///     .add_system(c.after(b).before(a));
    /// ```
    pub fn add_system<M>(&mut self, system: impl IntoSystemConfig<M> + 'static) -> &mut Self {
        self.add_config(system.into_config());
        self
    }

    /// Registers a [`SystemConfig`] into the graph, recursively registering owned dep
    /// systems first, then wiring explicit ordering edges.  Returns the [`NodeIndex`] of
    /// the newly registered system (used internally for edge wiring in recursive calls).
    fn add_config(&mut self, config: SystemConfig) -> SystemNodeIndex {
        let after_indices: Vec<SystemNodeIndex> = config
            .after
            .into_iter()
            .map(|dep| self.add_config(dep))
            .collect();

        let before_indices: Vec<SystemNodeIndex> = config
            .before
            .into_iter()
            .map(|dep| self.add_config(dep))
            .collect();

        let name = config.system.name();
        let mut access = SystemAccess::default();
        let mut metadata = SystemMetadata::default();
        config.system.fill_access(&mut metadata, &mut access);

        let node_idx: SystemNodeIndex = self
            .graph
            .add_node(SystemNode::new(
                self.systems.len().into(),
                access.clone(),
                metadata,
                name,
            ))
            .into();

        // Implicit edges from access-pattern conflicts.
        for node_index in &self.system_ids {
            if let Some(other_system) = self.graph.node_weight(**node_index)
                && !SystemAccess::are_disjoint(&access, other_system.access())
            {
                self.graph.add_edge(**node_index, *node_idx, ());
            }
        }

        // Explicit ordering edges.
        for dep_idx in after_indices {
            self.graph.add_edge(*dep_idx, *node_idx, ());
        }
        for dep_idx in before_indices {
            self.graph.add_edge(*node_idx, *dep_idx, ());
        }

        let needs_sync = access.needs_apply() && !is_sync_point(&config.system);

        self.system_ids.push(node_idx);
        self.systems.push(config.system);

        if needs_sync {
            self.add_sync_point();
        }

        node_idx
    }

    pub fn compile<T: SystemExecutor + 'static>(mut self, world: &mut World) -> CompiledSchedule {
        let dependency_count: Vec<usize> = self
            .system_ids
            .iter()
            .map(|idx| {
                self.graph
                    .neighbors_directed(**idx, Direction::Incoming)
                    .count()
            })
            .collect();

        let dependants = self
            .system_ids
            .iter()
            .map(|idx| {
                self.graph
                    .neighbors_directed(**idx, Direction::Outgoing)
                    .map(|node_index| *self.graph.node_weight(node_index).unwrap().index())
                    .collect()
            })
            .collect();

        let system_access: Vec<SystemAccess> = self
            .system_ids
            .iter()
            .map(|idx| self.graph.node_weight(**idx).unwrap().access().clone())
            .collect();

        let system_meta: Vec<SystemMetadata> = self
            .system_ids
            .iter()
            .map(|idx| self.graph.node_weight(**idx).unwrap().meta().clone())
            .collect();

        let sorted_systems = toposort(&self.graph, None)
            .expect("Cycle detected in schedule — check your .after()/.before() constraints")
            .into_iter()
            .map(|node_index| *self.graph.node_weight(node_index).unwrap().index())
            .collect::<Vec<_>>();

        self.systems
            .iter_mut()
            .for_each(|system| system.initialize(world));

        let compiled_data = CompiledScheduleData {
            systems: self.systems,
            sorted_systems,
            dependency_count,
            dependants,
            system_access,
            system_meta,
        };

        CompiledSchedule {
            executor: Box::new(T::init(&compiled_data)),
            compiled_data,
            graph: self.graph,
        }
    }

    fn add_sync_point(&mut self) {
        self.add_system(SyncPoint);
    }
}

impl fmt::Debug for Schedule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:?}",
            Dot::with_config(&self.graph, &[Config::EdgeNoLabel])
        )
    }
}

pub struct CompiledSchedule {
    executor: Box<dyn SystemExecutor>,
    compiled_data: CompiledScheduleData,
    graph: SystemDependencyGraph,
}

// No constructor methods here! Get this by compiling a schedule
impl CompiledSchedule {
    pub fn run(&mut self, world: &mut World) {
        self.executor.run(&mut self.compiled_data, world);
    }
}

impl fmt::Debug for CompiledSchedule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:?}",
            Dot::with_config(&self.graph, &[Config::EdgeNoLabel])
        )
    }
}

pub struct CompiledScheduleData {
    pub systems: Vec<BoxedSystem>,
    pub sorted_systems: Vec<usize>,
    pub dependency_count: Vec<usize>,
    pub dependants: Vec<Vec<usize>>,
    pub system_access: Vec<SystemAccess>,
    pub system_meta: Vec<SystemMetadata>,
}

define_label!(ScheduleLabel);

pub type InternedScheduleLabel = Interned<dyn ScheduleLabel>;

#[derive(Resource, Default, Debug)]
pub struct Schedules {
    schedules: HashMap<InternedScheduleLabel, Schedule>,
}

impl Schedules {
    /// Registers a system in the schedule identified by `update_group`.
    pub fn add_system<M>(
        &mut self,
        update_group: impl ScheduleLabel,
        system: impl IntoSystemConfig<M> + 'static,
    ) {
        self.schedules
            .entry(update_group.intern())
            .or_default()
            .add_system(system);
    }

    pub fn compile<T: SystemExecutor + 'static>(self, world: &mut World) -> CompiledSchedules {
        CompiledSchedules {
            compiled_schedules: self
                .schedules
                .into_iter()
                .map(|(label, schedule)| (label, schedule.compile::<T>(world)))
                .collect(),
        }
    }
}
#[derive(Resource, Debug, Default)]
pub struct CompiledSchedules {
    compiled_schedules: HashMap<InternedScheduleLabel, CompiledSchedule>,
}

impl CompiledSchedules {
    pub fn get(&self, label: impl ScheduleLabel) -> Option<&CompiledSchedule> {
        self.compiled_schedules.get(&label.intern())
    }

    pub fn get_mut(&mut self, label: impl ScheduleLabel) -> Option<&mut CompiledSchedule> {
        self.compiled_schedules.get_mut(&label.intern())
    }

    pub fn remove(&mut self, label: impl ScheduleLabel) -> Option<CompiledSchedule> {
        self.compiled_schedules.remove(&label.intern())
    }

    pub(crate) fn insert(&mut self, label: impl ScheduleLabel, schedule: CompiledSchedule) {
        self.compiled_schedules.insert(label.intern(), schedule);
    }
}

pub(crate) fn is_sync_point(system: &dyn System) -> bool {
    system.system_type() == TypeId::of::<SyncPoint>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::{
        config::IntoSystemConfig, executor::single_thread::SingleThreadedExecutor,
    };
    use petgraph::graph::NodeIndex;

    // ── Existing tests ────────────────────────────────────────────────────────

    #[test]
    fn schedule_new() {
        let schedule = Schedule::new();
        assert_eq!(schedule.graph.node_count(), 0);
        assert_eq!(schedule.system_ids.len(), 0);
    }

    #[test]
    fn add_system_builder_style() {
        let mut schedule = Schedule::new();
        schedule.add_system(|| {}).add_system(|| {});

        assert_eq!(schedule.graph.node_count(), 2);
        assert_eq!(schedule.system_ids.len(), 2);
    }

    #[test]
    fn system_dependency_graph_creation() {
        let mut schedule = Schedule::new();
        schedule.add_system(|| {});

        assert_eq!(schedule.graph.node_count(), 1);
    }

    #[test]
    fn multiple_systems_added() {
        let mut schedule = Schedule::new();
        schedule
            .add_system(|| {})
            .add_system(|| {})
            .add_system(|| {});

        assert_eq!(schedule.graph.node_count(), 3);
        assert_eq!(schedule.system_ids.len(), 3);
        assert_eq!(schedule.graph.node_count(), 3);
    }

    #[test]
    fn compile_and_run() {
        let mut schedule = Schedule::new();
        schedule
            .add_system(|| print!("First"))
            .add_system(|| print!("First"))
            .add_system(|| print!("First"));

        let mut world = World::new();
        schedule
            .compile::<SingleThreadedExecutor>(&mut world)
            .run(&mut world);
    }

    // ── Graph structure tests ─────────────────────────────────────────────────
    //
    // These tests use zero-parameter (data-disjoint) functions so that only
    // explicit ordering edges appear in the graph.

    #[test]
    fn after_registers_dep_and_main() {
        fn dep() {}
        fn main_sys() {}

        let mut schedule = Schedule::new();
        schedule.add_system(main_sys.after(dep));

        // Both systems must be present in the graph.
        assert_eq!(schedule.graph.node_count(), 2);
        assert_eq!(schedule.system_ids.len(), 2);
    }

    #[test]
    fn after_creates_dep_to_main_edge() {
        fn dep() {}
        fn main_sys() {}

        let mut schedule = Schedule::new();
        schedule.add_system(main_sys.after(dep));

        // dep is registered first → NodeIndex(0); main second → NodeIndex(1).
        // The explicit ordering edge must run dep before main: 0 → 1.
        assert_eq!(schedule.graph.edge_count(), 1);
        assert!(
            schedule
                .graph
                .contains_edge(NodeIndex::new(0), NodeIndex::new(1))
        );
    }

    #[test]
    fn before_registers_dep_and_main() {
        fn main_sys() {}
        fn dep() {}

        let mut schedule = Schedule::new();
        schedule.add_system(main_sys.before(dep));

        assert_eq!(schedule.graph.node_count(), 2);
        assert_eq!(schedule.system_ids.len(), 2);
    }

    #[test]
    fn before_creates_main_to_dep_edge() {
        fn main_sys() {}
        fn dep() {}

        let mut schedule = Schedule::new();
        schedule.add_system(main_sys.before(dep));

        // dep is registered first (as a "before" dep) → NodeIndex(0).
        // main is registered second → NodeIndex(1).
        // The explicit ordering edge must run main before dep: 1 → 0.
        assert_eq!(schedule.graph.edge_count(), 1);
        assert!(
            schedule
                .graph
                .contains_edge(NodeIndex::new(1), NodeIndex::new(0))
        );
    }

    #[test]
    fn after_before_chain_has_correct_edges() {
        fn sys_a() {}
        fn sys_b() {} // main: a → b → c
        fn sys_c() {}

        let mut schedule = Schedule::new();
        // Registration order in add_config:
        //   1. after  deps → sys_a : NodeIndex(0)
        //   2. before deps → sys_c : NodeIndex(1)
        //   3. main   sys_b        : NodeIndex(2)
        //   edges: 0→2 (a before b) and 2→1 (b before c)
        schedule.add_system(sys_b.after(sys_a).before(sys_c));

        assert_eq!(schedule.graph.node_count(), 3);
        assert_eq!(schedule.graph.edge_count(), 2);
        assert!(
            schedule
                .graph
                .contains_edge(NodeIndex::new(0), NodeIndex::new(2))
        ); // a → b
        assert!(
            schedule
                .graph
                .contains_edge(NodeIndex::new(2), NodeIndex::new(1))
        ); // b → c
    }

    #[test]
    fn nested_after_chain_has_correct_edges() {
        fn sys_a() {}
        fn sys_b() {}
        fn sys_c() {}

        let mut schedule = Schedule::new();
        // c.after(b.after(a)):  a → b → c
        // Registration order (depth-first): sys_a(0), sys_b(1), sys_c(2)
        // Edges: 0→1, 1→2
        schedule.add_system(sys_c.after(sys_b.after(sys_a)));

        assert_eq!(schedule.graph.node_count(), 3);
        assert_eq!(schedule.graph.edge_count(), 2);
        assert!(
            schedule
                .graph
                .contains_edge(NodeIndex::new(0), NodeIndex::new(1))
        );
        assert!(
            schedule
                .graph
                .contains_edge(NodeIndex::new(1), NodeIndex::new(2))
        );
    }

    // ── World::run_schedule reentrancy ──────────────────────────────────────

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

    #[derive(crate::Resource, Default)]
    struct Counter(u32);

    #[test]
    fn run_schedule_supports_a_schedule_calling_run_schedule_from_within_itself() {
        use crate::resource::ResMut;

        let mut world = World::new();
        world.insert_resource(Counter::default());

        let mut schedules = Schedules::default();
        schedules.add_system(Inner, |mut counter: ResMut<Counter>| counter.0 += 1);
        schedules.add_system(Outer, |world: &mut World| world.run_schedule(Inner));
        let compiled_schedules = schedules.compile::<SingleThreadedExecutor>(&mut world);
        world.insert_resource(compiled_schedules);

        // Outer's system calls world.run_schedule(Inner) while Outer's own entry is
        // still removed from CompiledSchedules — Inner must still be reachable.
        world.run_schedule(Outer);
        assert_eq!(world.get_resource::<Counter>().unwrap().0, 1);

        // Both schedules must have been put back after running, not dropped.
        world.run_schedule(Outer);
        assert_eq!(world.get_resource::<Counter>().unwrap().0, 2);
    }
}
