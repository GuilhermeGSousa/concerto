use std::{
    any::TypeId,
    cmp::Reverse,
    collections::{BinaryHeap, HashMap, VecDeque},
    fmt,
};

use crate::{
    Resource, System,
    component::CheckChangeTicks,
    define_label,
    intern::Interned,
    system::{
        BoxedSystem,
        access::SystemAccess,
        config::{DependencyTarget, IntoSystemConfig, NodeConfig},
        executor::SystemExecutor,
        graph::{SystemDependencyGraph, SystemNode},
        meta::SystemMetadata,
        reachability::Reachability,
        set::{InternedSystemSet, IntoSetConfig, SetEntry},
        stand_in::StandIn,
        sync_point::SyncPoint,
        tag::TagFilter,
    },
    world::World,
};
use derive_more::{Deref, From};
use fixedbitset::FixedBitSet;
use petgraph::{
    Direction,
    dot::{Config, Dot},
    graph::NodeIndex,
};

pub use concerto_ecs_macros::ScheduleLabel;

#[doc(hidden)]
#[derive(Clone, Copy, Eq, Hash, PartialEq, Deref, From)]
pub struct SystemNodeIndex(NodeIndex);

#[doc(hidden)]
#[derive(Clone, Copy, Eq, Hash, PartialEq, Deref, From)]
pub struct SystemIndex(usize);

/// A collection of systems and the ordering constraints between them.
///
/// Nothing is resolved until [`compile`](Schedule::compile), so a constraint may name a system
/// or set registered later. Conflicting systems with no explicit constraint run in
/// registration order; an explicit constraint always wins.
///
/// # Examples
///
/// ```
/// use concerto_ecs::{
///     IntoSystemConfig, Schedule, World,
///     system::executor::single_thread::SingleThreadedExecutor,
/// };
///
/// fn first() {}
/// fn second() {}
///
/// let mut schedule = Schedule::new();
/// schedule.add_system(second.after(first)).add_system(first);
///
/// let mut world = World::new();
/// schedule
///     .compile::<SingleThreadedExecutor>(&mut world)
///     .run(&mut world);
/// ```
#[derive(Default)]
pub struct Schedule {
    systems: Vec<BoxedSystem>,
    configs: Vec<NodeConfig>,
    set_configs: Vec<SetEntry>,
}

impl Schedule {
    /// Creates an empty schedule.
    pub fn new() -> Schedule {
        Self::default()
    }

    /// Adds a system, a tuple of systems, or a [`SystemConfig`](crate::SystemConfig).
    pub fn add_system<M>(&mut self, system: impl IntoSystemConfig<M>) -> &mut Self {
        let mut entries = Vec::new();
        system.into_config().into_entries(&mut entries);
        for entry in entries {
            self.systems.push(entry.system);
            self.configs.push(entry.config);
        }
        self
    }

    /// Adds ordering constraints between sets; repeated calls accumulate.
    pub fn configure_sets(&mut self, configs: impl IntoSetConfig) -> &mut Self {
        configs
            .into_set_config()
            .into_entries(&mut self.set_configs);
        self
    }

    fn stand_in_for_inactive(&mut self, filter: &TagFilter) {
        for (system, config) in self.systems.iter_mut().zip(&self.configs) {
            if !filter.admits(&config.tags) {
                *system = Box::new(StandIn::of(system.as_ref()));
            }
        }
    }

