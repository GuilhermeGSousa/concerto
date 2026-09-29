use std::marker::PhantomData;

use concerto_app::{
    extractor::Extracted,
    plugins::Plugin,
    schedule_groups::{Extract, Render},
};
use concerto_ecs::{
    command::CommandQueue,
    query::Query,
    resource::{Res, ResMut, Resource},
    system::input::SystemInputData,
};

use crate::{
    assets::material::ShaderRef,
    capabilities::RenderCapabilities,
    components::{
        camera::RenderCamera,
        material::{MaterialComponent, RenderMaterialComponent},
        mesh::RenderMeshInstance,
        render_entity::RenderEntity,
        shadows::RenderLighting,
        skeleton::{RenderSkeletonComponent, SkinUniforms},
    },
    device::RenderDevice,
    layouts::{CameraLayout, LightingLayout, SkeletonLayout},
    render_asset::{
        render_mesh::RenderMesh,
        render_texture::{DummyRenderTexture, RenderTexture},
        render_window::RenderWindow,
        AssetPreparationError, RenderAsset, RenderAssetPlugin, RenderAssets,
    },
    resources::RenderContext,
    shader::create_shader_module,
    Material,
};

// Generic material plugin.
//
// Registering `MaterialPlugin::<M>::new()` sets up everything needed to render
// meshes that use material type `M`:
//
// * Registers `M` as an asset type.
// * Creates a `wgpu::RenderPipeline` whose layout is built from `M`'s own bind
//   group layout plus whichever engine bind groups `M` declares it needs via
//   [`Material::needs_camera`], [`Material::needs_lighting`] and
//   [`Material::needs_skeleton`].
// * Adds a render pass (`material_renderpass<M>`) that only processes mesh
//   entities carrying [`MaterialComponent<M>`].
//
// # Using a custom material
//
// 1. Derive (or manually implement) [`AsBindGroup`] for your type, then implement
//    [`Material`] (usually with default methods).
// 2. Register the plugin: `app.register_plugin(MaterialPlugin::<MyMaterial>::new())`.
// 3. Attach [`MaterialComponent::<MyMaterial>`] + [`MeshComponent`] to an entity.
//
// # Shader bind-group convention
//
// | Group | Contents                              | Condition                      |
// |-------|----------------------------------------|--------------------------------|
// | 0     | Material's own bindings                | always                         |
// | 1     | Camera uniform                         | `M::needs_camera()` → true     |
// | 2     | Lighting uniform + both shadow-map arrays | `M::needs_lighting()` → true |
// | 3     | Skeleton (bone) uniforms               | `M::needs_skeleton()` → true   |
//
// Your WGSL only needs to declare the groups (and, within group 2, the
// specific bindings) that your material actually uses.

// ─── Default (built-in) shader source ────────────────────────────────────────

pub(crate) const DEFAULT_SHADER_SOURCE: &str = include_str!("shaders/shader.wgsl");

// ─── MaterialPipeline ─────────────────────────────────────────────────────────

// Stores the wgpu render pipeline and material bind-group layout for `M`.
//
// Inserted as a resource by [`MaterialPlugin<M>::finish`].
pub struct MaterialPipeline<M: 'static> {
    pub pipeline: wgpu::RenderPipeline,
    // The `@group(0)` bind-group layout for `M`'s own data.
    pub bind_group_layout: wgpu::BindGroupLayout,
    _marker: PhantomData<fn() -> M>,
}

impl<M: 'static> MaterialPipeline<M> {
    pub fn new(pipeline: wgpu::RenderPipeline, bind_group_layout: wgpu::BindGroupLayout) -> Self {
        Self {
            pipeline,
            bind_group_layout,
            _marker: PhantomData,
        }
    }
}

// Manual Resource impl — #[derive(Resource)] doesn't handle PhantomData<fn()>.
impl<M: 'static> Resource for MaterialPipeline<M> {
    fn name() -> &'static str {
        std::any::type_name::<MaterialPipeline<M>>()
    }
}

