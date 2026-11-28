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
    Entity, IntoSystemConfig,
};
use concerto_reflect::{Reflect, ReflectDefault};
use glam::{UVec2, Vec3};

use crate::{
    capabilities::RenderCapabilities,
    components::{
        camera::{Camera, PixelRect, RenderCamera},
        render_entity::RenderEntity,
    },
    device::RenderDevice,
    layouts::CameraLayout,
    queue::RenderQueue,
    render_asset::render_texture::RenderTexture,
    resources::RenderContext,
    sets::{PostProcessSet, RenderSet},
    shader::create_shader_module,
    view_target::ViewPipelines,
};

const BLUR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;
const SENSOR_HEIGHT: f32 = 0.024;
const MULTISAMPLED_DEPTH: &str = "MULTISAMPLED_DEPTH";

/// Keeps what is at one distance from a camera sharp and softens what is nearer and farther, as a lens does. Goes on the camera's entity, or on a virtual camera through its `focus`.
#[derive(Component, Clone, Copy, Debug, PartialEq, Reflect)]
#[reflect(Component, Default, SceneComponent)]
pub struct DepthOfField {
    /// The distance from the camera that is in focus, in world units.
    pub focus_distance: f32,
    /// The lens's aperture as an f-number: lower blurs more, and the range in focus is shallower.
    pub f_stop: f32,
    /// The widest blur, as a fraction of the picture's height; 0 is no blur.
    pub max_blur: f32,
}

impl Default for DepthOfField {
    fn default() -> Self {
        Self {
            focus_distance: 5.0,
            f_stop: 2.0,
            max_blur: 0.015,
        }
    }
}

impl DepthOfField {
    /// A lens focused at `focus_distance`.
    pub fn at(focus_distance: f32) -> Self {
        Self {
            focus_distance,
            ..Self::default()
        }
    }

    /// Sets the aperture as an f-number.
    pub fn with_f_stop(mut self, f_stop: f32) -> Self {
        self.f_stop = f_stop;
        self
    }

    /// Sets the widest blur, as a fraction of the picture's height.
    pub fn with_max_blur(mut self, max_blur: f32) -> Self {
        self.max_blur = max_blur;
        self
    }

    /// The lens `amount` of the way from this one to `other`.
    pub fn lerp(self, other: Self, amount: f32) -> Self {
        let mix = |from: f32, to: f32| from + (to - from) * amount;
        Self {
            focus_distance: mix(self.focus_distance, other.focus_distance),
            f_stop: mix(self.f_stop, other.f_stop),
            max_blur: mix(self.max_blur, other.max_blur),
        }
    }

    /// This lens with its widest blur scaled by `amount`, for fading it in from or out to no blur at all.
    pub fn faded(mut self, amount: f32) -> Self {
        self.max_blur *= amount.clamp(0.0, 1.0);
        self
    }

    /// The blur radius, as a fraction of the picture's height and before `max_blur`, of a point `distance` away through a lens of vertical field of view `fovy`.
    pub fn blur_at(&self, distance: f32, fovy: f32) -> f32 {
        let focal_length = SENSOR_HEIGHT * 0.5 / (fovy * 0.5).tan();
        let focus = self.focus_distance.max(focal_length * 2.0);
        let on_sensor =
            focal_length * focal_length / (self.f_stop.max(0.1) * (focus - focal_length));
        0.5 * on_sensor / SENSOR_HEIGHT * (distance - focus).abs() / distance.max(f32::EPSILON)
    }
}

/// Draws [`DepthOfField`] for the cameras that have it, in [`RenderSet::PostProcess`] before bloom.
///
/// # Panics
///
/// Panics during [`finish`](Plugin::finish) if [`RenderPlugin`](crate::plugin::RenderPlugin)
/// was not registered before it.
pub struct DepthOfFieldPlugin;

impl Plugin for DepthOfFieldPlugin {
    fn build(&self, app: &mut concerto_app::App) {
        app.register_type::<DepthOfField>();
        app.render_mut()
            .insert_resource(ExtractedDepthOfField::default())
            .insert_resource(DepthOfFieldViews::default());
        app.add_render_system(Extract, extract_depth_of_field)
            .add_render_system(
                Render,
                render_depth_of_field
                    .in_set(RenderSet::PostProcess)
                    .in_set(PostProcessSet::DepthOfField),
            );
    }

