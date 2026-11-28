//! The generic material plugin, which renders meshes with any [`Material`].
//!
//! Registering `MaterialPlugin::<M>::new()` sets up everything needed to
//! render meshes that use material type `M`:
//!
//! * Registers `M` as an asset type and [`MaterialComponent<M>`] for
//!   reflection.
//! * Creates an opaque and a blended `wgpu::RenderPipeline`, each for one and
//!   for four samples, whose layout is built from `M`'s own bind group layout
//!   plus whichever engine bind groups `M` declares it needs via
//!   [`Material::needs_camera`], [`Material::needs_lighting`] and
//!   [`Material::needs_skeleton`].
//! * Adds `queue_material_meshes<M>`, which culls and bins the mesh entities
//!   carrying [`MaterialComponent<M>`] for each camera, opaque ones by key and
//!   [`AlphaMode::Blend`] ones one draw each, and queues them in the camera's
//!   view phase: the opaque phase is drawn in [`RenderSet::Draw`] and the
//!   transparent phase, sorted back to front across every material type, in
//!   [`RenderSet::Transparent`].
//! * Adds `flag_alpha_instances<M>`, which flags blended instances so they stay
//!   out of the shadow maps, and masked instances of a material with a cutout
//!   shader so the cutout pass draws them.
//!
//! # Using a custom material
//!
//! 1. Derive (or implement by hand) [`AsBindGroup`] for your type, then
//!    implement [`Material`] (usually with its default methods) and
//!    `LoadableAsset`.
//! 2. Register the plugin: `app.register_plugin(MaterialPlugin::<MyMaterial>::new())`.
//! 3. Attach [`MaterialComponent<MyMaterial>`](MaterialComponent) and a
//!    [`concerto_mesh::MeshComponent`] to an entity.
//!
//! # Shader bind-group convention
//!
//! | Group | Contents                                  | Condition                    |
//! |-------|-------------------------------------------|------------------------------|
//! | 0     | Material's own bindings                   | always                       |
//! | 1     | Camera uniform                            | `M::needs_camera()` is true  |
//! | 2     | Lighting uniform + both shadow-map arrays | `M::needs_lighting()` is true |
//! | 3     | Skeleton (bone) uniforms                  | `M::needs_skeleton()` is true |
//!
//! Your WGSL declares only the groups (and, within group 2, the bindings) that
//! your material uses.

use std::marker::PhantomData;

use concerto_app::{
    extractor::Extracted,
    plugins::Plugin,
    schedule_groups::{Extract, Render},
};
use concerto_color::{Color, LinearRgba};
use concerto_ecs::{
    command::CommandQueue,
    component::removed::RemovedComponents,
    entity::EntityHashMap,
    query::Query,
    resource::{Res, ResMut, Resource},
    system::input::{SystemInputData, SystemLocal},
    world::{FromWorld, World},
    Entity, IntoSystemConfig, Without,
};
use concerto_foundation::assets::AssetId;

use crate::{
    assets::material::{AlphaMode, AsBindGroup, ShaderRef, StandardMaterial},
    batching::{
        clear_draw_stats, BatchedInstanceBuffers, BinCandidate, RenderDrawStats, ViewBatches,
    },
    capabilities::RenderCapabilities,
    components::{
        camera::RenderCamera,
        material::{HasMaterial, MaterialComponent, RenderMaterialComponent},
        mesh::{RenderMeshInstanceFlags, RenderMeshInstances},
        render_entity::{MainEntity, RenderEntity},
        skeleton::RenderSkeletonComponent,
    },
    depth_pass::{
        queue_cutout_meshes, render_cutout_meshes, CutoutBatches, CutoutPipelines, DepthPassLayout,
    },
    device::RenderDevice,
    frustum::Frustum,
    layouts::{CameraLayout, LightingLayout, SkeletonLayout},
    queue::RenderQueue,
    render_asset::{
        render_mesh::RenderMesh,
        render_texture::{DummyRenderTexture, RenderTexture},
        render_window::RenderWindow,
        AssetPreparationError, RenderAsset, RenderAssetPlugin, RenderAssets,
    },
    resources::RenderContext,
    sets::{PrepassSet, RenderSet},
    shader::{create_shader_module, ALPHA_TO_COVERAGE},
    view_phase::{
        clear_view_phases, PhaseBindings, PhaseInstances, PhaseItem, PhaseMesh, ViewPhases,
    },
    view_target::{Msaa, ViewPipelines, ViewTarget},
    Material,
};

pub(crate) const DEFAULT_SHADER_SOURCE: &str = include_str!("shaders/shader.wgsl");

