use std::any::TypeId;

use crate::{
    System,
    system::{access::SystemAccess, meta::SystemMetadata},
    world::{UnsafeWorldCell, World},
};

pub(crate) struct StandIn {
    name: &'static str,
    id: TypeId,
}

impl StandIn {
    pub(crate) fn of(system: &dyn System<In = ()>) -> Self {
        Self {
            name: system.name(),
            id: system.system_type(),
        }
    }
}

impl System for StandIn {
    type In = ();

    fn name(&self) -> &'static str {
        self.name
    }

    fn system_type(&self) -> TypeId {
        self.id
    }

    fn initialize(&mut self, _world: &mut World) {}

    fn fill_access(&self, _meta: &mut SystemMetadata, _access: &mut SystemAccess) {}

    unsafe fn run_unsafe(&mut self, _args: Self::In, _world: UnsafeWorldCell) {}

    fn apply(&mut self, _world: &mut World) {}
}
