use std::fmt;

use petgraph::graph::DiGraph;

use crate::system::{access::SystemAccess, meta::SystemMetadata, schedule::SystemIndex};

pub(crate) type SystemDependencyGraph = DiGraph<SystemNode, ()>;

pub struct SystemNode {
    system_index: SystemIndex,
    cached_access: SystemAccess,
    cached_metadata: SystemMetadata,
    pub(crate) name: &'static str,
}

impl SystemNode {
    pub(crate) fn new(
        system_index: SystemIndex,
        access: SystemAccess,
        metadata: SystemMetadata,
        name: &'static str,
    ) -> Self {
        Self {
            system_index,
            cached_access: access,
            cached_metadata: metadata,
            name,
        }
    }

    pub(crate) fn access(&self) -> &SystemAccess {
        &self.cached_access
    }

    pub(crate) fn meta(&self) -> &SystemMetadata {
        &self.cached_metadata
    }

    pub(crate) fn index(&self) -> SystemIndex {
        self.system_index
    }
}

impl fmt::Debug for SystemNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name)
    }
}
