use std::ops::Deref;

use concerto_ecs::resource::Resource;

/// The render world's wgpu queue; it derefs to the [`wgpu::Queue`].
#[derive(Resource)]
pub struct RenderQueue {
    pub(crate) queue: wgpu::Queue,
}

impl Deref for RenderQueue {
    type Target = wgpu::Queue;

    fn deref(&self) -> &Self::Target {
        &self.queue
    }
}
