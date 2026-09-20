use ecs::resource::Resource;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

/// Shared main/render-world counters used by the showcase and profilers.
#[derive(Resource, Clone, Default)]
pub struct UIRenderDiagnostics {
    geometry_rebuilds: Arc<AtomicU64>,
    text_reshapes: Arc<AtomicU64>,
    binding_rebuilds: Arc<AtomicU64>,
}

impl UIRenderDiagnostics {
    pub fn geometry_rebuilds(&self) -> u64 {
        self.geometry_rebuilds.load(Ordering::Relaxed)
    }

    pub fn text_reshapes(&self) -> u64 {
        self.text_reshapes.load(Ordering::Relaxed)
    }

    pub fn binding_rebuilds(&self) -> u64 {
        self.binding_rebuilds.load(Ordering::Relaxed)
    }

    pub(crate) fn record_geometry_rebuild(&self) {
        self.geometry_rebuilds.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_text_reshape(&self) {
        self.text_reshapes.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_binding_rebuild(&self) {
        self.binding_rebuilds.fetch_add(1, Ordering::Relaxed);
    }
}
