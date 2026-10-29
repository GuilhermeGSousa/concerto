use std::marker::PhantomData;

use concerto_ecs::{
    entity::Entity,
    resource::{Res, ResMut, Resource},
    Query,
};
use encase::{ShaderSize, UniformBuffer};
use glam::{Mat4, Vec3};
use wgpu::{
    util::DeviceExt, BindGroupDescriptor, Operations, RenderPassDepthStencilAttachment,
    RenderPassDescriptor, StoreOp, TextureView,
};

use crate::{
    capabilities::RenderCapabilities,
    components::{
        light::{LightType, RenderLight, RenderLights, RenderShadowsEnabled},
        mesh::RenderMeshInstance,
        skeleton::{RenderSkeletonComponent, SkinUniforms},
    },
    device::RenderDevice,
    layouts::LightingLayout,
    queue::RenderQueue,
    render_asset::{render_mesh::RenderMesh, RenderAssets},
    shadow_pipeline::ShadowPipeline,
};

// Cap on simultaneous shadow-casting lights, per pool (spot+directional
// share one pool; point lights have their own — see
// `RenderSpotDirectionalShadowMaps`/`RenderPointShadowMaps`). This is just a
// safety ceiling now, not the allocated size: each pool's actual GPU texture
// tracks real demand via `reconcile_capacity`.
pub(crate) const MAX_SHADOW_CASTERS: u32 = 128;

const SHADOW_MAP_SIZE: u32 = 1024;

// Minimum consecutive frames a pool's usage must stay below its current
// capacity before it actually shrinks. Growth is never delayed — an
// under-sized texture is a correctness bug, not just a memory tradeoff — but
// shrinking immediately would mean a texture recreation on every single
// caster removal, so this absorbs that churn (lights toggling shadows,
// respawning, etc). ~5s at 60fps; tune freely.
const SHADOW_MAP_SHRINK_DELAY_FRAMES: u32 = 300;

// Distinguishes the two shadow-map pool "shapes" that `ShadowMapPool<K>`
// supports. Spot and directional lights each need a single 2D view per
// caster; point lights are omnidirectional and need a full cube (6 views)
// per caster. Everything else — grow/shrink-with-hysteresis — is identical
// between them, so that's implemented once against this trait instead of
// twice.
pub(crate) trait ShadowMapKind: 'static {
    const VIEWS_PER_CASTER: u32;
    const LABEL: &'static str;

    fn array_view_dimension(capabilities: &RenderCapabilities) -> wgpu::TextureViewDimension;
}

pub(crate) struct SpotDirectionalShadowKind;

impl ShadowMapKind for SpotDirectionalShadowKind {
    const VIEWS_PER_CASTER: u32 = 1;
    const LABEL: &'static str = "spot_directional_shadow_maps";

    fn array_view_dimension(_: &RenderCapabilities) -> wgpu::TextureViewDimension {
        wgpu::TextureViewDimension::D2Array
    }
}

pub(crate) struct PointShadowKind;

impl ShadowMapKind for PointShadowKind {
    const VIEWS_PER_CASTER: u32 = 6;
    const LABEL: &'static str = "point_shadow_maps";

    fn array_view_dimension(capabilities: &RenderCapabilities) -> wgpu::TextureViewDimension {
        if capabilities.cube_array_textures {
            wgpu::TextureViewDimension::CubeArray
        } else {
            wgpu::TextureViewDimension::Cube
        }
    }
}

pub(crate) struct ShadowMapPool<K: ShadowMapKind> {
    pub(crate) texture: wgpu::Texture,
    // One view per underlying texture layer: a single caster view for
    // `SpotDirectionalShadowKind`, or one cube-face view for `PointShadowKind`.
    pub(crate) views: Vec<wgpu::TextureView>,
    pub(crate) slots: Vec<Entity>,
    capacity: u32,
    min_capacity: u32,
    max_capacity: u32,
    array_view_dimension: wgpu::TextureViewDimension,
    frames_below_capacity: u32,
    _kind: PhantomData<fn() -> K>,
}

