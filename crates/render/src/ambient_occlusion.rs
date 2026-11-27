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
use concerto_foundation::{assets::AssetId, transform::GlobalTransformRaw};
use concerto_mesh::Vertex;
use concerto_reflect::{Reflect, ReflectDefault};
use glam::{Mat4, UVec2, Vec3};

use crate::{
    assets::vertex::VertexBufferLayout,
    batching::{BatchedInstanceBuffers, BinCandidate, ViewBatches},
    capabilities::RenderCapabilities,
    components::{
        camera::{Camera, PixelRect, RenderCamera},
        mesh::{RenderMeshInstanceFlags, RenderMeshInstances},
        render_entity::{MainEntity, RenderEntity},
        shadows::RenderShadowCasterViewProj,
        skeleton::{RenderSkeletonComponent, SkinUniforms},
    },
    depth_pass::{DepthPassKind, DepthPassLayout, DepthPassView, DepthPassViews},
    device::RenderDevice,
    frustum::Frustum,
    layouts::{CameraLayout, SkeletonLayout},
    queue::RenderQueue,
    render_asset::{render_mesh::RenderMesh, render_texture::FallbackTextures, RenderAssets},
    resources::RenderContext,
    sets::{PrepassSet, RenderSet},
    shader::create_shader_module,
    sky_light::SkyRadianceMap,
    view_target::{prepare_view_targets, ViewTarget},
};

const RAW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;
const OCCLUSION_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

/// Darkens the ambient light of the standard material where nearby surfaces block it: creases, corners and contact with the ground.
///
/// Goes on the camera's entity. A camera without one has none.
#[derive(Component, Clone, Copy, Debug, PartialEq, Reflect)]
#[reflect(Component, Default, SceneComponent)]
pub struct ScreenSpaceAmbientOcclusion {
    /// How far, in world units, a surface looks for what blocks its ambient light.
    pub radius: f32,
    /// How much of the occlusion found is applied: 0 is none, 1 all of it.
    pub intensity: f32,
}

impl Default for ScreenSpaceAmbientOcclusion {
    fn default() -> Self {
        Self {
            radius: 0.5,
            intensity: 1.0,
        }
    }
}

impl ScreenSpaceAmbientOcclusion {
    /// Sets how far a surface looks for what blocks its ambient light.
    pub fn with_radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }

    /// Sets how much of the occlusion found is applied.
    pub fn with_intensity(mut self, intensity: f32) -> Self {
        self.intensity = intensity;
        self
    }
}

/// Draws [`ScreenSpaceAmbientOcclusion`] for the cameras that have it, from a depth prepass in [`RenderSet::Prepass`].
///
/// # Panics
///
/// Panics during [`finish`](Plugin::finish) if [`RenderPlugin`](crate::plugin::RenderPlugin)
/// was not registered before it.
pub struct AmbientOcclusionPlugin;

impl Plugin for AmbientOcclusionPlugin {
    fn build(&self, app: &mut concerto_app::App) {
        app.register_type::<ScreenSpaceAmbientOcclusion>();
        app.render_mut()
            .insert_resource(ExtractedAmbientOcclusion::default())
            .insert_resource(AmbientOcclusionViews::default());
        app.add_render_system(Extract, extract_ambient_occlusion)
            .add_render_system(
                Render,
                prepare_ambient_occlusion
                    .after(prepare_view_targets)
                    .before(RenderSet::Queue),
            )
            .add_render_system(
                Render,
                queue_ambient_occlusion_meshes.in_set(RenderSet::Queue),
            )
            .add_render_system(
                Render,
                render_ambient_occlusion_depth
                    .in_set(RenderSet::Prepass)
                    .in_set(PrepassSet::Depth),
            )
            .add_render_system(
                Render,
                render_ambient_occlusion
                    .in_set(RenderSet::Prepass)
                    .in_set(PrepassSet::Read),
            );
    }