/// The wgpu render pipelines and material bind-group layout for `M`.
///
/// [`MaterialPlugin<M>`] inserts it as a render-world resource when it
/// finishes.
pub struct MaterialPipeline<M: 'static> {
    pipelines: ViewPipelines,
    blended_pipelines: Option<ViewPipelines>,
    masked_pipeline: Option<wgpu::RenderPipeline>,
    /// The `@group(0)` bind-group layout for `M`'s own data.
    pub bind_group_layout: wgpu::BindGroupLayout,
    _marker: PhantomData<fn() -> M>,
}

impl<M: 'static> MaterialPipeline<M> {
    /// Creates the pipeline set from the opaque and blended pipelines and `M`'s own layout, with
    /// no masked pipeline.
    pub fn new(
        pipelines: ViewPipelines,
        blended_pipelines: Option<ViewPipelines>,
        bind_group_layout: wgpu::BindGroupLayout,
    ) -> Self {
        Self {
            pipelines,
            blended_pipelines,
            masked_pipeline: None,
            bind_group_layout,
            _marker: PhantomData,
        }
    }

    /// This pipeline set with `pipeline` drawing `AlphaMode::Mask` assets into a multisampled target with alpha to coverage.
    pub fn with_masked_pipeline(mut self, pipeline: wgpu::RenderPipeline) -> Self {
        self.masked_pipeline = Some(pipeline);
        self
    }

    /// The pipeline that draws an asset of `alpha_mode` into the opaque phase of a target of `samples` samples.
    ///
    /// A masked asset on a multisampled target gets the alpha-to-coverage
    /// pipeline when the material has one.
    pub fn pipeline_for(&self, alpha_mode: AlphaMode, samples: u32) -> &wgpu::RenderPipeline {
        match &self.masked_pipeline {
            Some(masked) if samples > 1 && matches!(alpha_mode, AlphaMode::Mask { .. }) => masked,
            _ => self.pipeline(samples),
        }
    }

    /// The pipeline that draws into a target of `samples` samples.
    pub fn pipeline(&self, samples: u32) -> &wgpu::RenderPipeline {
        self.pipelines.get(samples)
    }

    /// The pipeline that blends [`AlphaMode::Blend`] assets without writing depth into a target of `samples` samples.
    ///
    /// Returns `None` for a pipeline-only plugin.
    pub fn blended_pipeline(&self, samples: u32) -> Option<&wgpu::RenderPipeline> {
        self.blended_pipelines
            .as_ref()
            .map(|pipelines| pipelines.get(samples))
    }
}

// Manual Resource impl — #[derive(Resource)] doesn't handle PhantomData<fn()>.
impl<M: 'static> Resource for MaterialPipeline<M> {
    fn name() -> &'static str {
        std::any::type_name::<MaterialPipeline<M>>()
    }
}

/// The GPU-side form of a prepared `M` asset.
///
/// Holds the `wgpu::BindGroup` built from the material's data, bound at
/// `@group(0)` during the material render pass.
pub struct RenderMaterial<M: 'static> {
    /// The bind group of the material's data, bound at `@group(0)`.
    pub bind_group: wgpu::BindGroup,
    /// The source asset's alpha mode when it was prepared.
    pub alpha_mode: AlphaMode,
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
        _id: AssetId,
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
            alpha_mode: source_asset.alpha_mode(),
            _marker: PhantomData,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct MaterialBinKey {
    material: AssetId,
    mesh: AssetId,
}

pub(crate) struct MaterialViewBatches<M> {
    views: EntityHashMap<ViewBatches<MaterialBinKey>>,
    blended_views: EntityHashMap<ViewBatches<MaterialBinKey>>,
    _marker: PhantomData<fn() -> M>,
}

impl<M> Default for MaterialViewBatches<M> {
    fn default() -> Self {
        Self {
            views: EntityHashMap::default(),
            blended_views: EntityHashMap::default(),
            _marker: PhantomData,
        }
    }
}

impl<M: 'static> Resource for MaterialViewBatches<M> {
    fn name() -> &'static str {
        std::any::type_name::<MaterialViewBatches<M>>()
    }
}

pub(crate) const FALLBACK_MATERIAL: AssetId = AssetId::from_bytes([
    0x5f, 0x0a, 0x3c, 0x91, 0x7b, 0x2e, 0x4d, 0x18, 0x9c, 0x44, 0xe1, 0x6a, 0x02, 0xd7, 0xb3, 0x8f,
]);

fn clear_material<M: Material>(
    render_entity: Entity,
    render_materials: &Query<(&RenderMaterialComponent<M>, Option<&HasMaterial>)>,
    cmd: &mut CommandQueue,
) {
    if render_materials
        .get_entity(render_entity)
        .is_some_and(|(existing, _)| existing.material_asset_id != FALLBACK_MATERIAL)
    {
        cmd.remove::<RenderMaterialComponent<M>>(render_entity);
        cmd.remove::<HasMaterial>(render_entity);
    }
}

