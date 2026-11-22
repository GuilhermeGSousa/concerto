use std::ops::Range;

use concerto_app::{extractor::Extracted, subapp::SubApp};
use concerto_ecs::{
    entity::EntityHashMap,
    query::Query,
    resource::{Res, ResMut, Resource},
    Entity,
};
use concerto_foundation::transform::GlobalTransform;
use concerto_render::{
    capabilities::RenderCapabilities,
    components::{camera::RenderCamera, visibility::RenderLayers},
    device::RenderDevice,
    layouts::{CameraLayout, LightingLayout},
    queue::RenderQueue,
    resources::RenderContext,
    shader::create_shader_module,
    view_phase::{PhaseBindings, PhaseInstances, PhaseItem, PhaseMesh, ViewPhases},
    wgpu::{self, util::DeviceExt},
    ViewPipelines, ViewTarget,
};
use glam::Vec3;

use crate::{emitter::ParticleEmitter, simulation::Particles, PARTICLE_SHADER};

const INITIAL_INSTANCE_CAPACITY: u64 = 1024;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ParticleInstance {
    position_size: [f32; 4],
    color: [f32; 4],
    params: [f32; 4],
}

struct ExtractedEmitter {
    instances: Range<u32>,
    center: Vec3,
    layers: RenderLayers,
}

#[derive(Resource, Default)]
pub(crate) struct ExtractedParticles {
    emitters: Vec<ExtractedEmitter>,
    instances: Vec<ParticleInstance>,
}

pub(crate) fn extract_particles(
    emitters: Extracted<
        Query<(
            &ParticleEmitter,
            &Particles,
            &GlobalTransform,
            Option<&RenderLayers>,
        )>,
    >,
    mut extracted: ResMut<ExtractedParticles>,
) {
    let extracted = &mut *extracted;
    extracted.emitters.clear();
    extracted.instances.clear();
    for (emitter, particles, transform, layers) in emitters.iter() {
        if particles.alive().is_empty() {
            continue;
        }
        let first = extracted.instances.len() as u32;
        let start = emitter.start_color.to_linear();
        let end = emitter.end_color.to_linear();
        let lifetime = emitter.lifetime.max(f32::EPSILON);
        extracted
            .instances
            .extend(particles.alive().iter().map(|particle| {
                let age = (particle.age / lifetime).clamp(0.0, 1.0);
                let mix = |from: f32, to: f32| from + (to - from) * age;
                let alpha = mix(start.a, end.a);
                ParticleInstance {
                    position_size: particle
                        .position
                        .extend(mix(emitter.start_size, emitter.end_size))
                        .to_array(),
                    color: [
                        mix(start.r, end.r) * alpha,
                        mix(start.g, end.g) * alpha,
                        mix(start.b, end.b) * alpha,
                        if emitter.additive { 0.0 } else { alpha },
                    ],
                    params: [if emitter.lit { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0],
                }
            }));
        extracted.emitters.push(ExtractedEmitter {
            instances: first..extracted.instances.len() as u32,
            center: transform.translation(),
            layers: layers.copied().unwrap_or_default(),
        });
    }
}

struct ParticleView {
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

#[derive(Resource)]
pub(crate) struct ParticleDraw {
    pipelines: ViewPipelines,
    billboard_layout: wgpu::BindGroupLayout,
    corners: wgpu::Buffer,
    indices: wgpu::Buffer,
    instances: wgpu::Buffer,
    views: EntityHashMap<ParticleView>,
}

fn instance_buffer(device: &wgpu::Device, capacity: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Particle Instances"),
        size: capacity * std::mem::size_of::<ParticleInstance>() as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

impl ParticleDraw {
    pub(crate) fn new(render: &SubApp) -> Self {
        let needs = "ParticlesPlugin needs RenderPlugin, registered before it";
        let device = render.get_resource::<RenderDevice>().expect(needs);
        let capabilities = render.get_resource::<RenderCapabilities>().expect(needs);
        let camera_layout = render.get_resource::<CameraLayout>().expect(needs);
        let lighting_layout = render.get_resource::<LightingLayout>().expect(needs);
        let format = render
            .get_resource::<RenderContext>()
            .expect(needs)
            .view_target_format;

        let module = create_shader_module(
            device,
            "Particles",
            PARTICLE_SHADER,
            &capabilities.shader_defs(),
        );
        let billboard_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Particle Billboard Layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Particle Pipeline Layout"),
            bind_group_layouts: &[
                Some(&billboard_layout),
                Some(&camera_layout.camera_layout),
                Some(lighting_layout),
            ],
            immediate_size: 0,
        });
        let premultiplied = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        };
        let pipelines = ViewPipelines::new(|samples| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Particle Pipeline"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_main"),
                    buffers: &[
                        Some(wgpu::VertexBufferLayout {
                            array_stride: 8,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &wgpu::vertex_attr_array![0 => Float32x2],
                        }),
                        Some(wgpu::VertexBufferLayout {
                            array_stride: std::mem::size_of::<ParticleInstance>() as u64,
                            step_mode: wgpu::VertexStepMode::Instance,
                            attributes: &wgpu::vertex_attr_array![
                                1 => Float32x4,
                                2 => Float32x4,
                                3 => Float32x4
                            ],
                        }),
                    ],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState {
                            color: premultiplied,
                            alpha: premultiplied,
                        }),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: ViewTarget::DEPTH_FORMAT,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: samples,
                    ..Default::default()
                },
                multiview_mask: None,
                cache: None,
            })
        });
        let corners: [[f32; 2]; 4] = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];
        let indices: [u32; 6] = [0, 1, 2, 0, 2, 3];
        Self {
            pipelines,
            billboard_layout,
            corners: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Particle Corners"),
                contents: bytemuck::cast_slice(&corners),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Particle Indices"),
                contents: bytemuck::cast_slice(&indices),
                usage: wgpu::BufferUsages::INDEX,
            }),
            instances: instance_buffer(device, INITIAL_INSTANCE_CAPACITY),
            views: EntityHashMap::default(),
        }
    }

    fn view(&mut self, device: &wgpu::Device, camera: Entity) -> &ParticleView {
        let layout = &self.billboard_layout;
        self.views.entry(camera).or_insert_with(|| {
            let uniform = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Particle Billboard"),
                size: 32,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Particle Billboard Bind Group"),
                layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                }],
            });
            ParticleView {
                uniform,
                bind_group,
            }
        })
    }
}