    fn finish(&self, app: &mut concerto_app::App) {
        let render = app.render();
        let needs = "DepthOfFieldPlugin needs RenderPlugin, registered before it";
        let device = render.get_resource::<RenderDevice>().expect(needs);
        let capabilities = render.get_resource::<RenderCapabilities>().expect(needs);
        let camera_layout = render.get_resource::<CameraLayout>().expect(needs);
        let context = render.get_resource::<RenderContext>().expect(needs);
        if !capabilities.float_render_targets {
            log::warn!(
                "depth of field needs float render targets, which this adapter lacks; \
                 DepthOfField has no effect"
            );
            return;
        }
        let pipelines = DepthOfFieldPipelines::new(device, capabilities, camera_layout, context);
        app.render_mut().insert_resource(pipelines);
    }
}

struct ExtractedLens {
    lens: DepthOfField,
    fovy: f32,
}

#[derive(Resource, Default)]
pub(crate) struct ExtractedDepthOfField {
    cameras: EntityHashMap<ExtractedLens>,
}

pub(crate) fn extract_depth_of_field(
    cameras: Extracted<Query<(&DepthOfField, &Camera, &RenderEntity)>>,
    mut extracted: ResMut<ExtractedDepthOfField>,
) {
    extracted.cameras = cameras
        .iter()
        .map(|(lens, camera, render_entity)| {
            (
                **render_entity,
                ExtractedLens {
                    lens: *lens,
                    fovy: camera.fovy,
                },
            )
        })
        .collect();
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FocusUniform {
    rect: [f32; 4],
    forward: [f32; 3],
    focus_distance: f32,
    blur_scale: f32,
    max_blur: f32,
    _padding: [f32; 2],
}

struct FocusLayouts {
    single: wgpu::BindGroupLayout,
    multisampled: wgpu::BindGroupLayout,
}

#[derive(Resource)]
pub(crate) struct DepthOfFieldPipelines {
    layouts: FocusLayouts,
    radius: ViewPipelines,
    blur: ViewPipelines,
    sampler: wgpu::Sampler,
    format: wgpu::TextureFormat,
    multisampled_depth: bool,
}

fn focus_layout(device: &wgpu::Device, multisampled: bool) -> wgpu::BindGroupLayout {
    let texture = |binding, sample_type, multisampled| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled,
        },
        count: None,
    };
    let unfiltered = wgpu::TextureSampleType::Float { filterable: false };
    let filtered = wgpu::TextureSampleType::Float { filterable: true };
    let depth = if multisampled {
        wgpu::TextureSampleType::Depth
    } else {
        unfiltered
    };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Depth Of Field Layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            texture(1, depth, multisampled),
            texture(2, filtered, false),
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            texture(4, unfiltered, false),
        ],
    })
}

impl DepthOfFieldPipelines {
    fn new(
        device: &wgpu::Device,
        capabilities: &RenderCapabilities,
        camera_layout: &CameraLayout,
        context: &RenderContext,
    ) -> Self {
        let layouts = FocusLayouts {
            single: focus_layout(device, false),
            multisampled: focus_layout(device, true),
        };
        let format = context.view_target_format;
        let pipelines = |label: &'static str, entry_point: &'static str, target| {
            ViewPipelines::new(|samples| {
                let multisampled = samples > 1;
                let mut defs = capabilities.shader_defs();
                if multisampled {
                    defs.define(MULTISAMPLED_DEPTH);
                }
                let module = create_shader_module(
                    device,
                    label,
                    include_str!("shaders/depth_of_field.wgsl"),
                    &defs,
                );
                let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some(label),
                    bind_group_layouts: &[
                        Some(&camera_layout.camera_layout),
                        Some(if multisampled {
                            &layouts.multisampled
                        } else {
                            &layouts.single
                        }),
                    ],
                    immediate_size: 0,
                });
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(label),
                    layout: Some(&layout),
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
                            format: target,
                            blend: None,
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
            })
        };
        Self {
            radius: pipelines("Depth Of Field Radius", "fs_blur_radius", BLUR_FORMAT),
            blur: pipelines("Depth Of Field Blur", "fs_blur", format),
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("Depth Of Field Sampler"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            layouts,
            format,
            multisampled_depth: context.multisampled_depth_sampled,
        }
    }
}

struct DepthOfFieldView {
    size: UVec2,
    uniform: wgpu::Buffer,
    radius: RenderTexture,
    blurred: RenderTexture,
}

impl DepthOfFieldView {
    fn new(device: &wgpu::Device, pipelines: &DepthOfFieldPipelines, size: UVec2) -> Self {
        Self {
            size,
            uniform: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Depth Of Field Uniform"),
                size: std::mem::size_of::<FocusUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            radius: RenderTexture::camera_target(device, BLUR_FORMAT, size.x, size.y),
            blurred: RenderTexture::camera_target(device, pipelines.format, size.x, size.y),
        }
    }
}

