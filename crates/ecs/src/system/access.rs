use std::{
    any::TypeId,
    collections::{HashMap, HashSet},
};

use crate::{component::Component, resource::Resource};

/// The components a query's matches must have and must not have.
#[derive(Default, Clone)]
pub struct QueryShape {
    with: HashSet<TypeId>,
    without: HashSet<TypeId>,
}

impl QueryShape {
    /// Requires matches to have the component `component`.
    pub fn require(&mut self, component: TypeId) {
        self.with.insert(component);
    }

    /// Requires matches not to have the component `component`.
    pub fn exclude(&mut self, component: TypeId) {
        self.without.insert(component);
    }

    fn is_disjoint_from(&self, other: &Self) -> bool {
        !self.with.is_disjoint(&other.without) || !other.with.is_disjoint(&self.without)
    }
}

#[derive(Default, Clone)]
struct QueryAccess {
    reads: HashSet<TypeId>,
    writes: HashSet<TypeId>,
    shape: QueryShape,
}

impl QueryAccess {
    fn conflict_with(&self, other: &Self) -> Option<TypeId> {
        if self.shape.is_disjoint_from(&other.shape) {
            return None;
        }
        self.writes
            .iter()
            .find(|id| other.reads.contains(id) || other.writes.contains(id))
            .or_else(|| other.writes.iter().find(|id| self.reads.contains(id)))
            .copied()
    }
}

/// What a system reads and writes: components, resources, or the whole world.
///
/// The scheduler runs two systems at once only when [`SystemAccess::are_disjoint`] says their
/// access cannot conflict.
#[derive(Default, Clone)]
pub struct SystemAccess {
    queries: Vec<QueryAccess>,
    names: HashMap<TypeId, &'static str>,
    conflict: Option<String>,
    component_reads: HashSet<TypeId>,
    component_writes: HashSet<TypeId>,
    resource_reads: HashSet<TypeId>,
    resource_writes: HashSet<TypeId>,
    reads_all: bool,
    reads_all_components: bool,
    writes_all: bool,
    needs_apply: bool,
}

impl SystemAccess {
    /// Records that the system reads the component `T`.
    pub fn read_component<T: Component>(&mut self) {
        self.name::<T>();
        self.component_reads.insert(TypeId::of::<T>());
    }

    /// Records that the system writes the component `T`.
    pub fn write_component<T: Component>(&mut self) {
        self.name::<T>();
        self.component_writes.insert(TypeId::of::<T>());
    }

    /// Records that the system reads the resource `T`.
    pub fn read_resource<T: Resource>(&mut self) {
        self.name::<T>();
        self.resource_reads.insert(TypeId::of::<T>());
    }

    /// Records that the system writes the resource `T`.
    pub fn write_resource<T: Resource>(&mut self) {
        self.name::<T>();
        self.resource_writes.insert(TypeId::of::<T>());
    }

    /// Records that the system reads every component.
    pub fn read_all_components(&mut self) {
        self.reads_all_components = true;
    }

    /// Records that the system reads the whole world.
    pub fn read_world(&mut self) {
        self.reads_all = true;
    }

    /// Records that the system takes the world exclusively, so nothing else runs beside it.
    pub fn write_world(&mut self) {
        self.writes_all = true;
    }

    /// Records that the system queues changes, such as commands, that must be applied after it
    /// runs.
    pub fn set_needs_apply(&mut self) {
        self.needs_apply = true;
    }

    /// Returns whether the system queues changes that must be applied after it runs.
    pub fn needs_apply(&self) -> bool {
        self.needs_apply
    }

    /// Returns whether the system takes the world exclusively.
    pub fn is_exclusive(&self) -> bool {
        self.writes_all
    }

    /// Whether the system reads or writes the whole world.
    pub fn reads_world(&self) -> bool {
        self.reads_all || self.writes_all
    }

    /// Returns whether systems with access `a` and `b` can run at the same time: neither writes
    /// what the other reads or writes.
    pub fn are_disjoint(a: &Self, b: &Self) -> bool {
        if a.writes_all || b.writes_all {
            return false;
        }

        // Self reads all and other writes any
        if a.reads_all && (!b.component_writes.is_empty() || !b.resource_writes.is_empty()) {
            return false;
        }

        // Other reads all and self writes any
        if b.reads_all && (!a.component_writes.is_empty() || !a.resource_writes.is_empty()) {
            return false;
        }

        if (a.reads_all_components && !b.component_writes.is_empty())
            || (b.reads_all_components && !a.component_writes.is_empty())
        {
            return false;
        }

        // Self reads component and other writes
        if !a.component_reads.is_disjoint(&b.component_writes) {
            return false;
        }

        // Self writes component and other reads
        if !a.component_writes.is_disjoint(&b.component_reads) {
            return false;
        }

        // Both write component
        if !a.component_writes.is_disjoint(&b.component_writes) {
            return false;
        }

        // Self reads resource and other writes
        if !a.resource_reads.is_disjoint(&b.resource_writes) {
            return false;
        }

        // Self writes resource and other reads
        if !a.resource_writes.is_disjoint(&b.resource_reads) {
            return false;
        }

        // Both write resource
        if !a.resource_writes.is_disjoint(&b.resource_writes) {
            return false;
        }

        true
    }