// Manual impl, like `MaterialPipeline<M>` (material_plugin.rs) —
// #[derive(Resource)] doesn't handle generics/`PhantomData`.
impl<K: ShadowMapKind> Resource for ShadowMapPool<K> {
    fn name() -> &'static str {
        std::any::type_name::<Self>()
    }
}

impl<K: ShadowMapKind> ShadowMapPool<K> {
    pub(crate) fn new(device: &wgpu::Device, capabilities: &RenderCapabilities) -> Self {
        let array_view_dimension = K::array_view_dimension(capabilities);
        let is_array = matches!(
            array_view_dimension,
            wgpu::TextureViewDimension::D2Array | wgpu::TextureViewDimension::CubeArray
        );

        let max_capacity = pool_max_capacity(
            is_array,
            K::VIEWS_PER_CASTER,
            device.limits().max_texture_array_layers,
        );
        let min_capacity = if is_array && !capabilities.single_layer_texture_arrays {
            2
        } else {
            1
        };

        let (texture, views) = Self::build_gpu_resources(device, min_capacity);

        Self {
            texture,
            views,
            slots: Vec::new(),
            capacity: min_capacity,
            min_capacity,
            max_capacity,
            array_view_dimension,
            frames_below_capacity: 0,
            _kind: PhantomData,
        }
    }

    fn build_gpu_resources(
        device: &wgpu::Device,
        capacity: u32,
    ) -> (wgpu::Texture, Vec<wgpu::TextureView>) {
        let view_count = capacity * K::VIEWS_PER_CASTER;

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(K::LABEL),
            size: wgpu::Extent3d {
                width: SHADOW_MAP_SIZE,
                height: SHADOW_MAP_SIZE,
                depth_or_array_layers: view_count,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        // One single-layer view per underlying texture layer, to target as
        // a depth attachment when rendering into it.
        let views = (0..view_count)
            .map(|layer| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some(K::LABEL),
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: layer,
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();

        (texture, views)
    }

    // Sampled as a whole by `RenderLighting`'s combined bind group. Texture
    // views aren't separate GPU allocations, so building a fresh one here on
    // demand (rather than caching it) is cheap.
    pub(crate) fn array_view(&self) -> wgpu::TextureView {
        self.texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some(K::LABEL),
            dimension: Some(self.array_view_dimension),
            ..Default::default()
        })
    }

    // Grows immediately to fit `slots`; shrinks only after
    // `SHADOW_MAP_SHRINK_DELAY_FRAMES` consecutive frames of lower usage.
    // Returns whether it actually resized this call, so callers can tell
    // whether anything referencing the old texture/views (namely
    // `RenderLighting`'s bind group) needs rebuilding too.
    pub(crate) fn reconcile_capacity(&mut self, device: &wgpu::Device) -> bool {
        let needed = (self.slots.len() as u32).max(self.min_capacity);

        if needed > self.capacity {
            self.resize_to(device, needed);
            true
        } else if needed < self.capacity {
            self.frames_below_capacity += 1;
            if self.frames_below_capacity >= SHADOW_MAP_SHRINK_DELAY_FRAMES {
                self.resize_to(device, needed);
                true
            } else {
                false
            }
        } else {
            self.frames_below_capacity = 0;
            false
        }
    }

    pub(crate) fn get_view(&self, index: usize) -> Option<&TextureView> {
        self.views.get(index)
    }

    fn resize_to(&mut self, device: &wgpu::Device, capacity: u32) {
        let (texture, views) = Self::build_gpu_resources(device, capacity);
        self.texture = texture;
        self.views = views;
        self.capacity = capacity;
        self.frames_below_capacity = 0;
    }
}

fn pool_max_capacity(is_array: bool, views_per_caster: u32, max_texture_array_layers: u32) -> u32 {
    if is_array {
        MAX_SHADOW_CASTERS.min(max_texture_array_layers / views_per_caster)
    } else {
        1
    }
}

pub(crate) type RenderSpotDirectionalShadowMaps = ShadowMapPool<SpotDirectionalShadowKind>;
pub(crate) type RenderPointShadowMaps = ShadowMapPool<PointShadowKind>;