#[derive(Resource, Default)]
pub(crate) struct DepthOfFieldViews {
    cameras: EntityHashMap<DepthOfFieldView>,
    warned: bool,
}

struct FocusPass<'a> {
    label: &'static str,
    pipeline: &'a wgpu::RenderPipeline,
    camera: &'a wgpu::BindGroup,
    focus: &'a wgpu::BindGroup,
    target: &'a wgpu::TextureView,
    rect: PixelRect,
}

impl FocusPass<'_> {
    fn draw(&self, encoder: &mut wgpu::CommandEncoder) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            multiview_mask: None,
            label: Some(self.label),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                depth_slice: None,
                view: self.target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
        });
        self.rect.apply(&mut pass);
        pass.set_pipeline(self.pipeline);
        pass.set_bind_group(0, self.camera, &[]);
        pass.set_bind_group(1, self.focus, &[]);
        pass.draw(0..3, 0..1);
    }
}

pub(crate) fn render_depth_of_field(
    pipelines: Option<Res<DepthOfFieldPipelines>>,
    extracted: Res<ExtractedDepthOfField>,
    mut views: ResMut<DepthOfFieldViews>,
    mut device: ResMut<RenderDevice>,
    queue: Res<RenderQueue>,
    render_cameras: Query<(Entity, &RenderCamera)>,
    context: Res<RenderContext>,
) {
    let Some(pipelines) = pipelines else {
        return;
    };
    let views = &mut *views;
    views
        .cameras
        .retain(|camera, _| extracted.cameras.contains_key(camera));

    for (camera, render_camera) in render_cameras.iter() {
        let Some(extracted) = extracted.cameras.get(&camera) else {
            continue;
        };
        let lens = extracted.lens;
        let rect = render_camera.viewport_rect(render_camera.target_size(&context));
        if rect.is_empty() || lens.max_blur <= 0.0 {
            continue;
        }
        let view_target = &render_camera.view_target;
        let samples = view_target.sample_count();
        if samples > 1 && !pipelines.multisampled_depth {
            if !views.warned {
                views.warned = true;
                log::warn!(
                    "depth of field can't read multisampled depth on this adapter; \
                     give the camera Msaa::Off to use DepthOfField"
                );
            }
            continue;
        }
        let size = view_target.size();
        if views
            .cameras
            .get(&camera)
            .is_none_or(|view| view.size != size)
        {
            views
                .cameras
                .insert(camera, DepthOfFieldView::new(&device, &pipelines, size));
        }
        let view = &views.cameras[&camera];

        let world_from_clip = render_camera.camera_uniform.inverse_view_proj();
        let at = |depth: f32| world_from_clip.project_point3(Vec3::new(0.0, 0.0, depth));
        let forward = (at(1.0) - at(0.0)).normalize_or_zero();
        let height = rect.height as f32;
        let far_blur = lens.blur_at(f32::MAX, extracted.fovy);
        queue.write_buffer(
            &view.uniform,
            0,
            bytemuck::bytes_of(&FocusUniform {
                rect: [
                    rect.x as f32,
                    rect.y as f32,
                    rect.width as f32,
                    rect.height as f32,
                ],
                forward: forward.to_array(),
                focus_distance: lens.focus_distance,
                blur_scale: far_blur * height,
                max_blur: lens.max_blur * height,
                _padding: [0.0; 2],
            }),
        );
        let bind_group = |radius: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Depth Of Field Bind Group"),
                layout: if samples > 1 {
                    &pipelines.layouts.multisampled
                } else {
                    &pipelines.layouts.single
                },
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: view.uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(
                            &view_target.depth_texture().view,
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(
                            &view_target.main_texture().view,
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&pipelines.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(radius),
                    },
                ],
            })
        };
        let radius_focus = bind_group(&view.blurred.view);
        let focus = bind_group(&view.radius.view);

        let encoder = device.command_encoder();
        FocusPass {
            label: "Depth Of Field Radius Pass",
            pipeline: pipelines.radius.get(samples),
            camera: &render_camera.camera_bind_group,
            focus: &radius_focus,
            target: &view.radius.view,
            rect,
        }
        .draw(encoder);
        FocusPass {
            label: "Depth Of Field Blur Pass",
            pipeline: pipelines.blur.get(samples),
            camera: &render_camera.camera_bind_group,
            focus: &focus,
            target: &view.blurred.view,
            rect,
        }
        .draw(encoder);
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: view.blurred.texture(),
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: rect.x,
                    y: rect.y,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: view_target.main_texture().texture(),
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: rect.x,
                    y: rect.y,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: rect.width,
                height: rect.height,
                depth_or_array_layers: 1,
            },
        );
    }
}