// Extracts every `MaterialComponent<M>` into its `RenderMaterialComponent<M>`
// mirror. Upserts like the other extract systems: a mirror that already names
// the same material asset is left alone, and one naming a different asset is
// replaced, so swapping the handle on a live entity takes effect next frame.
// A nil handle or a removed component takes the mirror away.
pub(crate) fn extract_materials<M: Material>(
    materials: Extracted<Query<(&MaterialComponent<M>, &RenderEntity)>>,
    mut removed_materials: Extracted<RemovedComponents<MaterialComponent<M>>>,
    render_entities: Extracted<Query<&RenderEntity>>,
    render_materials: Query<(&RenderMaterialComponent<M>, Option<&HasMaterial>)>,
    standard_materials: Query<&RenderMaterialComponent<StandardMaterial>>,
    mut cmd: CommandQueue,
) {
    for (material, render_entity) in materials.iter() {
        let render_entity = **render_entity;
        let id = material.handle.id();

        if id.is_nil() {
            clear_material(render_entity, &render_materials, &mut cmd);
            continue;
        }

        if render_materials
            .get_entity(render_entity)
            .is_some_and(|(existing, assigned)| {
                existing.material_asset_id == id && assigned.is_some()
            })
        {
            continue;
        }

        if standard_materials
            .get_entity(render_entity)
            .is_some_and(|standard| standard.material_asset_id == FALLBACK_MATERIAL)
        {
            cmd.remove::<RenderMaterialComponent<StandardMaterial>>(render_entity);
        }
        cmd.insert(
            (RenderMaterialComponent::<M>::new(id), HasMaterial),
            render_entity,
        );
    }

    for entity in removed_materials.read() {
        if materials.contains_entity(entity) {
            continue;
        }
        if let Some(render_entity) = render_entities.get_entity(entity) {
            clear_material(**render_entity, &render_materials, &mut cmd);
        }
    }
}

pub(crate) fn assign_fallback_material(
    unassigned: Query<
        (Entity, &MainEntity),
        (
            Without<HasMaterial>,
            Without<RenderMaterialComponent<StandardMaterial>>,
        ),
    >,
    render_mesh_instances: Res<RenderMeshInstances>,
    render_materials: Option<ResMut<RenderAssets<RenderMaterial<StandardMaterial>>>>,
    pipeline: Option<Res<MaterialPipeline<StandardMaterial>>>,
    device: Res<RenderDevice>,
    render_textures: Res<RenderAssets<RenderTexture>>,
    dummy_texture: Res<DummyRenderTexture>,
    mut cmd: CommandQueue,
) {
    let (Some(mut render_materials), Some(pipeline)) = (render_materials, pipeline) else {
        return;
    };
    if !render_materials.contains(&FALLBACK_MATERIAL) {
        let grey =
            StandardMaterial::default().with_base_color_factor(Color::rgba(0.5, 0.5, 0.5, 1.0));
        let Ok(bind_group) = grey.create_bind_group(
            &device,
            &render_textures,
            &dummy_texture,
            &pipeline.bind_group_layout,
        ) else {
            return;
        };
        render_materials.insert(
            FALLBACK_MATERIAL,
            RenderMaterial {
                bind_group,
                alpha_mode: AlphaMode::Opaque,
                _marker: PhantomData,
            },
        );
    }

    for (render_entity, main_entity) in unassigned.iter() {
        if render_mesh_instances.contains_key(main_entity) {
            cmd.insert(
                RenderMaterialComponent::<StandardMaterial>::new(FALLBACK_MATERIAL),
                render_entity,
            );
        }
    }
}

pub(crate) fn flag_alpha_instances<M: Material>(
    render_mesh_query: Query<(&MainEntity, &RenderMaterialComponent<M>)>,
    render_materials: Res<RenderAssets<RenderMaterial<M>>>,
    mut render_mesh_instances: ResMut<RenderMeshInstances>,
) {
    for (main_entity, material) in render_mesh_query.iter() {
        let (Some(instance), Some(render_material)) = (
            render_mesh_instances.get_mut(main_entity),
            render_materials.get(&material.material_asset_id),
        ) else {
            continue;
        };
        instance.flags.set(
            RenderMeshInstanceFlags::BLENDED,
            render_material.alpha_mode == AlphaMode::Blend,
        );
        instance.flags.set(
            RenderMeshInstanceFlags::CUTOUT,
            M::cutout_shader().is_some()
                && matches!(render_material.alpha_mode, AlphaMode::Mask { .. }),
        );
    }
}

pub(crate) struct WindowOverlapWarned {
    warned: bool,
}