pub(crate) fn queue_particles(
    extracted: Res<ExtractedParticles>,
    mut draw: ResMut<ParticleDraw>,
    mut phases: ResMut<ViewPhases>,
    render_cameras: Query<(Entity, &RenderCamera)>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    draw.views
        .retain(|camera, _| render_cameras.contains_entity(*camera));
    if extracted.instances.is_empty() {
        return;
    }
    let stride = std::mem::size_of::<ParticleInstance>() as u64;
    let needed = extracted.instances.len() as u64;
    let mut capacity = draw.instances.size() / stride;
    if needed > capacity {
        while capacity < needed {
            capacity *= 2;
        }
        draw.instances = instance_buffer(&device, capacity);
    }
    queue.write_buffer(
        &draw.instances,
        0,
        bytemuck::cast_slice(&extracted.instances),
    );

    for (camera, render_camera) in render_cameras.iter() {
        let world_from_clip = render_camera.camera_uniform.inverse_view_proj();
        let at = |x: f32, y: f32| world_from_clip.project_point3(Vec3::new(x, y, 0.5));
        let right = (at(1.0, 0.0) - at(-1.0, 0.0)).normalize_or_zero();
        let up = (at(0.0, 1.0) - at(0.0, -1.0)).normalize_or_zero();
        let billboard = [right.extend(0.0).to_array(), up.extend(0.0).to_array()];
        let view = draw.view(&device, camera);
        queue.write_buffer(&view.uniform, 0, bytemuck::cast_slice(&billboard));
        let bind_group = view.bind_group.clone();

        let samples = render_camera.view_target.sample_count();
        let eye = render_camera.camera_uniform.view_position();
        let phase = phases.view_mut(camera);
        for emitter in &extracted.emitters {
            if !emitter.layers.intersects(render_camera.layers()) {
                continue;
            }
            phase.transparent.push(PhaseItem {
                pipeline: draw.pipelines.get(samples).clone(),
                bind_group: bind_group.clone(),
                bindings: PhaseBindings {
                    camera: true,
                    lighting: true,
                    skeleton: None,
                },
                mesh: PhaseMesh {
                    vertices: draw.corners.clone(),
                    indices: draw.indices.clone(),
                    index_count: 6,
                },
                instances: PhaseInstances {
                    buffer: draw.instances.clone(),
                    range: emitter.instances.clone(),
                },
                view_distance: emitter.center.distance(eye),
            });
        }
    }
}