    /// Resolves every constraint and initializes the systems for executor `T`.
    ///
    /// A system target missing from this schedule is ignored with a warning; an empty set is
    /// ignored silently.
    ///
    /// # Panics
    ///
    /// Panics if the explicit constraints contain a cycle.
    pub fn compile<T: SystemExecutor + 'static>(self, world: &mut World) -> CompiledSchedule {
        self.compile_filtered::<T>(world, &TagFilter::default())
    }

    /// Like [`compile`](Schedule::compile), with every system `filter` rejects replaced by a stand-in.
    pub fn compile_filtered<T: SystemExecutor + 'static>(
        mut self,
        world: &mut World,
        filter: &TagFilter,
    ) -> CompiledSchedule {
        self.stand_in_for_inactive(filter);
        self.systems
            .iter_mut()
            .for_each(|system| system.initialize(world));

        let mut graph = SystemDependencyGraph::new();
        let mut nodes: Vec<SystemNodeIndex> = self
            .systems
            .iter()
            .enumerate()
            .map(|(index, system)| add_node(&mut graph, index, system.as_ref()))
            .collect();

        let mut reachability = Reachability::new(self.systems.len());
        self.add_explicit_edges(&mut ExplicitEdges {
            graph: &mut graph,
            nodes: &nodes,
            reachability: &mut reachability,
        });

        let registered = self.systems.len();
        let mut schedule_graph = ScheduleGraph {
            systems: &mut self.systems,
            graph: &mut graph,
            nodes: &mut nodes,
            reachability: &mut reachability,
        };
        schedule_graph.insert_sync_points(world);
        schedule_graph.add_implicit_edges(registered);
        let order = schedule_graph.reduce_transitively();

        let compiled_data = CompiledScheduleData::build(self.systems, &graph, &nodes, &order);

        CompiledSchedule {
            executor: Box::new(T::init(&compiled_data)),
            compiled_data,
            graph,
        }
    }

    fn add_explicit_edges(&self, explicit: &mut ExplicitEdges) {
        let by_type = self.index_by_type();
        let by_set = self.index_by_set();

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
                    for member in members.ones() {
                        explicit.add(source, member, &owner);
                    }
                }
            }
            for target in &set_config.before {
                for sink in resolve(target, &by_type, &by_set, &owner) {
                    for member in members.ones() {
                        explicit.add(member, sink, &owner);
                    }
                }
            }
        }
    }

    fn index_by_type(&self) -> HashMap<TypeId, FixedBitSet> {
        let mut index: HashMap<TypeId, FixedBitSet> = HashMap::new();
        for (position, system) in self.systems.iter().enumerate() {
            index
                .entry(system.system_type())
                .or_insert_with(|| FixedBitSet::with_capacity(self.systems.len()))
                .insert(position);
        }
        index
    }

    fn index_by_set(&self) -> HashMap<InternedSystemSet, FixedBitSet> {
        let mut index: HashMap<InternedSystemSet, FixedBitSet> = HashMap::new();
        for (position, config) in self.configs.iter().enumerate() {
            for set in &config.sets {
                index
                    .entry(*set)
                    .or_insert_with(|| FixedBitSet::with_capacity(self.systems.len()))
                    .insert(position);
            }
        }
        index
    }
}

fn resolve<'a>(
    target: &DependencyTarget,
    by_type: &'a HashMap<TypeId, FixedBitSet>,
    by_set: &'a HashMap<InternedSystemSet, FixedBitSet>,
    owner: &str,
) -> impl Iterator<Item = usize> + use<'a> {
    let targets = match target {
        DependencyTarget::System { id, name } => {
            let matches = by_type.get(id);
            match matches.map(|matches| matches.count_ones(..)) {
                None => log::warn!(
                    "{owner} is ordered against `{name}`, which is not in this schedule; the constraint is ignored"
                ),
                Some(count) if count > 1 => log::warn!(
                    "{owner} is ordered against `{name}`, which is registered {count} times in this schedule; ordering against all copies"
                ),
                Some(_) => {}
            }
            matches
        }
        DependencyTarget::Set(set) => by_set.get(set),
    };
    targets.into_iter().flat_map(FixedBitSet::ones)
}

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

fn add_node(
    graph: &mut SystemDependencyGraph,
    index: usize,
    system: &dyn System<In = ()>,
) -> SystemNodeIndex {
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
}

struct ScheduleGraph<'a> {
    systems: &'a mut Vec<BoxedSystem>,
    graph: &'a mut SystemDependencyGraph,
    nodes: &'a mut Vec<SystemNodeIndex>,
    reachability: &'a mut Reachability,
}

