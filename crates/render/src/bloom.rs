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
    render_asset::render_texture::RenderTexture,
    resources::RenderContext,
    sets::{PostProcessSet, RenderSet},
    shader::create_shader_module,
};

const MAX_LEVELS: usize = 7;
const MIN_LEVEL_SIZE: u32 = 4;

/// Glow around the bright parts of a camera's scene.
///
/// Goes on the camera's entity. A camera without one has none.
#[derive(Component, Clone, Copy, Debug, PartialEq, Reflect)]
#[reflect(Component, Default, SceneComponent)]
pub struct Bloom {
    /// How much of the blurred scene is shown: 0 is none and leaves the frame as it was.
    pub intensity: f32,
    /// Scene light a texel must exceed to glow. At 0 everything glows a little and the blur is mixed into the scene, keeping its brightness; above 0 the glow is added on top.
    pub threshold: f32,
}

impl Default for Bloom {
    fn default() -> Self {
        Self {
            intensity: 0.15,
            threshold: 0.0,
        }
    }
}

impl Bloom {
    /// Sets how much of the blurred scene is shown.
    pub fn with_intensity(mut self, intensity: f32) -> Self {
        self.intensity = intensity;
        self
    }

    /// Sets the scene light a texel must exceed to glow.
    pub fn with_threshold(mut self, threshold: f32) -> Self {
        self.threshold = threshold;
        self
    }
}

/// Draws [`Bloom`] for the cameras that have it, in [`RenderSet::PostProcess`].
///
/// # Panics
///
/// Panics during [`finish`](Plugin::finish) if [`RenderPlugin`](crate::plugin::RenderPlugin)
/// was not registered before it.
pub struct BloomPlugin;

impl Plugin for BloomPlugin {
    fn build(&self, app: &mut concerto_app::App) {
        app.register_type::<Bloom>();
        app.render_mut()
            .insert_resource(ExtractedBloom::default())
            .insert_resource(BloomChains::default());
        app.add_render_system(Extract, extract_bloom)
            .add_render_system(
                Render,
                render_bloom
                    .in_set(RenderSet::PostProcess)
                    .in_set(PostProcessSet::Bloom),
            );
    }

    fn finish(&self, app: &mut concerto_app::App) {
        let render = app.render();
        let device = render
            .get_resource::<RenderDevice>()
            .expect("BloomPlugin needs RenderPlugin, registered before it");
        let capabilities = render
            .get_resource::<RenderCapabilities>()
            .expect("BloomPlugin needs RenderPlugin, registered before it");
        let format = render
            .get_resource::<RenderContext>()
            .expect("BloomPlugin needs RenderPlugin, registered before it")
            .view_target_format;
        let pipelines = BloomPipelines::new(device, capabilities, format);
        app.render_mut().insert_resource(pipelines);
    }
}

#[derive(Resource, Default)]
pub(crate) struct ExtractedBloom {
    cameras: EntityHashMap<Bloom>,
}

pub(crate) fn extract_bloom(
    cameras: Extracted<Query<(&Bloom, &RenderEntity), With<Camera>>>,
    mut extracted: ResMut<ExtractedBloom>,
) {
    extracted.cameras = cameras
        .iter()
        .map(|(bloom, render_entity)| (**render_entity, *bloom))
        .collect();
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BloomUniform {
    threshold: f32,
    knee: f32,
    _padding: [f32; 2],
}

#[derive(Resource)]
pub(crate) struct BloomPipelines {
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    format: wgpu::TextureFormat,
    downsample_first: wgpu::RenderPipeline,
    downsample: wgpu::RenderPipeline,
    upsample: wgpu::RenderPipeline,
    mix: wgpu::RenderPipeline,
    add: wgpu::RenderPipeline,
}

struct BloomPipeline<'a> {
    label: &'static str,
    entry_point: &'static str,
    blend: Option<wgpu::BlendState>,
    write_mask: wgpu::ColorWrites,
    device: &'a wgpu::Device,
    module: &'a wgpu::ShaderModule,
    layout: &'a wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
}

impl BloomPipeline<'_> {
    fn build(&self) -> wgpu::RenderPipeline {
        self.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(self.label),
                layout: Some(self.layout),
                vertex: wgpu::VertexState {
                    module: self.module,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: self.module,
                    entry_point: Some(self.entry_point),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: self.format,
                        blend: self.blend,
                        write_mask: self.write_mask,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
    }
}

fn constant_blend(destination: wgpu::BlendFactor) -> wgpu::BlendState {
    let component = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::Constant,
        dst_factor: destination,
        operation: wgpu::BlendOperation::Add,
    };
    wgpu::BlendState {
        color: component,
        alpha: component,
    }
}

impl BloomPipelines {
    fn new(
        device: &wgpu::Device,
        capabilities: &RenderCapabilities,
        format: wgpu::TextureFormat,
    ) -> Self {
        let module = create_shader_module(
            device,
            "Bloom",
            include_str!("shaders/bloom.wgsl"),
            &capabilities.shader_defs(),
        );
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Bloom Layout"),
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
            label: Some("Bloom Pipeline Layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Bloom Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let pipeline = |label, entry_point, blend, write_mask| {
            BloomPipeline {
                label,
                entry_point,
                blend,
                write_mask,
                device,
                module: &module,
                layout: &pipeline_layout,
                format,
            }
            .build()
        };
        let all = wgpu::ColorWrites::ALL;
        let color = wgpu::ColorWrites::COLOR;
        Self {
            downsample_first: pipeline("Bloom First Downsample", "fs_downsample_first", None, all),
            downsample: pipeline("Bloom Downsample", "fs_downsample", None, all),
            upsample: pipeline(
                "Bloom Upsample",
                "fs_upsample",
                Some(constant_blend(wgpu::BlendFactor::OneMinusConstant)),
                all,
            ),
            mix: pipeline(
                "Bloom Mix",
                "fs_upsample",
                Some(constant_blend(wgpu::BlendFactor::OneMinusConstant)),
                color,
            ),
            add: pipeline(
                "Bloom Add",
                "fs_upsample",
                Some(constant_blend(wgpu::BlendFactor::One)),
                color,
            ),
            layout,
            sampler,
            format,
        }
    }

