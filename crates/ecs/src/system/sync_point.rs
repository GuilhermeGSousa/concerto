use crate::{
    System,
    system::{access::SystemAccess, meta::SystemMetadata},
};

pub(crate) struct SyncPoint;

impl System for SyncPoint {
    fn name(&self) -> &'static str {
        "SyncPoint"
    }

    fn initialize(&mut self, _world: &mut crate::World) {}

    fn fill_access(&self, _meta: &mut SystemMetadata, access: &mut SystemAccess) {
        access.write_world();
    }

    unsafe fn run_unsafe(&mut self, _world: crate::world::UnsafeWorldCell) {}

    fn apply(&mut self, _world: &mut crate::World) {}
}