    fn finish(&self, app: &mut concerto_app::App) {
        let render = app.render();
        let device = render
            .get_resource::<RenderDevice>()
            .expect("AmbientOcclusionPlugin needs RenderPlugin, registered before it");
        let capabilities = render
            .get_resource::<RenderCapabilities>()
            .expect("AmbientOcclusionPlugin needs RenderPlugin, registered before it");
        let camera_layout = render
            .get_resource::<CameraLayout>()
            .expect("AmbientOcclusionPlugin needs RenderPlugin, registered before it");
        let skeleton_layout = render
            .get_resource::<SkeletonLayout>()
            .expect("AmbientOcclusionPlugin needs RenderPlugin, registered before it");
        if !capabilities.float_render_targets {
            log::warn!(
                "ambient occlusion needs float render targets, which this adapter lacks; \
                 ScreenSpaceAmbientOcclusion has no effect"
            );
            return;
        }
        let depth_pass_layout = render
            .get_resource::<DepthPassLayout>()
            .expect("AmbientOcclusionPlugin needs RenderPlugin, registered before it");
        let pipelines = AmbientOcclusionPipelines::new(
            device,
            capabilities,
            camera_layout,
            skeleton_layout,
            depth_pass_layout,
        );
        app.render_mut().insert_resource(pipelines);
    }
}

#[derive(Resource, Default)]
pub(crate) struct ExtractedAmbientOcclusion {
    cameras: EntityHashMap<ScreenSpaceAmbientOcclusion>,
}

pub(crate) fn extract_ambient_occlusion(
    cameras: Extracted<Query<(&ScreenSpaceAmbientOcclusion, &RenderEntity), With<Camera>>>,
    mut extracted: ResMut<ExtractedAmbientOcclusion>,
) {
    extracted.cameras = cameras
        .iter()
        .map(|(occlusion, render_entity)| (**render_entity, *occlusion))
        .collect();
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct OcclusionUniform {
    rect: [f32; 4],
    right: [f32; 3],
    pixels_per_unit: f32,
    up: [f32; 3],
    radius: f32,
    forward: [f32; 3],
    intensity: f32,
}

impl OcclusionUniform {
    fn new(
        render_camera: &RenderCamera,
        rect: PixelRect,
        settings: &ScreenSpaceAmbientOcclusion,
    ) -> Self {
        let world_from_clip = render_camera.camera_uniform.inverse_view_proj();
        let at =
            |x: f32, y: f32, depth: f32| world_from_clip.project_point3(Vec3::new(x, y, depth));
        let forward = (at(0.0, 0.0, 1.0) - at(0.0, 0.0, 0.0)).normalize_or_zero();
        let across = at(1.0, 0.0, 0.5) - at(-1.0, 0.0, 0.5);
        let upward = at(0.0, 1.0, 0.5) - at(0.0, -1.0, 0.5);
        let depth = (at(0.0, 0.0, 0.5) - render_camera.camera_uniform.view_position()).dot(forward);
        let width_at_unit_depth = across.length() / depth.max(f32::EPSILON);
        Self {
            rect: [
                rect.x as f32,
                rect.y as f32,
                rect.width as f32,
                rect.height as f32,
            ],
            right: across.normalize_or_zero().to_array(),
            pixels_per_unit: rect.width as f32 / width_at_unit_depth.max(f32::EPSILON),
            up: upward.normalize_or_zero().to_array(),
            radius: settings.radius.max(1e-3),
            forward: forward.to_array(),
            intensity: settings.intensity.clamp(0.0, 1.0),
        }
    }
}

#[derive(Resource)]
pub(crate) struct AmbientOcclusionPipelines {
    prepass: wgpu::RenderPipeline,
    view_proj_layout: wgpu::BindGroupLayout,
    occlusion: wgpu::RenderPipeline,
    occlusion_layout: wgpu::BindGroupLayout,
    blur: wgpu::RenderPipeline,
    blur_layout: wgpu::BindGroupLayout,
}

fn uniform_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn unfiltered_texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

struct FullScreenPipeline<'a> {
    label: &'static str,
    source: &'static str,
    layouts: &'a [Option<&'a wgpu::BindGroupLayout>],
    format: wgpu::TextureFormat,
}