impl ScheduleGraph<'_> {
    fn access(&self, index: usize) -> &SystemAccess {
        self.graph.node_weight(*self.nodes[index]).unwrap().access()
    }

    fn successors(&self, index: usize) -> Vec<usize> {
        self.graph
            .neighbors_directed(*self.nodes[index], Direction::Outgoing)
            .map(|node| *self.graph.node_weight(node).unwrap().index())
            .collect()
    }

    fn topological_order(&self) -> Vec<usize> {
        let count = self.nodes.len();
        let mut waiting: Vec<usize> = self
            .nodes
            .iter()
            .map(|node| {
                self.graph
                    .neighbors_directed(**node, Direction::Incoming)
                    .count()
            })
            .collect();
        let mut ready: BinaryHeap<Reverse<usize>> = (0..count)
            .filter(|&index| waiting[index] == 0)
            .map(Reverse)
            .collect();
        let mut order = Vec::with_capacity(count);
        while let Some(Reverse(index)) = ready.pop() {
            order.push(index);
            for successor in self.successors(index) {
                waiting[successor] -= 1;
                if waiting[successor] == 0 {
                    ready.push(Reverse(successor));
                }
            }
        }
        assert_eq!(
            order.len(),
            count,
            "schedule graph is acyclic by construction"
        );
        order
    }

    fn link(&mut self, from: usize, to: usize) {
        self.graph
            .update_edge(*self.nodes[from], *self.nodes[to], ());
        self.reachability.add_edge(from, to);
    }

    fn carries_commands_to(&self, from: usize, to: usize) -> bool {
        self.access(from).needs_apply() && !self.access(to).is_exclusive()
    }

    fn sync_distances(&self) -> Vec<usize> {
        let mut distance = vec![0; self.nodes.len()];
        for from in self.topological_order() {
            for to in self.successors(from) {
                let step = usize::from(self.carries_commands_to(from, to));
                distance[to] = distance[to].max(distance[from] + step);
            }
        }
        distance
    }

    fn insert_sync_points(&mut self, world: &mut World) {
        let distance = self.sync_distances();
        let mut sync_points: HashMap<usize, usize> = HashMap::new();
        for from in 0..distance.len() {
            for to in self.successors(from) {
                if !self.carries_commands_to(from, to) {
                    continue;
                }
                let sync_point = match sync_points.get(&distance[to]) {
                    Some(&sync_point) => sync_point,
                    None => {
                        let sync_point = self.add_sync_point(world);
                        sync_points.insert(distance[to], sync_point);
                        sync_point
                    }
                };
                self.link(from, sync_point);
                self.link(sync_point, to);
            }
        }
    }

    fn add_sync_point(&mut self, world: &mut World) -> usize {
        let mut sync_point: BoxedSystem = Box::new(SyncPoint);
        sync_point.initialize(world);
        let index = self.systems.len();
        self.nodes
            .push(add_node(self.graph, index, sync_point.as_ref()));
        self.systems.push(sync_point);
        self.reachability.add_node();
        index
    }

    fn add_implicit_edges(&mut self, registered: usize) {
        for later in 0..registered {
            for earlier in 0..later {
                let conflicts =
                    !SystemAccess::are_disjoint(self.access(earlier), self.access(later));
                if conflicts
                    && !self.reachability.reaches(earlier, later)
                    && self.reachability.try_add_edge(earlier, later)
                {
                    self.graph
                        .update_edge(*self.nodes[earlier], *self.nodes[later], ());
                }
            }
        }
    }

    fn reduce_transitively(&mut self) -> Vec<usize> {
        let order = self.topological_order();
        let successors: Vec<Vec<usize>> = (0..order.len())
            .map(|index| self.successors(index))
            .collect();
        let redundant = self.reachability.redundant_edges(&successors, &order);
        self.graph.retain_edges(|graph, edge| {
            let (from, to) = graph.edge_endpoints(edge).unwrap();
            !redundant[*graph[from].index()].contains(*graph[to].index())
        });
        order
    }
}

impl CompiledScheduleData {
    fn build(
        systems: Vec<BoxedSystem>,
        graph: &SystemDependencyGraph,
        nodes: &[SystemNodeIndex],
        order: &[usize],
    ) -> Self {
        let mut position = vec![0; order.len()];
        for (new, &old) in order.iter().enumerate() {
            position[old] = new;
        }
        let node = |old: usize| graph.node_weight(*nodes[old]).unwrap();

        let mut slots: Vec<Option<BoxedSystem>> = systems.into_iter().map(Some).collect();
        let systems = order
            .iter()
            .map(|&old| slots[old].take().unwrap())
            .collect();

        let dependency_count = order
            .iter()
            .map(|&old| {
                graph
                    .neighbors_directed(*nodes[old], Direction::Incoming)
                    .count()
            })
            .collect();

        let dependants = order
            .iter()
            .map(|&old| {
                graph
                    .neighbors_directed(*nodes[old], Direction::Outgoing)
                    .map(|next| position[*graph.node_weight(next).unwrap().index()])
                    .collect()
            })
            .collect();

        let system_access = order
            .iter()
            .map(|&old| node(old).access().clone())
            .collect();

        let system_meta = order.iter().map(|&old| node(old).meta().clone()).collect();

        Self {
            systems,
            dependency_count,
            dependants,
            system_access,
            system_meta,
        }
    }
}

impl fmt::Debug for Schedule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.systems.iter().map(|system| system.name()))
            .finish()
    }
}

/// A [`Schedule`] with its ordering resolved, ready to run.
pub struct CompiledSchedule {
    executor: Box<dyn SystemExecutor>,
    compiled_data: CompiledScheduleData,
    graph: SystemDependencyGraph,
}

impl CompiledSchedule {
    /// Runs the schedule once, first giving the world its tick check when one is due.
    pub fn run(&mut self, world: &mut World) {
        if let Some(check) = world.check_change_ticks() {
            self.check_change_ticks(check);
        }
        self.executor.run(&mut self.compiled_data, world);
    }