    fn bind_group(
        &self,
        device: &wgpu::Device,
        source: &wgpu::TextureView,
        uniform: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Bloom Bind Group"),
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

struct BloomLevel {
    texture: RenderTexture,
    bind_group: wgpu::BindGroup,
}

struct BloomChain {
    source_size: UVec2,
    uniform: wgpu::Buffer,
    levels: Vec<BloomLevel>,
}

fn bloom_level_sizes(source: UVec2) -> Vec<UVec2> {
    let mut sizes = Vec::new();
    let mut size = (source / 2).max(UVec2::ONE);
    loop {
        sizes.push(size);
        let next = size / 2;
        if sizes.len() == MAX_LEVELS || next.min_element() < MIN_LEVEL_SIZE {
            return sizes;
        }
        size = next;
    }
}

impl BloomChain {
    fn new(device: &wgpu::Device, pipelines: &BloomPipelines, source_size: UVec2) -> Self {
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Bloom Uniform"),
            size: std::mem::size_of::<BloomUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let levels = bloom_level_sizes(source_size)
            .into_iter()
            .map(|size| {
                let texture =
                    RenderTexture::camera_target(device, pipelines.format, size.x, size.y);
                let bind_group = pipelines.bind_group(device, &texture.view, &uniform);
                BloomLevel {
                    texture,
                    bind_group,
                }
            })
            .collect();
        Self {
            source_size,
            uniform,
            levels,
        }
    }
}

#[derive(Resource, Default)]
pub(crate) struct BloomChains {
    cameras: EntityHashMap<BloomChain>,
}

struct BloomPass<'a> {
    label: &'static str,
    pipeline: &'a wgpu::RenderPipeline,
    source: &'a wgpu::BindGroup,
    target: &'a wgpu::TextureView,
    load: wgpu::LoadOp<wgpu::Color>,
    blend_constant: f64,
}

impl BloomPass<'_> {
    fn draw(&self, encoder: &mut wgpu::CommandEncoder) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            multiview_mask: None,
            label: Some(self.label),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                depth_slice: None,
                view: self.target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: self.load,
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
            r: self.blend_constant,
            g: self.blend_constant,
            b: self.blend_constant,
            a: self.blend_constant,
        });
        pass.draw(0..3, 0..1);
    }
}

pub(crate) fn render_bloom(
    pipelines: Res<BloomPipelines>,
    extracted: Res<ExtractedBloom>,
    mut chains: ResMut<BloomChains>,
    mut device: ResMut<RenderDevice>,
    queue: Res<RenderQueue>,
    render_cameras: Query<(Entity, &RenderCamera)>,
) {
    chains
        .cameras
        .retain(|camera, _| extracted.cameras.contains_key(camera));

    for (camera, render_camera) in render_cameras.iter() {
        let Some(bloom) = extracted
            .cameras
            .get(&camera)
            .filter(|bloom| bloom.intensity > 0.0)
        else {
            continue;
        };
        let view_target = &render_camera.view_target;
        let source_size = view_target.size();
        if chains
            .cameras
            .get(&camera)
            .is_none_or(|chain| chain.source_size != source_size)
        {
            chains
                .cameras
                .insert(camera, BloomChain::new(&device, &pipelines, source_size));
        }
        let chain = &chains.cameras[&camera];

        let threshold = bloom.threshold.max(0.0);
        queue.write_buffer(
            &chain.uniform,
            0,
            bytemuck::bytes_of(&BloomUniform {
                threshold,
                knee: threshold * 0.5,
                _padding: [0.0; 2],
            }),
        );
        let scene = pipelines.bind_group(&device, &view_target.main_texture().view, &chain.uniform);

        let encoder = device.command_encoder();
        let clear = wgpu::LoadOp::Clear(wgpu::Color::BLACK);
        BloomPass {
            label: "Bloom First Downsample Pass",
            pipeline: &pipelines.downsample_first,
            source: &scene,
            target: &chain.levels[0].texture.view,
            load: clear,
            blend_constant: 0.0,
        }
        .draw(encoder);
        for pair in chain.levels.windows(2) {
            BloomPass {
                label: "Bloom Downsample Pass",
                pipeline: &pipelines.downsample,
                source: &pair[0].bind_group,
                target: &pair[1].texture.view,
                load: clear,
                blend_constant: 0.0,
            }
            .draw(encoder);
        }
        for (blurred, pair) in chain.levels.windows(2).rev().enumerate() {
            let blurred_levels = (blurred + 1) as f64;
            BloomPass {
                label: "Bloom Upsample Pass",
                pipeline: &pipelines.upsample,
                source: &pair[1].bind_group,
                target: &pair[0].texture.view,
                load: wgpu::LoadOp::Load,
                blend_constant: blurred_levels / (blurred_levels + 1.0),
            }
            .draw(encoder);
        }
        BloomPass {
            label: "Bloom Composite Pass",
            pipeline: if threshold > 0.0 {
                &pipelines.add
            } else {
                &pipelines.mix
            },
            source: &chain.levels[0].bind_group,
            target: &view_target.main_texture().view,
            load: wgpu::LoadOp::Load,
            blend_constant: f64::from(bloom.intensity.min(1.0)),
        }
        .draw(encoder);
    }
}
