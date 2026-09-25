use std::{
    any::TypeId,
    collections::{HashMap, VecDeque},
    fmt,
};

use crate::{
    Resource, System, define_label,
    intern::Interned,
    system::{
        BoxedSystem,
        access::SystemAccess,
        config::{DependencyTarget, IntoSystemConfig, IntoSystemConfigs},
        executor::SystemExecutor,
        graph::{SystemDependencyGraph, SystemNode},
        meta::SystemMetadata,
        reachability::Reachability,
        set::{InternedSystemSet, IntoSetConfigs, SetConfig},
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

pub use concerto_ecs_macros::ScheduleLabel;

#[derive(Clone, Copy, Eq, Hash, PartialEq, Deref, From)]
pub struct SystemNodeIndex(NodeIndex);

#[derive(Clone, Copy, Eq, Hash, PartialEq, Deref, From)]
pub struct SystemIndex(usize);

/// A system's set memberships and ordering constraints, kept until [`Schedule::compile`].
#[derive(Default)]
struct NodeConfig {
    sets: Vec<InternedSystemSet>,
    after: Vec<DependencyTarget>,
    before: Vec<DependencyTarget>,
}

/// A collection of systems and the ordering constraints between them.
///
/// Systems are added with [`add_system`](Schedule::add_system) or
/// [`add_systems`](Schedule::add_systems), and sets are ordered with
/// [`configure_sets`](Schedule::configure_sets). Nothing is resolved until
/// [`compile`](Schedule::compile), so a constraint may name a system or set that is
/// registered later.
///
/// Systems with conflicting data access and no explicit constraint between them run
/// in registration order.
///
/// # Example
/// ```
/// use concerto_ecs::{Schedule, World, Component, Query};
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
    systems: Vec<BoxedSystem>,
    configs: Vec<NodeConfig>,
    set_configs: Vec<SetConfig>,
}

impl Schedule {
    /// Creates an empty schedule.
    pub fn new() -> Schedule {
        Self::default()
    }

    /// Adds a system (or [`SystemConfig`](crate::system::config::SystemConfig)) to the schedule.
    ///
    /// ```
    /// # use concerto_ecs::{Schedule, IntoSystemConfig};
    /// # fn a() {} fn b() {} fn c() {}
    /// let mut schedule = Schedule::new();
    /// schedule
    ///     .add_system(a)
    ///     .add_system(b.after(a))
    ///     .add_system(c.after(b).before(a));
    /// ```
    pub fn add_system<M>(&mut self, system: impl IntoSystemConfig<M> + 'static) -> &mut Self {
        let config = system.into_config();
        self.systems.push(config.system);
        self.configs.push(NodeConfig {
            sets: config.sets,
            after: config.after,
            before: config.before,
        });
        self
    }

    /// Adds several systems to the schedule.
    ///
    /// ```
    /// # use concerto_ecs::{Schedule, IntoSystemConfigs};
    /// # fn a() {} fn b() {} fn c() {}
    /// let mut schedule = Schedule::new();
    /// schedule.add_systems((a, b, c).chain());
    /// ```
    pub fn add_systems<M>(&mut self, systems: impl IntoSystemConfigs<M>) -> &mut Self {
        for config in systems.into_configs() {
            self.add_system(config);
        }
        self
    }

    /// Declares ordering constraints on sets in this schedule.
    ///
    /// Configuring the same set more than once accumulates its constraints.
    pub fn configure_sets(&mut self, configs: impl IntoSetConfigs) -> &mut Self {
        self.set_configs.extend(configs.into_set_configs());
        self
    }

    pub fn compile<T: SystemExecutor + 'static>(mut self, world: &mut World) -> CompiledSchedule {
        self.insert_sync_points();

        let mut graph = SystemDependencyGraph::new();
        let nodes: Vec<SystemNodeIndex> = self
            .systems
            .iter()
            .enumerate()
            .map(|(index, system)| {
                let mut access = SystemAccess::default();
                let mut metadata = SystemMetadata::default();
                system.fill_access(&mut metadata, &mut access);
                graph
                    .add_node(SystemNode::new(
                        index.into(),
                        access,
                        metadata,
                        system.name(),
                    ))
                    .into()
            })
            .collect();

        let by_type = self.index_by_type();
        let by_set = self.index_by_set();
        let mut reachability = Reachability::new(self.systems.len());
        let mut explicit = ExplicitEdges {
            graph: &mut graph,
            nodes: &nodes,
            reachability: &mut reachability,
        };

        for (index, config) in self.configs.iter().enumerate() {
            let owner = format!("`{}`", self.systems[index].name());
            for target in &config.after {
                for source in resolve(target, &by_type, &by_set, &owner) {
                    explicit.add(source, index, &owner);
                }
            }
            for target in &config.before {
                for sink in resolve(target, &by_type, &by_set, &owner) {
                    explicit.add(index, sink, &owner);
                }
            }
        }

        for set_config in &self.set_configs {
            let Some(members) = by_set.get(&set_config.set) else {
                continue;
            };
            let owner = format!("set `{:?}`", set_config.set);
            for target in &set_config.after {
                for source in resolve(target, &by_type, &by_set, &owner) {
                    for member in members {
                        explicit.add(source, *member, &owner);
                    }
                }
            }
            for target in &set_config.before {
                for sink in resolve(target, &by_type, &by_set, &owner) {
                    for member in members {
                        explicit.add(*member, sink, &owner);
                    }
                }
            }
        }

        add_implicit_edges(&mut graph, &nodes, &mut reachability);

        self.systems
            .iter_mut()
            .for_each(|system| system.initialize(world));

        let compiled_data = build_compiled_data(self.systems, &graph, &nodes);

        CompiledSchedule {
            executor: Box::new(T::init(&compiled_data)),
            compiled_data,
            graph,
        }
    }

    /// Inserts a sync point after every system that has deferred work to apply.
    fn insert_sync_points(&mut self) {
        let mut systems = Vec::with_capacity(self.systems.len());
        let mut configs = Vec::with_capacity(self.configs.len());

        for (system, config) in self.systems.drain(..).zip(self.configs.drain(..)) {
            let mut access = SystemAccess::default();
            let mut metadata = SystemMetadata::default();
            system.fill_access(&mut metadata, &mut access);
            let needs_sync = access.needs_apply() && !is_sync_point(system.as_ref());

            systems.push(system);
            configs.push(config);

            if needs_sync {
                systems.push(Box::new(SyncPoint));
                configs.push(NodeConfig::default());
            }
        }

        self.systems = systems;
        self.configs = configs;
    }

    fn index_by_type(&self) -> HashMap<TypeId, Vec<usize>> {
        let mut index: HashMap<TypeId, Vec<usize>> = HashMap::new();
        for (position, system) in self.systems.iter().enumerate() {
            if is_sync_point(system.as_ref()) {
                continue;
            }
            index
                .entry(system.system_type())
                .or_default()
                .push(position);
        }
        index
    }

    fn index_by_set(&self) -> HashMap<InternedSystemSet, Vec<usize>> {
        let mut index: HashMap<InternedSystemSet, Vec<usize>> = HashMap::new();
        for (position, config) in self.configs.iter().enumerate() {
            for set in &config.sets {
                let members = index.entry(*set).or_default();
                if !members.contains(&position) {
                    members.push(position);
                }
            }
        }
        index
    }
}