// GPU-side representation of a prepared `M` instance.
//
// Stores the `wgpu::BindGroup` built from the material's data and ready to be
// bound at `@group(0)` during the material render pass.
pub struct RenderMaterial<M: 'static> {
    pub bind_group: wgpu::BindGroup,
    _marker: PhantomData<fn() -> M>,
}

impl<M: Material + 'static> RenderAsset for RenderMaterial<M> {
    type SourceAsset = M;
    type PreparationParams = (
        Res<'static, RenderDevice>,
        Res<'static, RenderAssets<RenderTexture>>,
        Res<'static, DummyRenderTexture>,
        Res<'static, MaterialPipeline<M>>,
    );

    fn prepare_asset(
        source_asset: &Self::SourceAsset,
        params: &mut SystemInputData<Self::PreparationParams>,
    ) -> Result<Self, AssetPreparationError> {
        let (device, render_textures, dummy_texture, pipeline) = params;
        let bind_group = source_asset.create_bind_group(
            device,
            render_textures,
            dummy_texture,
            &pipeline.bind_group_layout,
        )?;
        Ok(RenderMaterial {
            bind_group,
            _marker: PhantomData,
        })
    }
}

// ─── Systems ──────────────────────────────────────────────────────────────────

// Extracts every `MaterialComponent<M>` into its `RenderMaterialComponent<M>`
// mirror. Upserts like the other extract systems: a mirror that already names
// the same material asset is left alone, and one naming a different asset is
// replaced, so swapping the handle on a live entity takes effect next frame.
pub(crate) fn extract_materials<M: Material>(
    materials: Extracted<Query<(&MaterialComponent<M>, &RenderEntity)>>,
    render_materials: Query<&RenderMaterialComponent<M>>,
    mut cmd: CommandQueue,
) {
    for (material, render_entity) in materials.iter() {
        let render_entity = **render_entity;

        if render_materials
            .get_entity(render_entity)
            .is_some_and(|existing| existing.material_asset_id == material.handle.id())
        {
            continue;
        }

        cmd.insert(
            RenderMaterialComponent::<M>::new(material.handle.id()),
            render_entity,
        );
    }
}

// Clears color and depth for all cameras at the start of each frame.
//
// Runs before all `material_renderpass<M>` systems so every material pass can
// use `LoadOp::Load` without clobbering the previous pass's output.
pub(crate) fn clear_cameras(
    mut device: ResMut<RenderDevice>,
    render_cameras: Query<&RenderCamera>,
    render_window: Res<RenderWindow>,
) {
    let encoder = device.command_encoder();
    for render_camera in render_cameras.iter() {
        let swapchain_view = render_window.get_view();
        let color_view: &wgpu::TextureView = match &render_camera.render_target {
            Some(rt) => &rt.view,
            None => match swapchain_view {
                Some(v) => v,
                None => continue,
            },
        };
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Clear Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: render_camera.clear_color.r as f64,
                        g: render_camera.clear_color.g as f64,
                        b: render_camera.clear_color.b as f64,
                        a: render_camera.clear_color.a as f64,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &render_camera.depth_texture.view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            occlusion_query_set: None,
            timestamp_writes: None,
        });
    }
}