    fn name<T: 'static>(&mut self) {
        self.names
            .insert(TypeId::of::<T>(), std::any::type_name::<T>());
    }

    fn named(&self, other: &Self, id: TypeId) -> &'static str {
        self.names
            .get(&id)
            .or_else(|| other.names.get(&id))
            .copied()
            .unwrap_or("an unnamed type")
    }

    /// The first conflict found between two parameters of the system, or inside one query.
    pub fn conflict(&self) -> Option<&str> {
        self.conflict.as_deref()
    }

    /// Records a conflict inside one parameter, such as a query that reads and writes one component.
    pub fn report_conflict(&mut self, conflict: String) {
        self.conflict.get_or_insert(conflict);
    }

    /// Whether `other`, the data of the same query, reads or writes a component this writes, or writes one this reads.
    pub fn data_conflict(&self, other: &Self) -> Option<&'static str> {
        self.component_writes
            .iter()
            .find(|id| other.component_reads.contains(id) || other.component_writes.contains(id))
            .or_else(|| {
                other
                    .component_writes
                    .iter()
                    .find(|id| self.component_reads.contains(id))
            })
            .map(|id| self.named(other, *id))
    }

    /// Adds one query's access: what its data reads and writes, what its filter reads, and its shape.
    pub fn add_query(&mut self, data: Self, filter: Self, shape: QueryShape) {
        let mut reads = data.component_reads.clone();
        reads.extend(filter.component_reads.iter().copied());
        self.queries.push(QueryAccess {
            reads,
            writes: data.component_writes.clone(),
            shape,
        });
        self.combine(data);
        self.combine(filter);
    }

    /// Adds the access of one more parameter of the same system, recording a conflict with the ones before it.
    pub fn add_parameter(&mut self, parameter: Self) {
        if self.conflict.is_none() {
            self.conflict = self.parameter_conflict(&parameter);
        }
        self.combine(parameter);
    }

    fn touches_anything(&self) -> bool {
        self.reads_all
            || self.writes_all
            || self.reads_all_components
            || !self.component_reads.is_empty()
            || !self.component_writes.is_empty()
            || !self.resource_reads.is_empty()
            || !self.resource_writes.is_empty()
    }

    fn writes_anything(&self) -> bool {
        self.writes_all || !self.component_writes.is_empty() || !self.resource_writes.is_empty()
    }

    fn as_queries(&self) -> Vec<QueryAccess> {
        if !self.queries.is_empty() {
            return self.queries.clone();
        }
        vec![QueryAccess {
            reads: self.component_reads.clone(),
            writes: self.component_writes.clone(),
            shape: QueryShape::default(),
        }]
    }

    fn parameter_conflict(&self, other: &Self) -> Option<String> {
        if (self.writes_all && other.touches_anything())
            || (other.writes_all && self.touches_anything())
        {
            return Some("exclusive access to the world beside another parameter".into());
        }
        if (self.reads_all && other.writes_anything())
            || (other.reads_all && self.writes_anything())
        {
            return Some("a read of the whole world beside a parameter that writes".into());
        }
        let resource = self
            .resource_writes
            .iter()
            .find(|id| other.resource_reads.contains(id) || other.resource_writes.contains(id))
            .or_else(|| {
                other
                    .resource_writes
                    .iter()
                    .find(|id| self.resource_reads.contains(id))
            });
        if let Some(id) = resource {
            return Some(format!(
                "two parameters access the resource `{}` and at least one writes it",
                self.named(other, *id)
            ));
        }
        if (self.reads_all_components && !other.component_writes.is_empty())
            || (other.reads_all_components && !self.component_writes.is_empty())
        {
            return Some("a read of every component beside a query that writes".into());
        }
        let theirs = other.as_queries();
        self.as_queries().iter().find_map(|mine| {
            theirs.iter().find_map(|their| {
                mine.conflict_with(their).map(|id| {
                    format!(
                        "two queries access the component `{}`, at least one writes it, and no `With`/`Without` pair keeps them apart",
                        self.named(other, id)
                    )
                })
            })
        })
    }

    /// Merges `other` into this access, keeping the first parameter conflict either recorded.
    pub fn combine(&mut self, other: Self) {
        self.queries.extend(other.queries);
        self.names.extend(other.names);
        if self.conflict.is_none() {
            self.conflict = other.conflict;
        }
        self.component_reads.extend(other.component_reads);
        self.component_writes.extend(other.component_writes);
        self.resource_reads.extend(other.resource_reads);
        self.resource_writes.extend(other.resource_writes);
        self.reads_all |= other.reads_all;
        self.reads_all_components |= other.reads_all_components;
        self.writes_all |= other.writes_all;
        self.needs_apply |= other.needs_apply;
    }
}