/// Resolves an ordering target to the positions of the systems it names.
fn resolve(
    target: &DependencyTarget,
    by_type: &HashMap<TypeId, Vec<usize>>,
    by_set: &HashMap<InternedSystemSet, Vec<usize>>,
    owner: &str,
) -> Vec<usize> {
    match target {
        DependencyTarget::System { id, name } => match by_type.get(id) {
            Some(matches) => {
                if matches.len() > 1 {
                    log::warn!(
                        "{owner} is ordered against `{name}`, which is registered {} times in this schedule; ordering against all copies",
                        matches.len()
                    );
                }
                matches.clone()
            }
            None => {
                log::warn!(
                    "{owner} is ordered against `{name}`, which is not in this schedule; the constraint is ignored"
                );
                Vec::new()
            }
        },
        DependencyTarget::Set(set) => by_set.get(set).cloned().unwrap_or_default(),
    }
}

/// Adds explicit ordering edges, panicking on any edge that would close a cycle.
struct ExplicitEdges<'a> {
    graph: &'a mut SystemDependencyGraph,
    nodes: &'a [SystemNodeIndex],
    reachability: &'a mut Reachability,
}

impl ExplicitEdges<'_> {
    fn add(&mut self, from: usize, to: usize, owner: &str) {
        if from == to {
            return;
        }
        if self.reachability.reaches(to, from) {
            let path = self
                .path(to, from)
                .into_iter()
                .map(|index| format!("`{}`", self.name(index)))
                .collect::<Vec<_>>()
                .join(" -> ");
            panic!(
                "Cycle in schedule ordering: {owner} requires `{}` to run before `{}`, but other constraints already order {path}",
                self.name(from),
                self.name(to),
            );
        }
        self.graph
            .update_edge(*self.nodes[from], *self.nodes[to], ());
        self.reachability.add_edge(from, to);
    }

    fn name(&self, index: usize) -> &'static str {
        self.graph.node_weight(*self.nodes[index]).unwrap().name
    }

    /// Finds a path of explicit edges from `from` to `to`, as system positions.
    fn path(&self, from: usize, to: usize) -> Vec<usize> {
        let mut previous: HashMap<NodeIndex, NodeIndex> = HashMap::new();
        let mut queue = VecDeque::from([*self.nodes[from]]);
        let target = *self.nodes[to];

        while let Some(node) = queue.pop_front() {
            if node == target {
                break;
            }
            for next in self.graph.neighbors_directed(node, Direction::Outgoing) {
                if next != *self.nodes[from] && !previous.contains_key(&next) {
                    previous.insert(next, node);
                    queue.push_back(next);
                }
            }
        }

        let mut path = vec![target];
        while let Some(node) = previous.get(path.last().unwrap()) {
            path.push(*node);
        }
        path.into_iter()
            .rev()
            .map(|node| *self.graph.node_weight(node).unwrap().index())
            .collect()
    }
}