fn assign_layers(
    lights: &[(Entity, u32, bool)],
    spot_directional_limit: u32,
    point_limit: u32,
) -> (Vec<Entity>, Vec<Entity>) {
    let mut spot_directional = Vec::new();
    let mut point = Vec::new();
    for &(entity, light_type, enabled) in lights {
        if !enabled {
            continue;
        }
        if light_type == LightType::Point.index() {
            point.push(entity);
        } else {
            spot_directional.push(entity);
        }
    }

    for (casters, limit) in [
        (&mut spot_directional, spot_directional_limit),
        (&mut point, point_limit),
    ] {
        casters.sort_by_key(|entity| (entity.index(), entity.generation()));
        casters.truncate(limit as usize);
    }

    (spot_directional, point)
}

fn layer_of(slots: &[Entity], entity: Entity) -> i32 {
    slots
        .iter()
        .position(|&slot| slot == entity)
        .map_or(-1, |layer| layer as i32)
}

pub(crate) fn assign_shadow_layers(
    lights: Query<(Entity, &mut RenderLight, &RenderShadowsEnabled)>,
    mut spot_directional_shadow_maps: ResMut<RenderSpotDirectionalShadowMaps>,
    mut point_shadow_maps: ResMut<RenderPointShadowMaps>,
) {
    let casters: Vec<(Entity, u32, bool)> = lights
        .iter()
        .map(|(entity, light, enabled)| (entity, light.light_type, enabled.0))
        .collect();

    let (spot_directional, point) = assign_layers(
        &casters,
        spot_directional_shadow_maps.max_capacity,
        point_shadow_maps.max_capacity,
    );
    spot_directional_shadow_maps.slots = spot_directional;
    point_shadow_maps.slots = point;

    for (entity, mut light, _) in lights.iter() {
        let slots = if light.light_type == LightType::Point.index() {
            &point_shadow_maps.slots
        } else {
            &spot_directional_shadow_maps.slots
        };
        let layer = layer_of(slots, entity);
        if light.shadow_layer != layer {
            light.shadow_layer = layer;
        }
    }
}

// Shared array of shadow view-proj matrices for *spot/directional* casters
// only, indexed by `RenderLight::shadow_layer` — read at `@group(2)
// @binding(5)` by the main lighting shader to project a fragment's world
// position into light-clip-space for shadow sampling.
#[derive(Resource)]
pub(crate) struct RenderShadowViewProjs {
    buffer: wgpu::Buffer,
}

impl RenderShadowViewProjs {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let mut bytes = UniformBuffer::new(Vec::new());
        bytes
            .write(&[Mat4::IDENTITY; MAX_SHADOW_CASTERS as usize])
            .unwrap();

        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("shadow_view_projs"),
            contents: &bytes.into_inner(),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        Self { buffer }
    }

    pub(crate) fn write(&self, queue: &wgpu::Queue, slot: u32, view_proj: Mat4) {
        let offset = Mat4::SHADER_SIZE.get() * slot as u64;
        let mut bytes = UniformBuffer::new(Vec::new());
        bytes.write(&view_proj).unwrap();
        queue.write_buffer(&self.buffer, offset, &bytes.into_inner());
    }
}

// The combined `@group(2)` bind group consumed by any material with
// `needs_lighting() == true` — the lights uniform, both shadow-map arrays,
// and the spot/directional shadow view-proj array, merged into one group
// (see `LightingLayout`'s doc comment for why). Rebuilt whenever either
// shadow pool actually resizes (see `resize_shadow_maps`) — the lights and
// shadow-view-proj buffers never resize, so they never force a rebuild on
// their own.
#[derive(Resource)]
pub(crate) struct RenderLighting {
    pub(crate) bind_group: wgpu::BindGroup,
}