impl FullScreenPipeline<'_> {
    fn build(
        &self,
        device: &wgpu::Device,
        capabilities: &RenderCapabilities,
    ) -> wgpu::RenderPipeline {
        let module =
            create_shader_module(device, self.label, self.source, &capabilities.shader_defs());
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(self.label),
            bind_group_layouts: self.layouts,
            immediate_size: 0,
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(self.label),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: self.format,
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
    }
}

impl AmbientOcclusionPipelines {
    fn new(
        device: &wgpu::Device,
        capabilities: &RenderCapabilities,
        camera_layout: &CameraLayout,
        skeleton_layout: &SkeletonLayout,
        depth_pass_layout: &DepthPassLayout,
    ) -> Self {
        let view_proj_layout = depth_pass_layout.0.clone();
        let prepass_module = create_shader_module(
            device,
            "Depth Prepass",
            include_str!("shaders/shadow.wgsl"),
            &capabilities.shader_defs(),
        );
        let prepass_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Depth Prepass Pipeline Layout"),
            bind_group_layouts: &[Some(&view_proj_layout), Some(skeleton_layout)],
            immediate_size: 0,
        });
        let prepass = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Depth Prepass Pipeline"),
            layout: Some(&prepass_layout),
            vertex: wgpu::VertexState {
                module: &prepass_module,
                entry_point: Some("vs_main"),
                buffers: &[
                    Some(Vertex::describe()),
                    Some(GlobalTransformRaw::describe()),
                ],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: None,
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: ViewTarget::DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let occlusion_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Ambient Occlusion Layout"),
            entries: &[
                unfiltered_texture_entry(0),
                uniform_entry(1, wgpu::ShaderStages::FRAGMENT),
            ],
        });
        let occlusion = FullScreenPipeline {
            label: "Ambient Occlusion",
            source: include_str!("shaders/ambient_occlusion.wgsl"),
            layouts: &[Some(&camera_layout.camera_layout), Some(&occlusion_layout)],
            format: RAW_FORMAT,
        }
        .build(device, capabilities);

        let blur_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Ambient Occlusion Blur Layout"),
            entries: &[unfiltered_texture_entry(0)],
        });
        let blur = FullScreenPipeline {
            label: "Ambient Occlusion Blur",
            source: include_str!("shaders/ambient_occlusion_blur.wgsl"),
            layouts: &[Some(&blur_layout)],
            format: OCCLUSION_FORMAT,
        }
        .build(device, capabilities);

        Self {
            prepass,
            view_proj_layout,
            occlusion,
            occlusion_layout,
            blur,
            blur_layout,
        }
    }
}

struct AmbientOcclusionView {
    size: UVec2,
    depth: wgpu::TextureView,
    raw: wgpu::TextureView,
    occlusion: wgpu::TextureView,
    view_proj: RenderShadowCasterViewProj,
    uniform: wgpu::Buffer,
    occlusion_bind_group: wgpu::BindGroup,
    blur_bind_group: wgpu::BindGroup,
    batches: ViewBatches<AssetId>,
}

fn target_view(
    device: &wgpu::Device,
    label: &'static str,
    format: wgpu::TextureFormat,
    size: UVec2,
) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size.x,
                height: size.y,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

impl AmbientOcclusionView {
    fn new(device: &wgpu::Device, pipelines: &AmbientOcclusionPipelines, size: UVec2) -> Self {
        let depth = target_view(device, "Depth Prepass", ViewTarget::DEPTH_FORMAT, size);
        let raw = target_view(device, "Ambient Occlusion Raw", RAW_FORMAT, size);
        let occlusion = target_view(device, "Ambient Occlusion", OCCLUSION_FORMAT, size);
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Ambient Occlusion Uniform"),
            size: std::mem::size_of::<OcclusionUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let occlusion_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Ambient Occlusion Bind Group"),
            layout: &pipelines.occlusion_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&depth),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: uniform.as_entire_binding(),
                },
            ],
        });
        let blur_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Ambient Occlusion Blur Bind Group"),
            layout: &pipelines.blur_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&raw),
            }],
        });
        Self {
            size,
            depth,
            raw,
            occlusion,
            view_proj: RenderShadowCasterViewProj::new(device, &pipelines.view_proj_layout),
            uniform,
            occlusion_bind_group,
            blur_bind_group,
            batches: ViewBatches::new(device, "Depth Prepass Instances Buffer"),
        }
    }
}