/// Orders every pair of access-conflicting systems by registration order, unless an
/// explicit constraint already orders them the other way.
fn add_implicit_edges(
    graph: &mut SystemDependencyGraph,
    nodes: &[SystemNodeIndex],
    reachability: &mut Reachability,
) {
    for later in 0..nodes.len() {
        for earlier in 0..later {
            let conflicts = !SystemAccess::are_disjoint(
                graph.node_weight(*nodes[earlier]).unwrap().access(),
                graph.node_weight(*nodes[later]).unwrap().access(),
            );
            if conflicts && reachability.try_add_edge(earlier, later) {
                graph.update_edge(*nodes[earlier], *nodes[later], ());
            }
        }
    }
}

fn build_compiled_data(
    systems: Vec<BoxedSystem>,
    graph: &SystemDependencyGraph,
    nodes: &[SystemNodeIndex],
) -> CompiledScheduleData {
    let dependency_count: Vec<usize> = nodes
        .iter()
        .map(|idx| graph.neighbors_directed(**idx, Direction::Incoming).count())
        .collect();

    let dependants = nodes
        .iter()
        .map(|idx| {
            graph
                .neighbors_directed(**idx, Direction::Outgoing)
                .map(|node_index| *graph.node_weight(node_index).unwrap().index())
                .collect()
        })
        .collect();

    let system_access: Vec<SystemAccess> = nodes
        .iter()
        .map(|idx| graph.node_weight(**idx).unwrap().access().clone())
        .collect();

    let system_meta: Vec<SystemMetadata> = nodes
        .iter()
        .map(|idx| graph.node_weight(**idx).unwrap().meta().clone())
        .collect();

    let sorted_systems = toposort(graph, None)
        .expect("schedule graph is acyclic by construction")
        .into_iter()
        .map(|node_index| *graph.node_weight(node_index).unwrap().index())
        .collect::<Vec<_>>();

    CompiledScheduleData {
        systems,
        sorted_systems,
        dependency_count,
        dependants,
        system_access,
        system_meta,
    }
}

