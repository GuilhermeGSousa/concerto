use concerto_ecs::{
    component::Component,
    query::Query,
    reflect::{ReflectComponent, ReflectSceneComponent},
    resource::{Res, ResMut},
};
use concerto_reflect::{Reflect, ReflectDefault};
use glam::UVec2;

use crate::{
    components::camera::RenderCamera, device::RenderDevice,
    render_asset::render_texture::RenderTexture, resources::RenderContext,
};

/// How many samples a camera's scene is drawn with per pixel, to smooth the edges of geometry.
///
/// Goes on the camera's entity. A camera without one uses [`Msaa::Sample4`].
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
#[reflect(Component, Default, SceneComponent)]
pub enum Msaa {
    /// One sample: no anti-aliasing.
    Off,
    /// Four samples, which every adapter supports.
    #[default]
    Sample4,
}

impl Msaa {
    /// The number of samples per pixel.
    pub fn samples(self) -> u32 {
        match self {
            Msaa::Off => 1,
            Msaa::Sample4 => 4,
        }
    }
}

/// One render pipeline for each sample count a view target can have.
pub struct ViewPipelines {
    single: wgpu::RenderPipeline,
    multisampled: wgpu::RenderPipeline,
}

impl ViewPipelines {
    /// Builds the pipelines with `build`, which is given the sample count of each.
    pub fn new(mut build: impl FnMut(u32) -> wgpu::RenderPipeline) -> Self {
        Self {
            single: build(Msaa::Off.samples()),
            multisampled: build(Msaa::Sample4.samples()),
        }
    }

    /// The pipeline that draws into a target of `samples` samples.
    pub fn get(&self, samples: u32) -> &wgpu::RenderPipeline {
        if samples > 1 {
            &self.multisampled
        } else {
            &self.single
        }
    }
}

/// The textures one camera's scene is drawn into: an HDR colour texture and a depth texture, both the size of the camera's target, multisampled under [`Msaa`], and the single-sampled HDR texture the colour resolves to.
pub struct ViewTarget {
    main: RenderTexture,
    depth: RenderTexture,
    multisampled: Option<wgpu::TextureView>,
    samples: u32,
}

fn attachment(
    device: &wgpu::Device,
    label: &'static str,
    format: wgpu::TextureFormat,
    size: UVec2,
    samples: u32,
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size.x,
            height: size.y,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: samples,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

impl ViewTarget {
    /// The format of every view target's depth texture.
    pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

    /// Creates the textures at `size`, at least one texel each way, in the formats `context` names.
    pub fn new(device: &wgpu::Device, context: &RenderContext, size: UVec2, msaa: Msaa) -> Self {
        let format = context.view_target_format;
        let size = size.max(UVec2::ONE);
        let samples = msaa.samples();
        let main = RenderTexture::camera_target(device, format, size.x, size.y);
        if samples == 1 {
            return Self {
                main,
                depth: RenderTexture::create_depth_texture(
                    device,
                    size.x,
                    size.y,
                    "View Target Depth",
                ),
                multisampled: None,
                samples,
            };
        }
        let depth = attachment(
            device,
            "View Target Multisampled Depth",
            Self::DEPTH_FORMAT,
            size,
            samples,
            if context.multisampled_depth_sampled {
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING
            } else {
                wgpu::TextureUsages::RENDER_ATTACHMENT
            },
        );
        let color = attachment(
            device,
            "View Target Multisampled Colour",
            format,
            size,
            samples,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        Self {
            depth: RenderTexture {
                view: depth.create_view(&wgpu::TextureViewDescriptor::default()),
                sampler: main.sampler.clone(),
                texture: depth,
            },
            multisampled: Some(color.create_view(&wgpu::TextureViewDescriptor::default())),
            main,
            samples,
        }
    }

    /// The number of samples per pixel the scene passes draw with.
    pub fn sample_count(&self) -> u32 {
        self.samples
    }

    /// The size of both textures in texels.
    pub fn size(&self) -> UVec2 {
        let size = self.main.texture.size();
        UVec2::new(size.width, size.height)
    }

    /// The single-sampled HDR colour texture, holding linear scene light before exposure and tonemapping; under [`Msaa`] it holds the scene only once the scene passes have been resolved, from [`RenderSet::PostProcess`](crate::sets::RenderSet::PostProcess) on.
    pub fn main_texture(&self) -> &RenderTexture {
        &self.main
    }

    /// The depth texture the scene passes test against and write, multisampled under [`Msaa`].
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
            view: self.multisampled.as_ref().unwrap_or(&self.main.view),
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
    mut render_cameras: Query<&mut RenderCamera>,
) {
    for mut render_camera in render_cameras.iter_mut() {
        let size = render_camera.target_size(&context).max(UVec2::ONE);
        let msaa = render_camera.msaa;
        let view_target = &render_camera.view_target;
        if view_target.size() != size || view_target.sample_count() != msaa.samples() {
            render_camera.view_target = ViewTarget::new(&device, &context, size, msaa);
        }
    }
}

pub(crate) fn resolve_view_targets(
    mut device: ResMut<RenderDevice>,
    render_cameras: Query<&RenderCamera>,
) {
    let encoder = device.command_encoder();
    for render_camera in render_cameras.iter() {
        let view_target = &render_camera.view_target;
        let Some(multisampled) = &view_target.multisampled else {
            continue;
        };
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            multiview_mask: None,
            label: Some("View Target Resolve Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                depth_slice: None,
                view: multisampled,
                resolve_target: Some(&view_target.main.view),
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Discard,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
        });
    }
}