// Render pass for meshes that use material `M`.
//
// Only processes entities tagged with [`RenderMaterialComponent<M>`] so
// multiple `MaterialPlugin` instantiations for different material types can
// coexist without interfering with each other.
pub(crate) fn material_renderpass<M: Material>(
    pipeline: Res<MaterialPipeline<M>>,
    mut device: ResMut<RenderDevice>,
    render_mesh_query: Query<(
        &RenderMeshInstance,
        Option<&RenderSkeletonComponent>,
        &RenderMaterialComponent<M>,
    )>,
    render_cameras: Query<&RenderCamera>,
    render_meshes: Res<RenderAssets<RenderMesh>>,
    render_materials: Res<RenderAssets<RenderMaterial<M>>>,
    render_window: Res<RenderWindow>,
    render_lighting: Res<RenderLighting>,
    skins: Res<SkinUniforms>,
) {
    let encoder = device.command_encoder();

    for render_camera in render_cameras.iter() {
        let depth_load = if M::clear_depth() {
            wgpu::LoadOp::Clear(1.0)
        } else {
            wgpu::LoadOp::Load
        };

        // Route to RTT for texture-targeted cameras, or to the swapchain for window cameras.
        let swapchain_view = render_window.get_view();
        let color_view: &wgpu::TextureView = match &render_camera.render_target {
            Some(rt) => &rt.view,
            None => match swapchain_view {
                Some(v) => v,
                None => continue,
            },
        };

        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Material Render Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: M::depth_stencil().map(|_| {
                wgpu::RenderPassDepthStencilAttachment {
                    view: &render_camera.depth_texture.view,
                    depth_ops: Some(wgpu::Operations {
                        load: depth_load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }
            }),
            occlusion_query_set: None,
            timestamp_writes: None,
        });

        render_pass.set_pipeline(&pipeline.pipeline);

        // Set the engine built-in bind groups that M declared it needs.
        // Only the groups present in the pipeline layout are set — the
        // layout was built with the same conditions in `MaterialPlugin::finish()`.
        if M::needs_camera() {
            render_pass.set_bind_group(1, &render_camera.camera_bind_group, &[]);
        }
        if M::needs_lighting() {
            render_pass.set_bind_group(2, &render_lighting.bind_group, &[]);
        }

        for (mesh_instance, skeleton, render_mat_comp) in render_mesh_query.iter() {
            if let Some(mesh) = render_meshes.get(&mesh_instance.mesh_asset_id) {
                if let Some(render_mat) = render_materials.get(&render_mat_comp.material_asset_id) {
                    render_pass.set_bind_group(0, &render_mat.bind_group, &[]);
                } else {
                    continue;
                }

                if M::needs_skeleton() {
                    let offset = skeleton.map_or(0, |sk| sk.offset);
                    render_pass.set_bind_group(3, skins.bind_group(), &[offset]);
                }

                render_pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                render_pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                render_pass.set_vertex_buffer(1, mesh_instance.transform.slice(..));
                render_pass.draw_indexed(0..mesh.index_count, 0, 0..1);
            }
        }
    }
}

// ─── MaterialPlugin ───────────────────────────────────────────────────────────

// Register this plugin to enable rendering with material `M`.
//
// ```rust,ignore
// app.register_plugin(MaterialPlugin::<UnlitMaterial>::new());
// ```
//
// The plugin uses `M`'s [`Material`] flags to build a pipeline layout that
// contains only the bind groups the shaders actually need.  Shader code
// therefore never has to declare unused `@group(N)` bindings.
//
// # Pipeline-only mode
//
// For materials that have their own custom render pass (e.g. skybox, UI)
// you can use [`MaterialPlugin::pipeline_only`] to create just the
// [`MaterialPipeline<M>`] resource without registering the generic mesh
// rendering systems.
pub struct MaterialPlugin<M: Material> {
    // When `true`, only the [`MaterialPipeline<M>`] resource is set up.
    // No asset registration and no mesh rendering systems are added.
    pipeline_only: bool,
    _marker: PhantomData<fn() -> M>,
}

impl<M: Material> Default for MaterialPlugin<M> {
    fn default() -> Self {
        Self {
            pipeline_only: false,
            _marker: PhantomData,
        }
    }
}

impl<M: Material> MaterialPlugin<M> {
    // Full material plugin: registers the asset type, the render-asset
    // pipeline, and the generic mesh rendering systems.
    pub fn new() -> Self {
        Self::default()
    }