impl fmt::Debug for Schedule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.systems.iter().map(|system| system.name()))
            .finish()
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

    /// Registers several systems in the schedule identified by `update_group`.
    pub fn add_systems<M>(
        &mut self,
        update_group: impl ScheduleLabel,
        systems: impl IntoSystemConfigs<M>,
    ) {
        self.schedules
            .entry(update_group.intern())
            .or_default()
            .add_systems(systems);
    }

    /// Declares ordering constraints on sets in the schedule identified by `update_group`.
    pub fn configure_sets(
        &mut self,
        update_group: impl ScheduleLabel,
        configs: impl IntoSetConfigs,
    ) {
        self.schedules
            .entry(update_group.intern())
            .or_default()
            .configure_sets(configs);
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

    #[test]
    fn schedule_new() {
        let schedule = Schedule::new();
        assert_eq!(schedule.systems.len(), 0);
    }

    #[test]
    fn add_system_builder_style() {
        let mut schedule = Schedule::new();
        schedule.add_system(|| {}).add_system(|| {});

        assert_eq!(schedule.systems.len(), 2);
    }

    #[test]
    fn multiple_systems_added() {
        let mut schedule = Schedule::new();
        schedule
            .add_system(|| {})
            .add_system(|| {})
            .add_system(|| {});

        assert_eq!(schedule.systems.len(), 3);
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

    #[test]
    fn a_constraint_does_not_register_its_target() {
        fn dep() {}
        fn main_sys() {}

        let mut schedule = Schedule::new();
        schedule.add_system(main_sys.after(dep));

        assert_eq!(schedule.systems.len(), 1);
    }

    #[test]
    fn a_system_added_twice_is_two_nodes() {
        fn sys() {}

        let mut schedule = Schedule::new();
        schedule.add_system(sys).add_system(sys);

        assert_eq!(schedule.systems.len(), 2);
    }

    #[test]
    fn after_and_before_produce_one_edge_each() {
        fn sys_a() {}
        fn sys_b() {}
        fn sys_c() {}

        let mut schedule = Schedule::new();
        schedule
            .add_system(sys_c)
            .add_system(sys_b.after(sys_a).before(sys_c))
            .add_system(sys_a);

        let mut world = World::new();
        let compiled = schedule.compile::<SingleThreadedExecutor>(&mut world);

        assert_eq!(compiled.graph.node_count(), 3);
        assert_eq!(compiled.graph.edge_count(), 2);
        assert!(
            compiled
                .graph
                .contains_edge(NodeIndex::new(2), NodeIndex::new(1))
        );
        assert!(
            compiled
                .graph
                .contains_edge(NodeIndex::new(1), NodeIndex::new(0))
        );
    }

    #[test]
    fn a_self_referential_constraint_does_not_deadlock_compile() {
        fn sys() {}

        let mut schedule = Schedule::new();
        schedule.add_system(sys.after(sys));

        let mut world = World::new();
        schedule
            .compile::<SingleThreadedExecutor>(&mut world)
            .run(&mut world);
    }

    #[test]
    #[should_panic(expected = "Cycle in schedule ordering")]
    fn contradicting_explicit_constraints_panic() {
        fn a() {}
        fn b() {}

        let mut schedule = Schedule::new();
        schedule.add_system(a.before(b));
        schedule.add_system(b.before(a));

        let mut world = World::new();
        let _ = schedule.compile::<SingleThreadedExecutor>(&mut world);
    }

    #[test]
    fn a_cycle_panic_names_both_systems() {
        fn cycle_a() {}
        fn cycle_b() {}

        let result = std::panic::catch_unwind(|| {
            let mut schedule = Schedule::new();
            schedule.add_system(cycle_a.before(cycle_b));
            schedule.add_system(cycle_b.before(cycle_a));

            let mut world = World::new();
            let _ = schedule.compile::<SingleThreadedExecutor>(&mut world);
        });

        let payload = result.unwrap_err();
        let message = payload.downcast_ref::<String>().unwrap();
        assert!(message.contains("cycle_a"), "{message}");
        assert!(message.contains("cycle_b"), "{message}");
    }

    #[test]
    fn an_explicit_constraint_across_a_sync_point_compiles() {
        use crate::command::CommandQueue;
        use crate::resource::ResMut;

        #[derive(crate::Resource, Default)]
        struct Order(Vec<&'static str>);

        fn deferred(_commands: CommandQueue, mut order: ResMut<Order>) {
            order.0.push("deferred");
        }
        fn reader(mut order: ResMut<Order>) {
            order.0.push("reader");
        }

        let mut schedule = Schedule::new();
        schedule.add_system(deferred);
        schedule.add_system(reader.before(deferred));

        let mut world = World::new();
        world.insert_resource(Order::default());
        schedule
            .compile::<SingleThreadedExecutor>(&mut world)
            .run(&mut world);

        assert_eq!(
            world.get_resource::<Order>().unwrap().0,
            vec!["reader", "deferred"]
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