#[derive(Resource, Default)]
pub(crate) struct AmbientOcclusionViews {
    cameras: EntityHashMap<AmbientOcclusionView>,
}

pub(crate) fn prepare_ambient_occlusion(
    pipelines: Option<Res<AmbientOcclusionPipelines>>,
    extracted: Res<ExtractedAmbientOcclusion>,
    mut views: ResMut<AmbientOcclusionViews>,
    mut render_cameras: Query<(Entity, &mut RenderCamera)>,
    device: Res<RenderDevice>,
    camera_layout: Res<CameraLayout>,
    sky_radiance: Res<SkyRadianceMap>,
    fallbacks: Res<FallbackTextures>,
) {
    let Some(pipelines) = pipelines else {
        return;
    };
    views
        .cameras
        .retain(|camera, _| render_cameras.contains_entity(*camera));

    for (camera, mut render_camera) in render_cameras.iter_mut() {
        let wanted = extracted
            .cameras
            .get(&camera)
            .is_some_and(|settings| settings.intensity > 0.0);
        if !wanted {
            views.cameras.remove(&camera);
            if render_camera.ambient_occlusion.is_some() {
                render_camera.ambient_occlusion = None;
                render_camera.rebind(&device, &camera_layout, &sky_radiance, &fallbacks);
            }
            continue;
        }
        let size = render_camera.view_target.size();
        if views
            .cameras
            .get(&camera)
            .is_some_and(|view| view.size == size)
            && render_camera.ambient_occlusion.is_some()
        {
            continue;
        }
        let view = AmbientOcclusionView::new(&device, &pipelines, size);
        render_camera.ambient_occlusion = Some(view.occlusion.clone());
        render_camera.rebind(&device, &camera_layout, &sky_radiance, &fallbacks);
        views.cameras.insert(camera, view);
    }
}

pub(crate) fn queue_ambient_occlusion_meshes(
    mut views: ResMut<AmbientOcclusionViews>,
    mut depth_pass_views: ResMut<DepthPassViews>,
    context: Res<RenderContext>,
    render_cameras: Query<&RenderCamera>,
    render_mesh_query: Query<(&MainEntity, Option<&RenderSkeletonComponent>)>,
    render_mesh_instances: Res<RenderMeshInstances>,
    buffers: Res<BatchedInstanceBuffers>,
    render_meshes: Res<RenderAssets<RenderMesh>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    for (camera, view) in views.cameras.iter_mut() {
        let Some(render_camera) = render_cameras.get_entity(*camera) else {
            continue;
        };
        let frustum = Frustum::from_clip_from_world(render_camera.camera_uniform.view_proj());
        let rect = render_camera.viewport_rect(render_camera.target_size(&context));
        if !rect.is_empty() {
            depth_pass_views.0.push(DepthPassView {
                kind: DepthPassKind::Prepass,
                target: view.depth.clone(),
                view_proj: view.view_proj.bind_group().clone(),
                frustum,
                layers: Some(render_camera.layers),
                rect: Some(rect),
                shadow_layer: None,
            });
        }
        let candidates = render_mesh_query
            .iter()
            .filter_map(|(main_entity, skeleton)| {
                let instance = render_mesh_instances.get(main_entity)?;
                if !instance.layers.intersects(render_camera.layers)
                    || !instance
                        .flags
                        .contains(RenderMeshInstanceFlags::SHADOW_CASTER)
                    || instance.flags.intersects(
                        RenderMeshInstanceFlags::BLENDED | RenderMeshInstanceFlags::CUTOUT,
                    )
                    || !render_meshes.contains(&instance.mesh_asset_id)
                {
                    return None;
                }
                Some(BinCandidate {
                    key: instance.mesh_asset_id,
                    instance,
                    skeleton_offset: skeleton.map(|skeleton| skeleton.offset),
                })
            });
        view.batches
            .bins
            .rebuild(candidates, &frustum, &buffers.current_input_buffer);
        view.batches.upload(&device, &queue);
    }
}