impl FromWorld for WindowOverlapWarned {
    fn from_world(_: &World) -> Self {
        Self { warned: false }
    }
}

fn wgpu_color(color: LinearRgba) -> wgpu::Color {
    wgpu::Color {
        r: color.r as f64,
        g: color.g as f64,
        b: color.b as f64,
        a: color.a as f64,
    }
}

pub(crate) fn clear_cameras(
    mut device: ResMut<RenderDevice>,
    render_cameras: Query<&RenderCamera>,
    render_window: Res<RenderWindow>,
    context: Res<RenderContext>,
    mut overlap: SystemLocal<WindowOverlapWarned>,
) {
    let window_cameras = || {
        render_cameras
            .iter()
            .filter(|render_camera| render_camera.render_target.is_none())
    };
    let any_with_viewport = window_cameras().any(|camera| camera.viewport.is_some());
    let any_without_viewport = window_cameras().any(|camera| camera.viewport.is_none());
    if any_with_viewport && any_without_viewport && !overlap.warned {
        overlap.warned = true;
        log::warn!(
            "a camera without a viewport draws to the window beside cameras with one, so they \
             overlap and which one shows is undefined; keep one window camera without a \
             viewport, or give every window camera a viewport"
        );
    }

    let encoder = device.command_encoder();
    for render_camera in render_cameras.iter() {
        if render_camera
            .viewport_rect(render_camera.target_size(&context))
            .is_empty()
        {
            continue;
        }
        let view_target = &render_camera.view_target;
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            multiview_mask: None,
            label: Some("View Target Clear Pass"),
            color_attachments: &[Some(view_target.color_attachment(wgpu::LoadOp::Clear(
                wgpu_color(render_camera.clear_color),
            )))],
            depth_stencil_attachment: Some(view_target.depth_attachment(wgpu::LoadOp::Clear(1.0))),
            occlusion_query_set: None,
            timestamp_writes: None,
        });
    }

    if let Some(window_view) = render_window.get_view() {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            multiview_mask: None,
            label: Some("Window Clear Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                depth_slice: None,
                view: window_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
        });
    }
}

pub(crate) fn queue_material_meshes<M: Material>(
    render_mesh_query: Query<(
        &MainEntity,
        Option<&RenderSkeletonComponent>,
        &RenderMaterialComponent<M>,
    )>,
    render_cameras: Query<(Entity, &RenderCamera)>,
    render_mesh_instances: Res<RenderMeshInstances>,
    buffers: Res<BatchedInstanceBuffers>,
    render_meshes: Res<RenderAssets<RenderMesh>>,
    render_materials: Res<RenderAssets<RenderMaterial<M>>>,
    pipeline: Res<MaterialPipeline<M>>,
    mut view_batches: ResMut<MaterialViewBatches<M>>,
    mut phases: ResMut<ViewPhases>,
    mut draw_stats: ResMut<RenderDrawStats>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    let view_batches = &mut *view_batches;
    let (render_mesh_instances, render_meshes, render_materials) =
        (&*render_mesh_instances, &*render_meshes, &*render_materials);
    for (camera, render_camera) in render_cameras.iter() {
        let frustum = Frustum::from_clip_from_world(render_camera.camera_uniform.view_proj());
        let candidates = |blended: bool| {
            render_mesh_query
                .iter()
                .filter_map(move |(main_entity, skeleton, material)| {
                    let instance = render_mesh_instances.get(main_entity)?;
                    let render_material = render_materials.get(&material.material_asset_id)?;
                    if !instance.layers.intersects(render_camera.layers)
                        || !render_meshes.contains(&instance.mesh_asset_id)
                        || (render_material.alpha_mode == AlphaMode::Blend) != blended
                    {
                        return None;
                    }
                    Some(BinCandidate {
                        key: MaterialBinKey {
                            material: material.material_asset_id,
                            mesh: instance.mesh_asset_id,
                        },
                        instance,
                        skeleton_offset: skeleton.map(|skeleton| skeleton.offset),
                    })
                })
        };
        let samples = render_camera.view_target.sample_count();
        let phase = phases.view_mut(camera);
        let items = |batches: &ViewBatches<MaterialBinKey>,
                     blended: Option<&wgpu::RenderPipeline>| {
            batches
                .bins
                .batches
                .iter()
                .filter_map(|batch| {
                    let mesh = render_meshes.get(&batch.key.mesh)?;
                    let material = render_materials.get(&batch.key.material)?;
                    Some(PhaseItem {
                        pipeline: blended
                            .unwrap_or_else(|| pipeline.pipeline_for(material.alpha_mode, samples))
                            .clone(),
                        bind_group: material.bind_group.clone(),
                        bindings: PhaseBindings {
                            camera: M::needs_camera(),
                            lighting: M::needs_lighting(),
                            skeleton: M::needs_skeleton().then_some(batch.skeleton_offset),
                        },
                        mesh: PhaseMesh {
                            vertices: mesh.vertices.clone(),
                            indices: mesh.indices.clone(),
                            index_count: mesh.index_count,
                        },
                        instances: PhaseInstances {
                            buffer: batches.instances.buffer().clone(),
                            range: batch.instances.clone(),
                        },
                        view_distance: batch.view_distance,
                    })
                })
                .collect::<Vec<_>>()
        };

        let batches = view_batches
            .views
            .entry(camera)
            .or_insert_with(|| ViewBatches::new(&device, "Material View Instances Buffer"));
        let opaque =
            batches
                .bins
                .rebuild(candidates(false), &frustum, &buffers.current_input_buffer);
        batches.upload(&device, &queue);
        phase.opaque.extend(items(batches, None));

        let batches = view_batches
            .blended_views
            .entry(camera)
            .or_insert_with(|| ViewBatches::new(&device, "Blended Material View Instances Buffer"));
        let blended = batches.bins.rebuild_back_to_front(
            candidates(true),
            &frustum,
            render_camera.camera_uniform.view_position(),
            &buffers.current_input_buffer,
        );
        batches.upload(&device, &queue);
        if let Some(blended_pipeline) = pipeline.blended_pipeline(samples) {
            phase
                .transparent
                .extend(items(batches, Some(blended_pipeline)));
        }

        let total = draw_stats.cameras.entry(camera).or_default();
        total.draws += opaque.draws + blended.draws;
        total.culled += opaque.culled + blended.culled;
    }
    view_batches
        .views
        .retain(|camera, _| render_cameras.contains_entity(*camera));
    view_batches
        .blended_views
        .retain(|camera, _| render_cameras.contains_entity(*camera));
}