impl RenderLighting {
    pub(crate) fn new(
        device: &wgpu::Device,
        layout: &LightingLayout,
        lights: &RenderLights,
        spot_directional_shadow_maps: &RenderSpotDirectionalShadowMaps,
        point_shadow_maps: &RenderPointShadowMaps,
        shadow_view_projs: &RenderShadowViewProjs,
    ) -> Self {
        Self {
            bind_group: Self::build_bind_group(
                device,
                layout,
                lights,
                spot_directional_shadow_maps,
                point_shadow_maps,
                shadow_view_projs,
            ),
        }
    }

    pub(crate) fn rebuild(
        &mut self,
        device: &wgpu::Device,
        layout: &LightingLayout,
        lights: &RenderLights,
        spot_directional_shadow_maps: &RenderSpotDirectionalShadowMaps,
        point_shadow_maps: &RenderPointShadowMaps,
        shadow_view_projs: &RenderShadowViewProjs,
    ) {
        self.bind_group = Self::build_bind_group(
            device,
            layout,
            lights,
            spot_directional_shadow_maps,
            point_shadow_maps,
            shadow_view_projs,
        );
    }

    fn build_bind_group(
        device: &wgpu::Device,
        layout: &LightingLayout,
        lights: &RenderLights,
        spot_directional_shadow_maps: &RenderSpotDirectionalShadowMaps,
        point_shadow_maps: &RenderPointShadowMaps,
        shadow_view_projs: &RenderShadowViewProjs,
    ) -> wgpu::BindGroup {
        let spot_directional_view = spot_directional_shadow_maps.array_view();
        let point_view = point_shadow_maps.array_view();

        // Both pools use identical comparison-sampler settings, so one
        // shared sampler covers both shadow bindings.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("lighting_shadow_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });

        device.create_bind_group(&BindGroupDescriptor {
            label: Some("lighting_bind_group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: lights.buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&spot_directional_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&point_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: shadow_view_projs.buffer.as_entire_binding(),
                },
            ],
        })
    }
}

pub(crate) fn resize_shadow_maps(
    mut spot_directional_shadow_maps: ResMut<RenderSpotDirectionalShadowMaps>,
    mut point_shadow_maps: ResMut<RenderPointShadowMaps>,
    mut lighting: ResMut<RenderLighting>,
    device: Res<RenderDevice>,
    lighting_layout: Res<LightingLayout>,
    lights: Res<RenderLights>,
    shadow_view_projs: Res<RenderShadowViewProjs>,
) {
    let spot_directional_resized = spot_directional_shadow_maps.reconcile_capacity(&device);
    let point_resized = point_shadow_maps.reconcile_capacity(&device);

    if spot_directional_resized || point_resized {
        lighting.rebuild(
            &device,
            &lighting_layout,
            &lights,
            &spot_directional_shadow_maps,
            &point_shadow_maps,
            &shadow_view_projs,
        );
    }
}

const DIRECTIONAL_SHADOW_DISTANCE: f32 = 50.0;
const DIRECTIONAL_SHADOW_HALF_EXTENT: f32 = 50.0;
const DIRECTIONAL_SHADOW_NEAR: f32 = 0.1;
const DIRECTIONAL_SHADOW_FAR: f32 = 200.0;

// Spot lights have no explicit range in this engine yet, so this is just a
// generous fixed far plane rather than something derived from the light.
const SPOT_SHADOW_NEAR: f32 = 0.1;
const SPOT_SHADOW_FAR: f32 = 100.0;

// `Vec3::Y` degenerates as an up vector once `direction` is nearly parallel
// to it (straight up/down directional or spot lights), so fall back to Z.
fn shadow_up_vector(direction: Vec3) -> Vec3 {
    if direction.dot(Vec3::Y).abs() > 0.999 {
        Vec3::Z
    } else {
        Vec3::Y
    }
}

pub(crate) struct RenderShadowCasterViewProj {
    buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl RenderShadowCasterViewProj {
    pub(crate) fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout) -> Self {
        let mut bytes = UniformBuffer::new(Vec::new());
        bytes.write(&Mat4::IDENTITY).unwrap();

        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("shadow_caster_view_proj"),
            contents: &bytes.into_inner(),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("shadow_caster_view_proj_bind_group"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });

        Self { buffer, bind_group }
    }

    pub(crate) fn write(&self, queue: &wgpu::Queue, view_proj: Mat4) {
        let mut bytes = UniformBuffer::new(Vec::new());
        bytes.write(&view_proj).unwrap();
        queue.write_buffer(&self.buffer, 0, &bytes.into_inner());
    }
}

#[derive(Resource, Default)]
pub(crate) struct RenderShadowCasterViewProjs(Vec<RenderShadowCasterViewProj>);

pub(crate) fn update_shadow_view_proj(
    lights: Query<&RenderLight>,
    spot_directional_shadow_maps: Res<RenderSpotDirectionalShadowMaps>,
    shadow_view_projs: Res<RenderShadowViewProjs>,
    queue: Res<RenderQueue>,
) {
    for (layer, &entity) in spot_directional_shadow_maps.slots.iter().enumerate() {
        if let Some(light) = lights.get_entity(entity) {
            shadow_view_projs.write(&queue, layer as u32, shadow_view_proj(light));
        }
    }
}

fn shadow_view_proj(light: &RenderLight) -> Mat4 {
    let up = shadow_up_vector(light.direction);

    if light.light_type == LightType::Directional.index() {
        let eye = -light.direction * DIRECTIONAL_SHADOW_DISTANCE;
        let view = Mat4::look_at_rh(eye, eye + light.direction, up);
        let half = DIRECTIONAL_SHADOW_HALF_EXTENT;
        let proj = Mat4::orthographic_rh(
            -half,
            half,
            -half,
            half,
            DIRECTIONAL_SHADOW_NEAR,
            DIRECTIONAL_SHADOW_FAR,
        );
        proj * view
    } else {
        let view = Mat4::look_at_rh(light.translation, light.translation + light.direction, up);
        let fov = 2.0 * light.cos_cone_angle.clamp(-1.0, 1.0).acos();
        let proj = Mat4::perspective_rh(fov, 1.0, SPOT_SHADOW_NEAR, SPOT_SHADOW_FAR);
        proj * view
    }
}