struct FullScreenPass<'a> {
    label: &'static str,
    target: &'a wgpu::TextureView,
    clear: wgpu::Color,
    rect: PixelRect,
    pipeline: &'a wgpu::RenderPipeline,
    bind_groups: &'a [&'a wgpu::BindGroup],
}

impl FullScreenPass<'_> {
    fn draw(&self, encoder: &mut wgpu::CommandEncoder) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            multiview_mask: None,
            label: Some(self.label),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                depth_slice: None,
                view: self.target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(self.clear),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
        });
        self.rect.apply(&mut pass);
        pass.set_pipeline(self.pipeline);
        for (index, bind_group) in self.bind_groups.iter().enumerate() {
            pass.set_bind_group(index as u32, *bind_group, &[]);
        }
        pass.draw(0..3, 0..1);
    }
}

pub(crate) fn render_ambient_occlusion_depth(
    pipelines: Option<Res<AmbientOcclusionPipelines>>,
    views: Res<AmbientOcclusionViews>,
    render_cameras: Query<(Entity, &RenderCamera)>,
    render_meshes: Res<RenderAssets<RenderMesh>>,
    skins: Res<SkinUniforms>,
    context: Res<RenderContext>,
    mut device: ResMut<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    let Some(pipelines) = pipelines else {
        return;
    };
    let encoder = device.command_encoder();
    for (camera, render_camera) in render_cameras.iter() {
        let Some(view) = views.cameras.get(&camera) else {
            continue;
        };
        let rect = render_camera.viewport_rect(render_camera.target_size(&context));
        if rect.is_empty() {
            continue;
        }

        let clip_from_world: Mat4 = render_camera.camera_uniform.view_proj();
        view.view_proj.write(&queue, clip_from_world);

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            multiview_mask: None,
            label: Some("Depth Prepass"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &view.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            occlusion_query_set: None,
            timestamp_writes: None,
        });
        rect.apply(&mut pass);
        if view.batches.bins.batches.is_empty() {
            continue;
        }
        pass.set_pipeline(&pipelines.prepass);
        pass.set_bind_group(0, view.view_proj.bind_group(), &[]);
        pass.set_vertex_buffer(1, view.batches.instances.buffer().slice(..));
        for batch in &view.batches.bins.batches {
            let Some(mesh) = render_meshes.get(&batch.key) else {
                continue;
            };
            pass.set_bind_group(1, skins.bind_group(), &[batch.skeleton_offset]);
            pass.set_vertex_buffer(0, mesh.vertices.slice(..));
            pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..mesh.index_count, 0, batch.instances.clone());
        }
    }
}

pub(crate) fn render_ambient_occlusion(
    pipelines: Option<Res<AmbientOcclusionPipelines>>,
    extracted: Res<ExtractedAmbientOcclusion>,
    views: Res<AmbientOcclusionViews>,
    render_cameras: Query<(Entity, &RenderCamera)>,
    context: Res<RenderContext>,
    mut device: ResMut<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    let Some(pipelines) = pipelines else {
        return;
    };
    let encoder = device.command_encoder();
    for (camera, render_camera) in render_cameras.iter() {
        let (Some(view), Some(settings)) =
            (views.cameras.get(&camera), extracted.cameras.get(&camera))
        else {
            continue;
        };
        let rect = render_camera.viewport_rect(render_camera.target_size(&context));
        if rect.is_empty() {
            continue;
        }

        queue.write_buffer(
            &view.uniform,
            0,
            bytemuck::bytes_of(&OcclusionUniform::new(render_camera, rect, settings)),
        );

        FullScreenPass {
            label: "Ambient Occlusion Pass",
            target: &view.raw,
            clear: wgpu::Color {
                r: 1.0,
                g: 60000.0,
                b: 0.0,
                a: 1.0,
            },
            rect,
            pipeline: &pipelines.occlusion,
            bind_groups: &[&render_camera.camera_bind_group, &view.occlusion_bind_group],
        }
        .draw(encoder);
        FullScreenPass {
            label: "Ambient Occlusion Blur Pass",
            target: &view.occlusion,
            clear: wgpu::Color::WHITE,
            rect,
            pipeline: &pipelines.blur,
            bind_groups: &[&view.blur_bind_group],
        }
        .draw(encoder);
    }
}