struct PipelineVariant<'a> {
    label: &'a str,
    blend: Option<wgpu::BlendState>,
    depth_stencil: Option<wgpu::DepthStencilState>,
    samples: u32,
    fragment: &'a wgpu::ShaderModule,
    alpha_to_coverage: bool,
}

/// The plugin that renders meshes with material `M`.
///
/// It uses `M`'s [`Material`] flags to build a pipeline layout holding only the
/// bind groups the shaders need, so shader code never declares unused
/// `@group(N)` bindings. See the [module docs](crate::material_plugin) for the bind-group
/// convention.
///
/// A material with its own render pass, such as the skybox or UI, uses
/// [`MaterialPlugin::pipeline_only`] to create just the
/// [`MaterialPipeline<M>`] resource without the generic mesh rendering
/// systems.
///
/// # Panics
///
/// Panics during [`finish`](Plugin::finish) if [`RenderPlugin`](crate::plugin::RenderPlugin)
/// was not registered before it.
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
    /// The full material plugin, which registers the asset type, the
    /// render-asset pipeline and the generic mesh rendering systems.
    pub fn new() -> Self {
        Self::default()
    }

    /// A plugin that only creates the [`MaterialPipeline<M>`] resource, whose
    /// pipeline targets the window's format rather than the view target's.
    ///
    /// Use it when the material has its own render pass, such as the skybox or
    /// UI, and needs only the wgpu pipeline built from its trait methods,
    /// without the generic mesh rendering systems.
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
        app.register_type::<MaterialComponent<M>>();
        app.track_removals::<MaterialComponent<M>>();
        app.register_plugin(RenderAssetPlugin::<RenderMaterial<M>>::new());

        app.add_render_system(Extract, extract_materials::<M>)
            .add_render_system(Render, flag_alpha_instances::<M>.before(clear_draw_stats))
            .add_render_system(
                Render,
                queue_material_meshes::<M>
                    .in_set(RenderSet::Queue)
                    .after(clear_draw_stats)
                    .after(clear_view_phases),
            );
        if M::cutout_shader().is_some() {
            app.add_render_system(Render, queue_cutout_meshes::<M>.in_set(RenderSet::Shadows))
                .add_render_system(
                    Render,
                    render_cutout_meshes::<M>
                        .in_set(RenderSet::Prepass)
                        .in_set(PrepassSet::Cutout),
                );
        }
    }

    fn finish(&self, app: &mut concerto_app::App) {
        let device = app
            .render()
            .get_resource::<RenderDevice>()
            .expect("RenderDevice not found; register RenderPlugin before MaterialPlugin");
        let context = app
            .render()
            .get_resource::<RenderContext>()
            .expect("RenderContext not found");
        let target_format = if self.pipeline_only {
            context.surface_config.format
        } else {
            context.view_target_format
        };

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

        let all_layouts: Vec<Option<&wgpu::BindGroupLayout>> =
            all_layouts.into_iter().map(Some).collect();
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Material Pipeline Layout"),
            bind_group_layouts: &all_layouts,
            immediate_size: 0,
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
        let vertex_layouts: Vec<_> = M::vertex_layouts().into_iter().map(Some).collect();

        let create_pipeline = |variant: PipelineVariant| {
            let PipelineVariant {
                label,
                blend,
                depth_stencil,
                samples,
                fragment,
                alpha_to_coverage,
            } = variant;
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &vs_module,
                    entry_point: Some("vs_main"),
                    buffers: &vertex_layouts,
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: fragment,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: target_format,
                        blend,
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
                depth_stencil,
                multisample: wgpu::MultisampleState {
                    count: samples,
                    mask: !0,
                    alpha_to_coverage_enabled: alpha_to_coverage,
                },
                multiview_mask: None,
                cache: None,
            })
        };

        let phase_depth = || {
            if self.pipeline_only {
                return M::depth_stencil();
            }
            Some(M::depth_stencil().unwrap_or(wgpu::DepthStencilState {
                format: ViewTarget::DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }))
        };
        let pipeline = ViewPipelines::new(|samples| {
            create_pipeline(PipelineVariant {
                label: "Material Pipeline",
                blend: M::blend_state(),
                depth_stencil: phase_depth(),
                samples,
                fragment: &fs_module,
                alpha_to_coverage: false,
            })
        });
        let blended_pipeline = (!self.pipeline_only).then(|| {
            ViewPipelines::new(|samples| {
                create_pipeline(PipelineVariant {
                    label: "Blended Material Pipeline",
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    depth_stencil: phase_depth().map(|depth_stencil| wgpu::DepthStencilState {
                        depth_write_enabled: Some(false),
                        ..depth_stencil
                    }),
                    samples,
                    fragment: &fs_module,
                    alpha_to_coverage: false,
                })
            })
        });
        let masked_pipeline = (!self.pipeline_only && M::alpha_to_coverage()).then(|| {
            let mut defs = shader_defs.clone();
            defs.define(ALPHA_TO_COVERAGE);
            let fragment = create_shader_module(device, "Masked Material FS", fs_src, &defs);
            create_pipeline(PipelineVariant {
                label: "Masked Material Pipeline",
                blend: M::blend_state(),
                depth_stencil: phase_depth(),
                samples: Msaa::Sample4.samples(),
                fragment: &fragment,
                alpha_to_coverage: true,
            })
        });

        let mut material_pipeline =
            MaterialPipeline::<M>::new(pipeline, blended_pipeline, material_layout);
        if let Some(masked_pipeline) = masked_pipeline {
            material_pipeline = material_pipeline.with_masked_pipeline(masked_pipeline);
        }
        app.render_mut().insert_resource(material_pipeline);
        if !self.pipeline_only {
            app.render_mut()
                .insert_resource(MaterialViewBatches::<M>::default());
            let render = app.render();
            let cutout = CutoutPipelines::<M>::new(
                render.get_resource::<RenderDevice>().unwrap(),
                render
                    .get_resource::<RenderCapabilities>()
                    .expect("RenderCapabilities not found"),
                render
                    .get_resource::<DepthPassLayout>()
                    .expect("DepthPassLayout not found"),
                render
                    .get_resource::<SkeletonLayout>()
                    .expect("SkeletonLayout not found"),
                &render
                    .get_resource::<MaterialPipeline<M>>()
                    .unwrap()
                    .bind_group_layout,
            );
            if let Some(cutout) = cutout {
                app.render_mut()
                    .insert_resource(cutout)
                    .insert_resource(CutoutBatches::<M>::default());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        assets::mesh::Mesh, batching::ViewDrawStats, components::camera::Camera,
        draw_target::DrawTarget, gpu::Gpu, plugin::RenderPlugin,
    };
    use concerto_app::{
        main_schedule::MainSchedulePlugin,
        plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
        App,
    };
    use concerto_foundation::{
        assets::{asset_server::AssetServer, handle::AssetHandle},
        transform::Transform,
    };
    use concerto_mesh::MeshComponent;
    use concerto_window::plugin::{Window, WindowPlugin};
    use glam::Vec3;

    struct Spawned {
        app: App,
        entity: Entity,
    }

    fn app(gpu: &Gpu, material: AssetId) -> Spawned {
        let mut app = App::new();
        app.insert_resource(gpu.clone())
            .insert_resource(DrawTarget::None)
            .insert_resource(Window::new((64, 48), 1.0));
        app.register_plugin(MainSchedulePlugin)
            .register_plugin(AssetManagerPlugin)
            .register_plugin(TimePlugin)
            .register_plugin(WindowPlugin)
            .register_plugin(TransformPlugin)
            .register_plugin(RenderPlugin)
            .register_plugin(MaterialPlugin::<StandardMaterial>::default());
        let world = app.main_mut().world_mut();
        world.register_component::<Transform>();
        let entity = world.spawn((
            MaterialComponent::<StandardMaterial> {
                handle: AssetHandle::weak(material),
            },
            Transform::IDENTITY,
        ));
        app.finish_plugin_build();
        Spawned { app, entity }
    }

    fn drawn_with(app: &mut App) -> Vec<AssetId> {
        let world = app.render_mut().world_mut();
        world
            .query::<&RenderMaterialComponent<StandardMaterial>, ()>()
            .iter(world)
            .map(|material| material.material_asset_id)
            .collect()
    }

    #[test]
    fn a_material_plugin_registers_its_component_and_the_handle_it_holds() {
        use concerto_ecs::reflect::{AppTypeRegistry, ReflectComponent, ReflectSceneLeaf};
        use concerto_foundation::assets::handle::ReflectAssetHandle;
        use std::any::TypeId;

        let mut app = App::new();
        app.register_plugin(MainSchedulePlugin)
            .register_plugin(AssetManagerPlugin)
            .register_plugin(MaterialPlugin::<StandardMaterial>::new());

        let registry = app.get_resource::<AppTypeRegistry>().unwrap();
        let component = TypeId::of::<MaterialComponent<StandardMaterial>>();
        let handle = TypeId::of::<AssetHandle<StandardMaterial>>();
        assert!(registry
            .get_type_data::<ReflectComponent>(component)
            .is_some());
        assert!(registry.get_type_data::<ReflectSceneLeaf>(handle).is_some());
        assert_eq!(
            registry
                .get_type_data::<ReflectAssetHandle>(handle)
                .map(|data| data.kind()),
            Some("StandardMaterial")
        );
    }

    #[test]
    fn swapping_a_material_handle_changes_the_material_its_render_entity_draws_with() {
        let Some(gpu) = Gpu::request_for_test() else {
            return;
        };
        let (first, second) = (AssetId::new(), AssetId::new());
        let Spawned { mut app, entity } = app(&gpu, first);
        app.update();
        assert_eq!(drawn_with(&mut app), [first]);

        app.main_mut()
            .world_mut()
            .get_component_for_entity_mut::<MaterialComponent<StandardMaterial>>(entity)
            .unwrap()
            .handle = AssetHandle::weak(second);
        app.update();

        assert_eq!(drawn_with(&mut app), [second]);
    }

    fn scene(gpu: &Gpu) -> App {
        let mut app = App::new();
        app.insert_resource(gpu.clone())
            .insert_resource(DrawTarget::None)
            .insert_resource(Window::new((64, 48), 1.0));
        app.register_plugin(MainSchedulePlugin)
            .register_plugin(AssetManagerPlugin)
            .register_plugin(TimePlugin)
            .register_plugin(WindowPlugin)
            .register_plugin(TransformPlugin)
            .register_plugin(RenderPlugin)
            .register_plugin(MaterialPlugin::<StandardMaterial>::default());
        let world = app.main_mut().world_mut();
        world.register_component::<Transform>();
        world.spawn((Camera::default(), Transform::IDENTITY));
        app.finish_plugin_build();
        app
    }

    fn mesh(app: &App) -> AssetHandle<Mesh> {
        app.get_resource::<AssetServer>()
            .unwrap()
            .add(Mesh::cuboid(Vec3::ONE))
    }

    fn material(app: &App) -> AssetHandle<StandardMaterial> {
        app.get_resource::<AssetServer>()
            .unwrap()
            .add(StandardMaterial::default())
    }

    fn spawn(
        app: &mut App,
        mesh: &AssetHandle<Mesh>,
        material: &AssetHandle<StandardMaterial>,
        translation: Vec3,
    ) -> Entity {
        app.main_mut().world_mut().spawn((
            MeshComponent {
                handle: mesh.clone(),
            },
            MaterialComponent::<StandardMaterial> {
                handle: material.clone(),
            },
            Transform::from_translation(translation),
        ))
    }

    fn prepared(app: &App, meshes: &[AssetId], materials: &[AssetId]) -> bool {
        let render = app.render();
        let render_meshes = render.get_resource::<RenderAssets<RenderMesh>>().unwrap();
        let render_materials = render
            .get_resource::<RenderAssets<RenderMaterial<StandardMaterial>>>()
            .unwrap();
        meshes.iter().all(|id| render_meshes.contains(id))
            && materials.iter().all(|id| render_materials.contains(id))
    }

    fn settle(
        app: &mut App,
        meshes: &[&AssetHandle<Mesh>],
        materials: &[&AssetHandle<StandardMaterial>],
    ) {
        let meshes: Vec<AssetId> = meshes.iter().map(|mesh| mesh.id()).collect();
        let materials: Vec<AssetId> = materials.iter().map(|material| material.id()).collect();
        for _ in 0..100 {
            app.update();
            if prepared(app, &meshes, &materials) {
                break;
            }
        }
        assert!(prepared(app, &meshes, &materials));
        app.update();
        app.update();
    }

    fn camera_stats(app: &App) -> ViewDrawStats {
        let stats = app.render().get_resource::<RenderDrawStats>().unwrap();
        assert_eq!(stats.cameras.len(), 1);
        *stats.cameras.values().next().unwrap()
    }

    #[test]
    fn a_mesh_draws_with_the_fallback_material_while_it_has_none_of_its_own() {
        let Some(gpu) = Gpu::request_for_test() else {
            return;
        };
        let mut app = scene(&gpu);
        let cube = mesh(&app);
        let entity = app.main_mut().world_mut().spawn((
            MeshComponent {
                handle: cube.clone(),
            },
            Transform::from_translation(Vec3::new(0.0, 0.0, -5.0)),
        ));

        settle(&mut app, &[&cube], &[]);

        assert_eq!(drawn_with(&mut app), [FALLBACK_MATERIAL]);
        assert_eq!(
            camera_stats(&app),
            ViewDrawStats {
                draws: 1,
                culled: 0
            }
        );

        let white = material(&app);
        app.main_mut().world_mut().insert(
            MaterialComponent::<StandardMaterial> {
                handle: white.clone(),
            },
            entity,
        );
        settle(&mut app, &[&cube], &[&white]);

        assert_eq!(drawn_with(&mut app), [white.id()]);
        assert_eq!(
            camera_stats(&app),
            ViewDrawStats {
                draws: 1,
                culled: 0
            }
        );

        app.main_mut()
            .world_mut()
            .get_component_for_entity_mut::<MaterialComponent<StandardMaterial>>(entity)
            .unwrap()
            .handle = AssetHandle::default();
        app.update();
        app.update();

        assert_eq!(
            drawn_with(&mut app),
            [FALLBACK_MATERIAL],
            "a nil handle is no material"
        );

        app.main_mut().world_mut().insert(
            MaterialComponent::<StandardMaterial> {
                handle: white.clone(),
            },
            entity,
        );
        app.update();
        app.update();
        app.main_mut()
            .world_mut()
            .remove_component::<MaterialComponent<StandardMaterial>>(entity);
        app.update();
        app.update();

        assert_eq!(
            drawn_with(&mut app),
            [FALLBACK_MATERIAL],
            "a removed material brings the fallback back"
        );
    }

    #[test]
    fn meshes_sharing_a_mesh_and_material_draw_in_one_call() {
        let Some(gpu) = Gpu::request_for_test() else {
            return;
        };
        let mut app = scene(&gpu);
        let (cube, white) = (mesh(&app), material(&app));
        for x in [-1.0, 0.0, 1.0] {
            spawn(&mut app, &cube, &white, Vec3::new(x, 0.0, -5.0));
        }

        settle(&mut app, &[&cube], &[&white]);

        assert_eq!(
            camera_stats(&app),
            ViewDrawStats {
                draws: 1,
                culled: 0
            }
        );
    }

    #[test]
    fn meshes_with_different_materials_draw_once_per_material() {
        let Some(gpu) = Gpu::request_for_test() else {
            return;
        };
        let mut app = scene(&gpu);
        let cube = mesh(&app);
        let (first, second) = (material(&app), material(&app));
        spawn(&mut app, &cube, &first, Vec3::new(-1.0, 0.0, -5.0));
        spawn(&mut app, &cube, &second, Vec3::new(0.0, 0.0, -5.0));
        spawn(&mut app, &cube, &first, Vec3::new(1.0, 0.0, -5.0));

        settle(&mut app, &[&cube], &[&first, &second]);

        assert_eq!(
            camera_stats(&app),
            ViewDrawStats {
                draws: 2,
                culled: 0
            }
        );
    }

    #[test]
    fn a_mesh_behind_the_camera_is_culled() {
        let Some(gpu) = Gpu::request_for_test() else {
            return;
        };
        let mut app = scene(&gpu);
        let (cube, white) = (mesh(&app), material(&app));
        spawn(&mut app, &cube, &white, Vec3::new(0.0, 0.0, -5.0));
        spawn(&mut app, &cube, &white, Vec3::new(0.0, 0.0, 5.0));

        settle(&mut app, &[&cube], &[&white]);

        assert_eq!(
            camera_stats(&app),
            ViewDrawStats {
                draws: 1,
                culled: 1
            }
        );
    }
}
