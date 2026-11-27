use concerto_app::{
    extractor::Extracted,
    plugins::Plugin,
    schedule_groups::{Extract, Render},
};
use concerto_ecs::{
    component::Component,
    entity::EntityHashMap,
    query::Query,
    reflect::{ReflectComponent, ReflectSceneComponent},
    resource::{Res, ResMut, Resource},
    Entity, IntoSystemConfig, With,
};
use concerto_foundation::time::Time;
use concerto_reflect::{Reflect, ReflectDefault};
use glam::UVec2;

use crate::{
    capabilities::RenderCapabilities,
    components::{
        camera::{Camera, RenderCamera},
        render_entity::RenderEntity,
    },
    device::RenderDevice,
    queue::RenderQueue,
    resources::RenderContext,
    sets::RenderSet,
    shader::create_shader_module,
};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;
const LEVEL_SIZES: [u32; 5] = [256, 64, 16, 4, 1];

/// Makes a camera's exposure follow the brightness of what it sees, as an eye does.
///
/// The scene's average is brought toward middle grey. Goes on the camera's
/// entity; [`Exposure`](crate::Exposure) still applies on top, as a
/// compensation.
#[derive(Component, Clone, Copy, Debug, PartialEq, Reflect)]
#[reflect(Component, Default, SceneComponent)]
pub struct AutoExposure {
    /// The furthest the exposure is brought down for a bright scene, in stops; at most 0.
    pub min_stops: f32,
    /// The furthest the exposure is brought up for a dark scene, in stops; at least 0.
    pub max_stops: f32,
    /// How quickly the exposure follows the scene: the share of the remaining difference closed each second is `1 - exp(-speed)`.
    pub speed: f32,
}

impl Default for AutoExposure {
    fn default() -> Self {
        Self {
            min_stops: -4.0,
            max_stops: 4.0,
            speed: 2.0,
        }
    }
}

impl AutoExposure {
    /// Sets how far the exposure may be brought down and up, in stops.
    pub fn with_range(mut self, min_stops: f32, max_stops: f32) -> Self {
        self.min_stops = min_stops;
        self.max_stops = max_stops;
        self
    }

    /// Sets how quickly the exposure follows the scene.
    pub fn with_speed(mut self, speed: f32) -> Self {
        self.speed = speed;
        self
    }
}

/// Measures and adapts the exposure of cameras with [`AutoExposure`], in [`RenderSet::PostProcess`].
///
/// # Panics
///
/// Panics during [`finish`](Plugin::finish) if [`RenderPlugin`](crate::plugin::RenderPlugin)
/// was not registered before it.
pub struct AutoExposurePlugin;

impl Plugin for AutoExposurePlugin {
    fn build(&self, app: &mut concerto_app::App) {
        app.register_type::<AutoExposure>();
        app.render_mut()
            .insert_resource(ExtractedAutoExposure::default())
            .insert_resource(AutoExposureViews::default());
        app.add_render_system(Extract, extract_auto_exposure)
            .add_render_system(Render, adapt_exposure.in_set(RenderSet::PostProcess));
    }

    fn finish(&self, app: &mut concerto_app::App) {
        let render = app.render();
        let device = render
            .get_resource::<RenderDevice>()
            .expect("AutoExposurePlugin needs RenderPlugin, registered before it");
        let capabilities = render
            .get_resource::<RenderCapabilities>()
            .expect("AutoExposurePlugin needs RenderPlugin, registered before it");
        if !capabilities.float_render_targets {
            log::warn!(
                "auto exposure needs float render targets, which this adapter lacks; \
                 AutoExposure has no effect"
            );
            return;
        }
        let pipelines = AutoExposurePipelines::new(device, capabilities);
        app.render_mut().insert_resource(pipelines);
    }
}

#[derive(Resource, Default)]
pub(crate) struct ExtractedAutoExposure {
    cameras: EntityHashMap<AutoExposure>,
    seconds: f32,
}

pub(crate) fn extract_auto_exposure(
    cameras: Extracted<Query<(&AutoExposure, &RenderEntity), With<Camera>>>,
    time: Extracted<Res<Time>>,
    mut extracted: ResMut<ExtractedAutoExposure>,
) {
    extracted.cameras = cameras
        .iter()
        .map(|(exposure, render_entity)| (**render_entity, *exposure))
        .collect();
    extracted.seconds = time.delta().as_secs_f32();
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ExposureUniform {
    rect: [f32; 4],
    min_stops: f32,
    max_stops: f32,
    _padding: [f32; 2],
}

#[derive(Resource)]
pub(crate) struct AutoExposurePipelines {
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    measure: wgpu::RenderPipeline,
    reduce: wgpu::RenderPipeline,
    adapt: wgpu::RenderPipeline,
}

impl AutoExposurePipelines {
    fn new(device: &wgpu::Device, capabilities: &RenderCapabilities) -> Self {
        let module = create_shader_module(
            device,
            "Auto Exposure",
            include_str!("shaders/auto_exposure.wgsl"),
            &capabilities.shader_defs(),
        );
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Auto Exposure Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Auto Exposure Pipeline Layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |label, entry_point, blend| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(entry_point),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: FORMAT,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let toward = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::Constant,
            dst_factor: wgpu::BlendFactor::OneMinusConstant,
            operation: wgpu::BlendOperation::Add,
        };
        Self {
            measure: pipeline("Auto Exposure Measure", "fs_measure", None),
            reduce: pipeline("Auto Exposure Reduce", "fs_reduce", None),
            adapt: pipeline(
                "Auto Exposure Adapt",
                "fs_adapt",
                Some(wgpu::BlendState {
                    color: toward,
                    alpha: toward,
                }),
            ),
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("Auto Exposure Sampler"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            layout,
        }
    }

    fn bind_group(
        &self,
        device: &wgpu::Device,
        source: &wgpu::TextureView,
        uniform: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Auto Exposure Bind Group"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform.as_entire_binding(),
                },
            ],
        })
    }
}