// Renders one depth-only pass per assigned layer of
// `RenderSpotDirectionalShadowMaps`, into that layer. Every mesh instance is
// redrawn into every caster with no culling — same as `material_renderpass`,
// which redraws every instance per camera today.
pub(crate) fn render_shadow_maps(
    pipeline: Res<ShadowPipeline>,
    mut device: ResMut<RenderDevice>,
    spot_directional_shadow_maps: Res<RenderSpotDirectionalShadowMaps>,
    _point_shadow_maps: Res<RenderPointShadowMaps>,
    mut caster_view_projs: ResMut<RenderShadowCasterViewProjs>,
    queue: Res<RenderQueue>,
    lights: Query<&RenderLight>,
    render_mesh_query: Query<(&RenderMeshInstance, Option<&RenderSkeletonComponent>)>,
    render_meshes: Res<RenderAssets<RenderMesh>>,
    skins: Res<SkinUniforms>,
) {
    for (layer, &entity) in spot_directional_shadow_maps.slots.iter().enumerate() {
        let Some(light) = lights.get_entity(entity) else {
            continue;
        };

        let Some(view) = spot_directional_shadow_maps.get_view(layer) else {
            continue;
        };

        while caster_view_projs.0.len() <= layer {
            caster_view_projs.0.push(RenderShadowCasterViewProj::new(
                &device,
                &pipeline.bind_group_layout,
            ));
        }
        let view_proj = &caster_view_projs.0[layer];
        view_proj.write(&queue, shadow_view_proj(light));

        let encoder = device.command_encoder();
        let mut render_pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("Shadow Depth Render Pass"),
            color_attachments: &[],
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view,
                depth_ops: Some(Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        render_pass.set_pipeline(&pipeline.pipeline);
        render_pass.set_bind_group(0, &view_proj.bind_group, &[]);

        for (mesh_instance, skeleton) in render_mesh_query.iter() {
            if let Some(mesh) = render_meshes.get(&mesh_instance.mesh_asset_id) {
                let offset = skeleton.map_or(0, |sk| sk.offset);
                render_pass.set_bind_group(1, skins.bind_group(), &[offset]);

                render_pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                render_pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                render_pass.set_vertex_buffer(1, mesh_instance.transform.slice(..));
                render_pass.draw_indexed(0..mesh.index_count, 0, 0..1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        components::{
            light::{Light, RenderLightSlot},
            render_entity::{MainEntity, RenderEntity},
        },
        draw_target::DrawTarget,
        gpu::Gpu,
        plugin::RenderPlugin,
        shadow_pipeline::ShadowPipelinePlugin,
    };
    use concerto_app::{
        main_schedule::MainSchedulePlugin,
        plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
        App,
    };
    use concerto_ecs::World;
    use concerto_foundation::transform::Transform;
    use concerto_window::plugin::{Window, WindowPlugin};

    #[test]
    fn pool_capacity_is_capped_by_texture_array_layers() {
        assert_eq!(pool_max_capacity(true, 6, 256), 42);
        assert_eq!(pool_max_capacity(true, 1, 256), 128);
        assert_eq!(pool_max_capacity(true, 6, 8192), MAX_SHADOW_CASTERS);
        assert_eq!(pool_max_capacity(false, 6, 256), 1);
    }

    fn gpu() -> Option<Gpu> {
        Gpu::request_for_test()
    }

    fn app(gpu: &Gpu) -> App {
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
            .register_plugin(ShadowPipelinePlugin);
        app.main_mut().world_mut().register_component::<Transform>();
        app.finish_plugin_build();
        app
    }

    fn spawn(app: &mut App, light: Light) -> Entity {
        app.main_mut()
            .world_mut()
            .spawn((light, Transform::IDENTITY))
    }

    fn render_entity(app: &App, main: Entity) -> Entity {
        **app
            .main()
            .world()
            .get_component_for_entity::<RenderEntity>(main)
            .unwrap()
    }

    fn layer(app: &App, main: Entity) -> i32 {
        app.render()
            .world()
            .get_component_for_entity::<RenderLight>(render_entity(app, main))
            .unwrap()
            .shadow_layer
    }

    fn to_main(app: &App, slots: &[Entity]) -> Vec<Entity> {
        slots
            .iter()
            .map(|&render_entity| {
                **app
                    .render()
                    .world()
                    .get_component_for_entity::<MainEntity>(render_entity)
                    .unwrap()
            })
            .collect()
    }

    fn spot_slots(app: &App) -> Vec<Entity> {
        let pool = app
            .render()
            .get_resource::<RenderSpotDirectionalShadowMaps>()
            .unwrap();
        to_main(app, &pool.slots)
    }

    fn point_slots(app: &App) -> Vec<Entity> {
        let pool = app
            .render()
            .get_resource::<RenderPointShadowMaps>()
            .unwrap();
        to_main(app, &pool.slots)
    }

    fn spot_capacity(app: &App) -> u32 {
        app.render()
            .get_resource::<RenderSpotDirectionalShadowMaps>()
            .unwrap()
            .capacity
    }

    fn edit(app: &mut App, main: Entity, change: impl FnOnce(&mut Light)) {
        change(
            app.main_mut()
                .world_mut()
                .get_component_for_entity_mut::<Light>(main)
                .unwrap(),
        );
    }

    #[test]
    fn ticking_shadows_after_spawn_gives_the_light_a_layer() {
        let Some(gpu) = gpu() else {
            return;
        };
        let mut app = app(&gpu);
        let light = spawn(&mut app, Light::spot_light(0.6));

        app.update();
        app.update();
        assert_eq!(layer(&app, light), -1);
        assert!(spot_slots(&app).is_empty());

        edit(&mut app, light, |light| light.shadowmaps_enabled = true);
        app.update();
        assert_eq!(layer(&app, light), 0);
        assert_eq!(spot_slots(&app), vec![light]);
    }

    #[test]
    fn turning_shadows_off_frees_the_layer_at_once() {
        let Some(gpu) = gpu() else {
            return;
        };
        let mut app = app(&gpu);
        let a = spawn(&mut app, Light::spot_light(0.6).with_shadows());
        let b = spawn(&mut app, Light::spot_light(0.6).with_shadows());
        let c = spawn(&mut app, Light::spot_light(0.6).with_shadows());

        app.update();
        assert_eq!([layer(&app, a), layer(&app, b), layer(&app, c)], [0, 1, 2]);
        assert_eq!(spot_capacity(&app), 3);

        edit(&mut app, a, |light| light.shadowmaps_enabled = false);
        app.update();
        assert_eq!([layer(&app, a), layer(&app, b), layer(&app, c)], [-1, 0, 1]);
        assert_eq!(spot_slots(&app), vec![b, c]);
        assert_eq!(spot_capacity(&app), 3);

        edit(&mut app, a, |light| light.shadowmaps_enabled = true);
        app.update();
        assert_eq!([layer(&app, a), layer(&app, b), layer(&app, c)], [0, 1, 2]);
    }

    #[test]
    fn switching_a_shadowed_spot_to_point_moves_it_between_pools() {
        let Some(gpu) = gpu() else {
            return;
        };
        let mut app = app(&gpu);
        let a = spawn(&mut app, Light::spot_light(0.6).with_shadows());
        let b = spawn(&mut app, Light::directional_light().with_shadows());

        app.update();
        assert_eq!(spot_slots(&app), vec![a, b]);

        edit(&mut app, a, |light| light.light_type = LightType::Point);
        app.update();
        assert_eq!(spot_slots(&app), vec![b]);
        assert_eq!(layer(&app, b), 0);
        assert_eq!(point_slots(&app), vec![a]);
        assert_eq!(layer(&app, a), 0);

        edit(&mut app, a, |light| {
            light.light_type = LightType::Spot { cone_angle: 0.6 }
        });
        app.update();
        assert_eq!(spot_slots(&app), vec![a, b]);
        assert_eq!([layer(&app, a), layer(&app, b)], [0, 1]);
        assert!(point_slots(&app).is_empty());
    }

    #[test]
    fn a_despawned_caster_frees_its_layer() {
        let Some(gpu) = gpu() else {
            return;
        };
        let mut app = app(&gpu);
        let a = spawn(&mut app, Light::spot_light(0.6).with_shadows());
        let b = spawn(&mut app, Light::spot_light(0.6).with_shadows());
        let c = spawn(&mut app, Light::spot_light(0.6).with_shadows());
        app.update();

        app.main_mut().world_mut().despawn(a);
        app.update();
        assert_eq!([layer(&app, b), layer(&app, c)], [0, 1]);

        let lights = app.render().get_resource::<RenderLights>().unwrap();
        assert_eq!(lights.slots.len(), 2);
        for main in [b, c] {
            let render_entity = render_entity(&app, main);
            let slot = **app
                .render()
                .world()
                .get_component_for_entity::<RenderLightSlot>(render_entity)
                .unwrap();
            assert_eq!(lights.slots[slot as usize], render_entity);
        }
    }

    fn entities() -> (Entity, Entity, Entity) {
        let mut world = World::new();
        (world.spawn(()), world.spawn(()), world.spawn(()))
    }

    fn spot() -> u32 {
        LightType::Spot { cone_angle: 0.6 }.index()
    }

    fn directional() -> u32 {
        LightType::Directional.index()
    }

    fn point() -> u32 {
        LightType::Point.index()
    }

    const LIMIT: u32 = MAX_SHADOW_CASTERS;

    #[test]
    fn a_light_ticked_on_gets_layer_zero() {
        let (a, _, _) = entities();

        let (spots, points) = assign_layers(&[(a, spot(), false)], LIMIT, LIMIT);
        assert!(spots.is_empty());
        assert!(points.is_empty());
        assert_eq!(layer_of(&spots, a), -1);

        let (spots, points) = assign_layers(&[(a, spot(), true)], LIMIT, LIMIT);
        assert_eq!(spots, vec![a]);
        assert!(points.is_empty());
        assert_eq!(layer_of(&spots, a), 0);
    }

    #[test]
    fn turning_shadows_off_gives_no_layer() {
        let (a, b, _) = entities();

        let (spots, _) = assign_layers(&[(a, spot(), true), (b, spot(), true)], LIMIT, LIMIT);
        assert_eq!(spots, vec![a, b]);

        let (spots, _) = assign_layers(&[(a, spot(), false), (b, spot(), true)], LIMIT, LIMIT);
        assert_eq!(spots, vec![b]);
        assert_eq!(layer_of(&spots, a), -1);
        assert_eq!(layer_of(&spots, b), 0);
    }

    #[test]
    fn spot_to_point_moves_the_light_between_pools() {
        let (a, b, _) = entities();

        let (spots, points) =
            assign_layers(&[(a, spot(), true), (b, directional(), true)], LIMIT, LIMIT);
        assert_eq!(spots, vec![a, b]);
        assert!(points.is_empty());

        let (spots, points) = assign_layers(
            &[(a, point(), true), (b, directional(), true)],
            LIMIT,
            LIMIT,
        );
        assert_eq!(spots, vec![b]);
        assert_eq!(points, vec![a]);
        assert_eq!(layer_of(&spots, b), 0);
        assert_eq!(layer_of(&points, a), 0);

        let (spots, points) =
            assign_layers(&[(a, spot(), true), (b, directional(), true)], LIMIT, LIMIT);
        assert_eq!(spots, vec![a, b]);
        assert!(points.is_empty());
    }

    #[test]
    fn spot_and_directional_share_the_spot_pool() {
        let (a, _, _) = entities();

        for light_type in [spot(), directional()] {
            let (spots, points) = assign_layers(&[(a, light_type, true)], LIMIT, LIMIT);
            assert_eq!(spots, vec![a]);
            assert!(points.is_empty());
        }
    }

    #[test]
    fn the_point_limit_of_one_keeps_the_first_point_light() {
        let (a, b, c) = entities();

        let (_, points) = assign_layers(
            &[(c, point(), true), (a, point(), true), (b, point(), true)],
            LIMIT,
            1,
        );
        assert_eq!(points, vec![a]);
        assert_eq!(layer_of(&points, b), -1);
        assert_eq!(layer_of(&points, c), -1);
    }

    #[test]
    fn spot_casters_past_the_limit_get_no_layer() {
        let (a, b, c) = entities();

        let (spots, _) = assign_layers(
            &[(a, spot(), true), (b, spot(), true), (c, spot(), true)],
            2,
            LIMIT,
        );
        assert_eq!(spots, vec![a, b]);
        assert_eq!(layer_of(&spots, c), -1);
    }

    #[test]
    fn the_order_is_stable() {
        let (a, b, c) = entities();
        let shuffled = [(c, spot(), true), (a, spot(), true), (b, spot(), true)];
        let rotated = [(b, spot(), true), (c, spot(), true), (a, spot(), true)];

        assert_eq!(assign_layers(&shuffled, LIMIT, LIMIT).0, vec![a, b, c]);
        assert_eq!(assign_layers(&rotated, LIMIT, LIMIT).0, vec![a, b, c]);
        assert_eq!(
            assign_layers(&shuffled, LIMIT, LIMIT),
            assign_layers(&shuffled, LIMIT, LIMIT)
        );
    }

    #[test]
    fn each_layer_keeps_one_view_proj_buffer() {
        let Some(gpu) = gpu() else {
            return;
        };
        let mut app = app(&gpu);
        let first = spawn(&mut app, Light::spot_light(0.6).with_shadows());
        spawn(&mut app, Light::spot_light(0.6).with_shadows());

        for _ in 0..3 {
            app.update();
        }
        assert_eq!(buffer_count(&app), 2);

        edit(&mut app, first, |light| light.shadowmaps_enabled = false);
        app.update();
        assert_eq!(buffer_count(&app), 2);

        edit(&mut app, first, |light| light.shadowmaps_enabled = true);
        app.update();
        assert_eq!(buffer_count(&app), 2);
    }

    fn buffer_count(app: &App) -> usize {
        app.render()
            .get_resource::<RenderShadowCasterViewProjs>()
            .unwrap()
            .0
            .len()
    }
}