    // Pipeline-only mode: only creates the [`MaterialPipeline<M>`] resource.
    //
    // Use this when the material has its own custom render pass (e.g. skybox,
    // UI) and you only need the wgpu pipeline to be built from the material's
    // trait methods without the generic mesh rendering systems.
    pub fn pipeline_only() -> Self {
        Self {
            pipeline_only: true,
            _marker: PhantomData,
        }
    }
}

impl<M: Material> Plugin for MaterialPlugin<M> {
    fn build(&self, app: &mut concerto_app::App) {
        if self.pipeline_only {
            // Pipeline-only: no asset store, no render-asset preparation, no
            // mesh rendering systems.  Only the pipeline resource is created
            // in `finish`.
            return;
        }

        app.register_asset::<M>();
        app.register_plugin(RenderAssetPlugin::<RenderMaterial<M>>::new());

        // extract_materials<M> must be per-material so we know which material handle
        // to store in RenderMaterialComponent<M>. Mesh transforms, however, are
        // handled by the shared extract_meshes system already registered by
        // RenderPlugin, which iterates over all entities with RenderEntity
        // regardless of material type.
        app.add_render_system(Extract, extract_materials::<M>)
            .add_render_system(Render, material_renderpass::<M>);
    }

    fn finish(&self, app: &mut concerto_app::App) {
        let device = app
            .render()
            .get_resource::<RenderDevice>()
            .expect("RenderDevice not found; register RenderPlugin before MaterialPlugin");
        let surface_format = app
            .render()
            .get_resource::<RenderContext>()
            .expect("RenderContext not found")
            .surface_config
            .format;

        // Build the material's own @group(0) bind-group layout.
        let material_layout = M::bind_group_layout(device);

        let mut all_layouts: Vec<&wgpu::BindGroupLayout> = vec![&material_layout];

        if M::needs_camera() {
            let camera_layout = app
                .render()
                .get_resource::<CameraLayout>()
                .expect("CameraLayout not found");
            all_layouts.push(&camera_layout.camera_layout);
        }

        if M::needs_lighting() {
            let lighting_layout = app
                .render()
                .get_resource::<LightingLayout>()
                .expect("LightingLayout not found");
            all_layouts.push(lighting_layout);
        }

        if M::needs_skeleton() {
            let skeleton_layout = app
                .render()
                .get_resource::<SkeletonLayout>()
                .expect("SkeletonLayout not found");
            all_layouts.push(skeleton_layout);
        }

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Material Pipeline Layout"),
            bind_group_layouts: &all_layouts,
            push_constant_ranges: &[],
        });

        // Resolve shader sources (fall back to the built-in Phong shader).
        let vs_src: &str = match M::vertex_shader() {
            ShaderRef::Default => DEFAULT_SHADER_SOURCE,
            ShaderRef::Source(src) => src,
        };
        let fs_src: &str = match M::fragment_shader() {
            ShaderRef::Default => DEFAULT_SHADER_SOURCE,
            ShaderRef::Source(src) => src,
        };

        let mut shader_defs = app
            .render()
            .get_resource::<RenderCapabilities>()
            .expect("RenderCapabilities not found; register RenderPlugin before MaterialPlugin")
            .shader_defs();
        M::shader_defs(&mut shader_defs);

        let vs_module = create_shader_module(device, "Material VS", vs_src, &shader_defs);
        let fs_module = create_shader_module(device, "Material FS", fs_src, &shader_defs);

        // Use the vertex layouts from the material trait.
        let vertex_layouts = M::vertex_layouts();

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Material Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &vs_module,
                entry_point: Some("vs_main"),
                buffers: &vertex_layouts,
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &fs_module,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: M::blend_state(),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: M::topology(),
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: M::cull_mode(),
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: M::depth_stencil(),
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });

        app.render_mut()
            .insert_resource(MaterialPipeline::<M>::new(pipeline, material_layout));
    }
}
