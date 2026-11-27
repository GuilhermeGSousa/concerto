use std::ops::Deref;

use concerto_ecs::resource::Resource;
use wgpu::{CommandEncoder, CommandEncoderDescriptor};

use crate::queue::RenderQueue;

/// The render world's wgpu device and the command encoder its systems record into this frame.
///
/// It derefs to the [`wgpu::Device`].
#[derive(Resource)]
pub struct RenderDevice {
    pub(crate) device: wgpu::Device,
    pub(crate) encoder: Option<CommandEncoder>,
}

impl RenderDevice {
    /// Returns the frame's command encoder, creating it on first use.
    pub fn command_encoder(&mut self) -> &mut CommandEncoder {
        self.encoder.get_or_insert_with(|| {
            self.device
                .create_command_encoder(&CommandEncoderDescriptor::default())
        })
    }

    /// Submits the frame's command encoder to `queue`, if anything created one.
    pub fn finish(&mut self, queue: &RenderQueue) {
        if let Some(encoder) = self.encoder.take() {
            queue.submit(std::iter::once(encoder.finish()));
        }
    }

    /// Calls `f` with the device and the frame's command encoder, creating the encoder if there is
    /// none.
    pub fn scoped_encoder(&mut self, f: impl FnOnce(&wgpu::Device, &mut CommandEncoder)) {
        let mut encoder = self.encoder.take().unwrap_or(
            self.device
                .create_command_encoder(&CommandEncoderDescriptor::default()),
        );
        f(self, &mut encoder);

        self.encoder = Some(encoder);
    }
}

impl Deref for RenderDevice {
    type Target = wgpu::Device;

    fn deref(&self) -> &Self::Target {
        &self.device
    }
}
