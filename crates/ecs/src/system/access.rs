use std::{any::TypeId, collections::HashSet};

use crate::{component::Component, resource::Resource};

#[derive(Default, Clone)]
pub struct SystemAccess {
    component_reads: HashSet<TypeId>,
    component_writes: HashSet<TypeId>,
    resource_reads: HashSet<TypeId>,
    resource_writes: HashSet<TypeId>,
    reads_all: bool,
    writes_all: bool,
    needs_apply: bool,
}

impl SystemAccess {
    pub fn read_component<T: Component>(&mut self) {
        self.component_reads.insert(TypeId::of::<T>());
    }

    pub fn write_component<T: Component>(&mut self) {
        self.component_writes.insert(TypeId::of::<T>());
    }

    pub fn read_resource<T: Resource>(&mut self) {
        self.resource_reads.insert(TypeId::of::<T>());
    }

    pub fn write_resource<T: Resource>(&mut self) {
        self.resource_writes.insert(TypeId::of::<T>());
    }

    pub fn read_world(&mut self) {
        self.reads_all = true;
    }

    pub fn write_world(&mut self) {
        self.writes_all = true;
    }

    pub fn set_needs_apply(&mut self) {
        self.needs_apply = true;
    }

    pub fn needs_apply(&self) -> bool {
        self.needs_apply
    }

    pub fn is_exclusive(&self) -> bool {
        self.writes_all
    }

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

    pub fn combine(&mut self, other: Self) {
        self.component_reads.extend(other.component_reads);
        self.component_writes.extend(other.component_writes);
        self.resource_reads.extend(other.resource_reads);
        self.resource_writes.extend(other.resource_writes);
        self.reads_all |= other.reads_all;
        self.writes_all |= other.writes_all;
        self.needs_apply |= other.needs_apply;
    }
}