struct ExposureLevel {
    view: wgpu::TextureView,
    source: wgpu::BindGroup,
}

struct AutoExposureView {
    uniform: wgpu::Buffer,
    levels: Vec<ExposureLevel>,
    adapted: wgpu::TextureView,
    settled: bool,
}

fn level_view(device: &wgpu::Device, size: u32) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("Auto Exposure Level"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

impl AutoExposureView {
    fn new(device: &wgpu::Device, pipelines: &AutoExposurePipelines) -> Self {
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Auto Exposure Uniform"),
            size: std::mem::size_of::<ExposureUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let levels = LEVEL_SIZES
            .iter()
            .map(|&size| {
                let view = level_view(device, size);
                let source = pipelines.bind_group(device, &view, &uniform);
                ExposureLevel { view, source }
            })
            .collect();
        Self {
            uniform,
            levels,
            adapted: level_view(device, 1),
            settled: false,
        }
    }
}

#[derive(Resource, Default)]
pub(crate) struct AutoExposureViews {
    cameras: EntityHashMap<AutoExposureView>,
}

struct ExposurePass<'a> {
    label: &'static str,
    pipeline: &'a wgpu::RenderPipeline,
    source: &'a wgpu::BindGroup,
    target: &'a wgpu::TextureView,
    toward: f64,
}

impl ExposurePass<'_> {
    fn draw(&self, encoder: &mut wgpu::CommandEncoder) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            multiview_mask: None,
            label: Some(self.label),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                depth_slice: None,
                view: self.target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
        });
        pass.set_pipeline(self.pipeline);
        pass.set_bind_group(0, self.source, &[]);
        pass.set_blend_constant(wgpu::Color {
            r: self.toward,
            g: self.toward,
            b: self.toward,
            a: self.toward,
        });
        pass.draw(0..3, 0..1);
    }
}

pub(crate) fn adapt_exposure(
    pipelines: Option<Res<AutoExposurePipelines>>,
    extracted: Res<ExtractedAutoExposure>,
    mut views: ResMut<AutoExposureViews>,
    mut device: ResMut<RenderDevice>,
    queue: Res<RenderQueue>,
    mut render_cameras: Query<(Entity, &mut RenderCamera)>,
    context: Res<RenderContext>,
) {
    let Some(pipelines) = pipelines else {
        return;
    };
    views
        .cameras
        .retain(|camera, _| extracted.cameras.contains_key(camera));

    for (camera, mut render_camera) in render_cameras.iter_mut() {
        let Some(settings) = extracted.cameras.get(&camera) else {
            if render_camera.adapted_exposure.is_some() {
                render_camera.adapted_exposure = None;
            }
            continue;
        };
        let target_size = render_camera.target_size(&context).max(UVec2::ONE);
        let rect = render_camera.viewport_rect(target_size);
        if rect.is_empty() {
            continue;
        }
        let view = views
            .cameras
            .entry(camera)
            .or_insert_with(|| AutoExposureView::new(&device, &pipelines));
        if render_camera.adapted_exposure.is_none() {
            render_camera.adapted_exposure = Some(view.adapted.clone());
        }

        let size = target_size.as_vec2();
        queue.write_buffer(
            &view.uniform,
            0,
            bytemuck::bytes_of(&ExposureUniform {
                rect: [
                    rect.x as f32 / size.x,
                    rect.y as f32 / size.y,
                    rect.width as f32 / size.x,
                    rect.height as f32 / size.y,
                ],
                min_stops: settings.min_stops.min(0.0),
                max_stops: settings.max_stops.max(0.0),
                _padding: [0.0; 2],
            }),
        );
        let scene = pipelines.bind_group(
            &device,
            &render_camera.view_target.main_texture().view,
            &view.uniform,
        );
        let toward = if view.settled {
            1.0 - (-extracted.seconds * settings.speed.max(0.0)).exp()
        } else {
            1.0
        };
        view.settled = true;

        let encoder = device.command_encoder();
        ExposurePass {
            label: "Auto Exposure Measure Pass",
            pipeline: &pipelines.measure,
            source: &scene,
            target: &view.levels[0].view,
            toward: 0.0,
        }
        .draw(encoder);
        for pair in view.levels.windows(2) {
            ExposurePass {
                label: "Auto Exposure Reduce Pass",
                pipeline: &pipelines.reduce,
                source: &pair[0].source,
                target: &pair[1].view,
                toward: 0.0,
            }
            .draw(encoder);
        }
        ExposurePass {
            label: "Auto Exposure Adapt Pass",
            pipeline: &pipelines.adapt,
            source: &view.levels[LEVEL_SIZES.len() - 1].source,
            target: &view.adapted,
            toward: f64::from(toward),
        }
        .draw(encoder);
    }
}