    /// Clamps every system's stored ticks that are older than the check allows.
    pub fn check_change_ticks(&mut self, check: CheckChangeTicks) {
        for system in &mut self.compiled_data.systems {
            system.check_change_tick(check);
        }
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

/// The resolved systems and dependency graph a [`SystemExecutor`] runs.
pub struct CompiledScheduleData {
    /// The initialized systems, including inserted sync points, in topological order.
    pub systems: Vec<BoxedSystem>,
    /// How many systems must finish before each system may start.
    pub dependency_count: Vec<usize>,
    /// For each system, the positions of the systems waiting on it.
    pub dependants: Vec<Vec<usize>>,
    /// The data each system reads and writes.
    pub system_access: Vec<SystemAccess>,
    /// Scheduling metadata for each system.
    pub system_meta: Vec<SystemMetadata>,
}

define_label!(
    /// A name for a [`Schedule`]; derive it with `#[derive(ScheduleLabel)]`.
    ScheduleLabel
);

/// A [`ScheduleLabel`] interned for cheap copying and comparison.
pub type InternedScheduleLabel = Interned<dyn ScheduleLabel>;

/// Every uncompiled [`Schedule`] of a world, keyed by [`ScheduleLabel`].
#[derive(Resource, Default, Debug)]
pub struct Schedules {
    schedules: HashMap<InternedScheduleLabel, Schedule>,
}

impl Schedules {
    /// Adds systems to the schedule labelled `update_group`.
    pub fn add_system<M>(
        &mut self,
        update_group: impl ScheduleLabel,
        system: impl IntoSystemConfig<M>,
    ) {
        self.schedules
            .entry(update_group.intern())
            .or_default()
            .add_system(system);
    }

    /// Adds ordering constraints between sets in the schedule labelled `update_group`.
    pub fn configure_sets(
        &mut self,
        update_group: impl ScheduleLabel,
        configs: impl IntoSetConfig,
    ) {
        self.schedules
            .entry(update_group.intern())
            .or_default()
            .configure_sets(configs);
    }

    /// Compiles every schedule for executor `T`.
    ///
    /// # Panics
    ///
    /// Panics if any schedule's explicit constraints contain a cycle.
    pub fn compile<T: SystemExecutor + 'static>(self, world: &mut World) -> CompiledSchedules {
        self.compile_filtered::<T>(world, &TagFilter::default())
    }

    /// Like [`compile`](Schedules::compile), with `filter` applied to every schedule.
    pub fn compile_filtered<T: SystemExecutor + 'static>(
        self,
        world: &mut World,
        filter: &TagFilter,
    ) -> CompiledSchedules {
        CompiledSchedules {
            compiled_schedules: self
                .schedules
                .into_iter()
                .map(|(label, schedule)| (label, schedule.compile_filtered::<T>(world, filter)))
                .collect(),
        }
    }
}

/// Every [`CompiledSchedule`] of a world, keyed by [`ScheduleLabel`].
#[derive(Resource, Debug, Default)]
pub struct CompiledSchedules {
    compiled_schedules: HashMap<InternedScheduleLabel, CompiledSchedule>,
}

impl CompiledSchedules {
    /// Returns the compiled schedule labelled `label`, or `None` when there is none.
    pub fn get(&self, label: impl ScheduleLabel) -> Option<&CompiledSchedule> {
        self.compiled_schedules.get(&label.intern())
    }

    /// Returns the compiled schedule labelled `label` mutably, or `None` when there is none.
    pub fn get_mut(&mut self, label: impl ScheduleLabel) -> Option<&mut CompiledSchedule> {
        self.compiled_schedules.get_mut(&label.intern())
    }

    /// Takes the compiled schedule labelled `label` out, or returns `None` when there is none.
    pub fn remove(&mut self, label: impl ScheduleLabel) -> Option<CompiledSchedule> {
        self.compiled_schedules.remove(&label.intern())
    }

    pub(crate) fn insert(&mut self, label: impl ScheduleLabel, schedule: CompiledSchedule) {
        self.compiled_schedules.insert(label.intern(), schedule);
    }

    pub(crate) fn check_change_ticks(&mut self, check: CheckChangeTicks) {
        for schedule in self.compiled_schedules.values_mut() {
            schedule.check_change_ticks(check);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::executor::single_thread::SingleThreadedExecutor;

    use crate::testing::{Inner, Outer};

    // ── World::run_schedule reentrancy ──────────────────────────────────────

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

    fn queue_one(_: crate::CommandQueue) {}

    fn queue_two(_: crate::CommandQueue) {}

    #[test]
    fn unordered_command_systems_wait_on_nothing() {
        let mut world = World::new();
        let mut schedule = Schedule::new();
        schedule.add_system(queue_one).add_system(queue_two);

        let compiled = schedule.compile::<SingleThreadedExecutor>(&mut world);

        assert_eq!(compiled.compiled_data.dependency_count, [0, 0]);
    }
}
