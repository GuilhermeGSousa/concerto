use concerto_ecs::{query::Query, resource::Res};
use glam::UVec2;

use crate::{
    components::camera::{create_rtt, RenderCamera},
    device::RenderDevice,
    render_asset::render_texture::RenderTexture,
    resources::RenderContext,
};

/// The textures one camera's scene is drawn into: an HDR colour texture and a depth texture, both the size of the camera's target.
pub struct ViewTarget {
    main: RenderTexture,
    depth: RenderTexture,
}

impl ViewTarget {
    /// The format of every view target's depth texture.
    pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

    /// Creates the textures at `size`, at least one texel each way, with a colour texture of `format`.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, size: UVec2) -> Self {
        let size = size.max(UVec2::ONE);
        Self {
            main: create_rtt(device, format, size.x, size.y),
            depth: RenderTexture::create_depth_texture(device, size.x, size.y, "View Target Depth"),
        }
    }

    /// The size of both textures in texels.
    pub fn size(&self) -> UVec2 {
        let size = self.main.texture.size();
        UVec2::new(size.width, size.height)
    }

    /// The HDR colour texture, holding linear scene light before exposure and tonemapping.
    pub fn main_texture(&self) -> &RenderTexture {
        &self.main
    }

    /// The depth texture the scene passes test against and write.
    pub fn depth_texture(&self) -> &RenderTexture {
        &self.depth
    }

    /// The colour attachment of a pass that draws into the scene.
    pub fn color_attachment(
        &self,
        load: wgpu::LoadOp<wgpu::Color>,
    ) -> wgpu::RenderPassColorAttachment<'_> {
        wgpu::RenderPassColorAttachment {
            depth_slice: None,
            view: &self.main.view,
            resolve_target: None,
            ops: wgpu::Operations {
                load,
                store: wgpu::StoreOp::Store,
            },
        }
    }

    /// The depth attachment of a pass that draws into the scene.
    pub fn depth_attachment(
        &self,
        load: wgpu::LoadOp<f32>,
    ) -> wgpu::RenderPassDepthStencilAttachment<'_> {
        wgpu::RenderPassDepthStencilAttachment {
            view: &self.depth.view,
            depth_ops: Some(wgpu::Operations {
                load,
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }
    }
}

pub(crate) fn prepare_view_targets(
    device: Res<RenderDevice>,
    context: Res<RenderContext>,
    render_cameras: Query<&mut RenderCamera>,
) {
    for mut render_camera in render_cameras.iter() {
        let size = render_camera.target_size(&context).max(UVec2::ONE);
        if render_camera.view_target.size() != size {
            render_camera.view_target = ViewTarget::new(&device, context.view_target_format, size);
        }
    }
}
