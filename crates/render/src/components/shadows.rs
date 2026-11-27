use std::marker::PhantomData;

use concerto_ecs::{
    entity::Entity,
    resource::{Res, ResMut, Resource},
    Query,
};
use concerto_foundation::assets::AssetId;
use encase::{ShaderSize, UniformBuffer};
use glam::{Mat4, Vec2, Vec3};
use wgpu::{
    util::DeviceExt, BindGroupDescriptor, Operations, RenderPassDepthStencilAttachment,
    RenderPassDescriptor, StoreOp, TextureView,
};

use crate::{
    batching::{
        BatchedInstanceBuffers, BinCandidate, RenderDrawStats, ViewBatches, ViewBins, ViewDrawStats,
    },
    capabilities::RenderCapabilities,
    components::{
        camera::RenderCamera,
        light::{
            EntityOrder, LightType, RenderLight, RenderLights, RenderShadowCascades,
            RenderShadowsEnabled, ShadowCascades, VisibleLights, POINT_SHADOW_NEAR,
        },
        mesh::{RenderMeshInstanceFlags, RenderMeshInstances},
        render_entity::MainEntity,
        skeleton::{RenderSkeletonComponent, SkinUniforms},
    },
    depth_pass::{DepthPassKind, DepthPassView, DepthPassViews},
    device::RenderDevice,
    frustum::Frustum,
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

const POINT_SHADOW_MAP_SIZE: u32 = 512;

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
    const FACE_SIZE: u32;
    const LABEL: &'static str;

    fn array_view_dimension(capabilities: &RenderCapabilities) -> wgpu::TextureViewDimension;
}

pub(crate) struct SpotDirectionalShadowKind;

impl ShadowMapKind for SpotDirectionalShadowKind {
    const VIEWS_PER_CASTER: u32 = 1;
    const FACE_SIZE: u32 = SHADOW_MAP_SIZE;
    const LABEL: &'static str = "spot_directional_shadow_maps";

    fn array_view_dimension(_: &RenderCapabilities) -> wgpu::TextureViewDimension {
        wgpu::TextureViewDimension::D2Array
    }
}

pub(crate) struct PointShadowKind;

impl ShadowMapKind for PointShadowKind {
    const VIEWS_PER_CASTER: u32 = 6;
    const FACE_SIZE: u32 = POINT_SHADOW_MAP_SIZE;
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
                width: K::FACE_SIZE,
                height: K::FACE_SIZE,
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

#[derive(Clone, Copy)]
struct ShadowCandidate {
    entity: Entity,
    light_type: u32,
    enabled: bool,
    layers: u32,
}

#[derive(Debug, PartialEq)]
struct AssignedLayers {
    spot_directional: Vec<Entity>,
    point: Vec<Entity>,
}

fn assign_layers(
    lights: &[ShadowCandidate],
    spot_directional_limit: u32,
    point_limit: u32,
) -> AssignedLayers {
    let mut casters: Vec<&ShadowCandidate> = lights.iter().filter(|light| light.enabled).collect();
    casters.sort_by_key(|light| EntityOrder::of(&light.entity));

    let mut spot_directional = Vec::new();
    let mut point = Vec::new();
    for light in casters {
        let is_point = light.light_type == LightType::Point.index();
        let slots = if is_point {
            &mut point
        } else {
            &mut spot_directional
        };
        let limit = if is_point {
            point_limit
        } else {
            spot_directional_limit
        };
        if slots.len() as u32 + light.layers > limit {
            continue;
        }
        slots.extend(std::iter::repeat_n(light.entity, light.layers as usize));
    }

    AssignedLayers {
        spot_directional,
        point,
    }
}

fn layer_of(slots: &[Entity], entity: Entity) -> i32 {
    slots
        .iter()
        .position(|&slot| slot == entity)
        .map_or(-1, |layer| layer as i32)
}

fn cameras_in_order(cameras: &mut Query<(Entity, &mut RenderCamera)>) -> Vec<Entity> {
    let mut entities: Vec<Entity> = cameras.iter_mut().map(|(entity, _)| entity).collect();
    entities.sort_by_key(EntityOrder::of);
    entities
}

pub(crate) fn assign_shadow_layers(
    mut lights: Query<(Entity, &mut RenderLight, &RenderShadowsEnabled)>,
    mut cameras: Query<(Entity, &mut RenderCamera)>,
    visible: Res<VisibleLights>,
    mut spot_directional_shadow_maps: ResMut<RenderSpotDirectionalShadowMaps>,
    mut point_shadow_maps: ResMut<RenderPointShadowMaps>,
    queue: Res<RenderQueue>,
) {
    let views = cameras_in_order(&mut cameras);
    for (entity, mut camera) in cameras.iter_mut() {
        let shadow_view = views.iter().position(|&view| view == entity).unwrap_or(0) as u32;
        if camera.camera_uniform.shadow_view != shadow_view {
            camera.camera_uniform.shadow_view = shadow_view;
            camera.write_uniform(&queue);
        }
    }

    let candidates: Vec<ShadowCandidate> = lights
        .iter_mut()
        .filter(|(entity, _, _)| visible.contains(*entity))
        .map(|(entity, light, enabled)| ShadowCandidate {
            entity,
            light_type: light.light_type,
            enabled: enabled.0,
            layers: if light.light_type == LightType::Directional.index() {
                light.shadow_cascades * views.len() as u32
            } else {
                1
            },
        })
        .collect();

    let assigned = assign_layers(
        &candidates,
        spot_directional_shadow_maps.max_capacity,
        point_shadow_maps.max_capacity,
    );
    spot_directional_shadow_maps.slots = assigned.spot_directional;
    point_shadow_maps.slots = assigned.point;

    for (entity, mut light, _) in lights.iter_mut() {
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
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("shadow_view_projs"),
            contents: &identity_view_projs(),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        Self { buffer }
    }

    pub(crate) fn write(&self, queue: &wgpu::Queue, slot: u32, view_proj: Mat4) {
        queue.write_buffer(
            &self.buffer,
            view_proj_offset(slot),
            &view_proj_bytes(view_proj),
        );
    }
}

fn identity_view_projs() -> Vec<u8> {
    let mut bytes = UniformBuffer::new(Vec::new());
    bytes
        .write(&[Mat4::IDENTITY; MAX_SHADOW_CASTERS as usize])
        .unwrap();
    bytes.into_inner()
}

fn view_proj_offset(slot: u32) -> u64 {
    Mat4::SHADER_SIZE.get() * slot as u64
}

fn view_proj_bytes(view_proj: Mat4) -> Vec<u8> {
    let mut bytes = UniformBuffer::new(Vec::new());
    bytes.write(&view_proj).unwrap();
    bytes.into_inner()
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

const CASCADE_CASTER_DISTANCE: f32 = 200.0;
const CASCADE_RADIUS_STEP: f32 = 0.0625;

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
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("shadow_caster_view_proj"),
            contents: &view_proj_bytes(Mat4::IDENTITY),
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
        queue.write_buffer(&self.buffer, 0, &view_proj_bytes(view_proj));
    }

    pub(crate) fn bind_group(&self) -> &wgpu::BindGroup {
        &self.bind_group
    }
}

#[derive(Resource, Default)]
pub(crate) struct RenderShadowCasterViewProjs(Vec<RenderShadowCasterViewProj>);

#[derive(Resource, Default)]
pub(crate) struct SpotDirectionalClipFromWorld(Vec<Mat4>);

struct CascadeCamera {
    world_from_clip: Mat4,
    position: Vec3,
}

pub(crate) fn update_shadow_view_proj(
    lights: Query<(&RenderLight, &RenderShadowCascades)>,
    cameras: Query<(Entity, &RenderCamera)>,
    spot_directional_shadow_maps: Res<RenderSpotDirectionalShadowMaps>,
    shadow_view_projs: Res<RenderShadowViewProjs>,
    mut clip_from_world: ResMut<SpotDirectionalClipFromWorld>,
    queue: Res<RenderQueue>,
) {
    let mut ordered: Vec<Entity> = cameras.iter().map(|(entity, _)| entity).collect();
    ordered.sort_by_key(EntityOrder::of);
    let views: Vec<CascadeCamera> = ordered
        .iter()
        .filter_map(|&entity| cameras.get_entity(entity))
        .map(|(_, camera)| CascadeCamera {
            world_from_clip: camera.camera_uniform.inverse_view_proj(),
            position: camera.camera_uniform.view_position(),
        })
        .collect();

    let slots = &spot_directional_shadow_maps.slots;
    let mut matrices = Vec::with_capacity(slots.len());
    for (layer, &entity) in slots.iter().enumerate() {
        let Some((light, cascades)) = lights.get_entity(entity) else {
            matrices.push(Mat4::IDENTITY);
            continue;
        };
        if light.light_type != LightType::Directional.index() {
            matrices.push(light.spot_view_proj());
            continue;
        }
        let count = light.shadow_cascades.max(1) as usize;
        let offset = layer - layer_of(slots, entity).max(0) as usize;
        let matrix = views.get(offset / count).map_or(Mat4::IDENTITY, |view| {
            cascade_view_proj(light.direction, view, &cascades.0, (offset % count) as u32)
        });
        matrices.push(matrix);
    }

    for (layer, matrix) in matrices.iter().enumerate() {
        if clip_from_world.0.get(layer) != Some(matrix) {
            shadow_view_projs.write(&queue, layer as u32, *matrix);
        }
    }
    clip_from_world.0 = matrices;
}

impl RenderLight {
    fn spot_view_proj(&self) -> Mat4 {
        let direction = self.direction;
        let up = shadow_up_vector(direction);
        let translation = self.translation;
        let view = glam::camera::rh::view::look_at_mat4(translation, translation + direction, up);
        let fov = 2.0 * self.cos_cone_angle.clamp(-1.0, 1.0).acos();
        let proj = glam::camera::rh::proj::directx::perspective(
            fov,
            1.0,
            SPOT_SHADOW_NEAR,
            SPOT_SHADOW_FAR,
        );
        proj * view
    }
}

const CLIP_CORNERS: [Vec2; 4] = [
    Vec2::new(-1.0, -1.0),
    Vec2::new(1.0, -1.0),
    Vec2::new(-1.0, 1.0),
    Vec2::new(1.0, 1.0),
];

fn cascade_view_proj(
    direction: Vec3,
    camera: &CascadeCamera,
    cascades: &ShadowCascades,
    index: u32,
) -> Mat4 {
    let near_corners =
        CLIP_CORNERS.map(|corner| camera.world_from_clip.project_point3(corner.extend(0.0)));
    let far_corners =
        CLIP_CORNERS.map(|corner| camera.world_from_clip.project_point3(corner.extend(1.0)));
    let near_centre = near_corners.iter().sum::<Vec3>() / 4.0;
    let far_centre = far_corners.iter().sum::<Vec3>() / 4.0;
    let forward = (far_centre - near_centre).normalize_or_zero();
    let camera_near = (near_centre - camera.position).dot(forward);
    let camera_far = (far_centre - camera.position).dot(forward);
    let depth = (camera_far - camera_near).max(1e-6);
    let along = |distance: f32| ((distance - camera_near) / depth).clamp(0.0, 1.0);
    let start = along(cascades.near_bound(index));
    let end = along(cascades.far_bound(index)).max(start + 1e-4);

    let mut corners = [Vec3::ZERO; 8];
    for corner in 0..4 {
        let ray = far_corners[corner] - near_corners[corner];
        corners[corner] = near_corners[corner] + ray * start;
        corners[corner + 4] = near_corners[corner] + ray * end;
    }
    let centre = corners.iter().sum::<Vec3>() / 8.0;
    let reach = corners
        .iter()
        .map(|corner| corner.distance(centre))
        .fold(0.0, f32::max);
    let radius =
        ((reach / CASCADE_RADIUS_STEP).ceil() * CASCADE_RADIUS_STEP).max(CASCADE_RADIUS_STEP);

    let up = shadow_up_vector(direction);
    let light_from_world = glam::camera::rh::view::look_at_mat4(Vec3::ZERO, direction, up);
    let texel = 2.0 * radius / SHADOW_MAP_SIZE as f32;
    let mut snapped = light_from_world.transform_point3(centre);
    snapped.x = (snapped.x / texel).floor() * texel;
    snapped.y = (snapped.y / texel).floor() * texel;
    let centre = light_from_world.inverse().transform_point3(snapped);

    let eye = centre - direction * (radius + CASCADE_CASTER_DISTANCE);
    let view = glam::camera::rh::view::look_at_mat4(eye, centre, up);
    let proj = glam::camera::rh::proj::directx::orthographic(
        -radius,
        radius,
        -radius,
        radius,
        0.0,
        2.0 * radius + CASCADE_CASTER_DISTANCE,
    );
    proj * view
}

struct CubeFace {
    forward: Vec3,
    up: Vec3,
}

const CUBE_FACES: [CubeFace; 6] = [
    CubeFace {
        forward: Vec3::X,
        up: Vec3::Y,
    },
    CubeFace {
        forward: Vec3::NEG_X,
        up: Vec3::Y,
    },
    CubeFace {
        forward: Vec3::Y,
        up: Vec3::Z,
    },
    CubeFace {
        forward: Vec3::NEG_Y,
        up: Vec3::NEG_Z,
    },
    CubeFace {
        forward: Vec3::NEG_Z,
        up: Vec3::Y,
    },
    CubeFace {
        forward: Vec3::Z,
        up: Vec3::Y,
    },
];

fn point_face_view_proj(translation: Vec3, far: f32, face: usize) -> Mat4 {
    let CubeFace { forward, up } = CUBE_FACES[face];
    let view = glam::camera::rh::view::look_at_mat4(translation, translation + forward, up);
    let proj = glam::camera::rh::proj::directx::perspective(
        std::f32::consts::FRAC_PI_2,
        1.0,
        POINT_SHADOW_NEAR,
        far,
    );
    proj * view
}

#[derive(Clone, Copy, PartialEq)]
enum ShadowPool {
    SpotDirectional,
    Point,
}

#[derive(Clone, Copy)]
struct ShadowView {
    light: Entity,
    pool: ShadowPool,
    layer: usize,
}

impl ShadowView {
    fn clip_from_world(
        &self,
        light: &RenderLight,
        spot_directional: &SpotDirectionalClipFromWorld,
    ) -> Mat4 {
        match self.pool {
            ShadowPool::SpotDirectional => spot_directional
                .0
                .get(self.layer)
                .copied()
                .unwrap_or(Mat4::IDENTITY),
            ShadowPool::Point => point_face_view_proj(
                light.translation,
                light.shadow_far,
                self.layer % PointShadowKind::VIEWS_PER_CASTER as usize,
            ),
        }
    }

    fn target<'a>(
        &self,
        spot_directional: &'a RenderSpotDirectionalShadowMaps,
        point: &'a RenderPointShadowMaps,
    ) -> Option<&'a TextureView> {
        match self.pool {
            ShadowPool::SpotDirectional => spot_directional.get_view(self.layer),
            ShadowPool::Point => point.get_view(self.layer),
        }
    }
}

fn shadow_views(
    spot_directional: &RenderSpotDirectionalShadowMaps,
    point: &RenderPointShadowMaps,
) -> Vec<ShadowView> {
    let faces = PointShadowKind::VIEWS_PER_CASTER as usize;
    let spot_directional_views =
        spot_directional
            .slots
            .iter()
            .enumerate()
            .map(|(layer, &light)| ShadowView {
                light,
                pool: ShadowPool::SpotDirectional,
                layer,
            });
    let point_views = point.slots.iter().enumerate().flat_map(|(slot, &light)| {
        (0..faces).map(move |face| ShadowView {
            light,
            pool: ShadowPool::Point,
            layer: slot * faces + face,
        })
    });
    spot_directional_views.chain(point_views).collect()
}

pub(crate) fn collect_shadow_depth_views(
    spot_directional_shadow_maps: Res<RenderSpotDirectionalShadowMaps>,
    point_shadow_maps: Res<RenderPointShadowMaps>,
    clip_from_world: Res<SpotDirectionalClipFromWorld>,
    pipeline: Res<ShadowPipeline>,
    mut caster_view_projs: ResMut<RenderShadowCasterViewProjs>,
    mut depth_pass_views: ResMut<DepthPassViews>,
    lights: Query<&RenderLight>,
    device: Res<RenderDevice>,
) {
    let views = shadow_views(&spot_directional_shadow_maps, &point_shadow_maps);
    for (layer, view) in views.iter().enumerate() {
        while caster_view_projs.0.len() <= layer {
            caster_view_projs.0.push(RenderShadowCasterViewProj::new(
                &device,
                &pipeline.bind_group_layout,
            ));
        }
        let (Some(light), Some(target)) = (
            lights.get_entity(view.light),
            view.target(&spot_directional_shadow_maps, &point_shadow_maps),
        ) else {
            continue;
        };
        depth_pass_views.0.push(DepthPassView {
            kind: DepthPassKind::Shadow,
            target: target.clone(),
            view_proj: caster_view_projs.0[layer].bind_group().clone(),
            frustum: Frustum::from_clip_from_world(view.clip_from_world(light, &clip_from_world)),
            layers: None,
            rect: None,
            shadow_layer: Some(layer),
        });
    }
}

#[derive(Resource, Default)]
pub(crate) struct ShadowViewBatches(pub(crate) Vec<ViewBatches<AssetId>>);

#[allow(clippy::too_many_arguments)]
pub(crate) fn queue_shadow_meshes(
    spot_directional_shadow_maps: Res<RenderSpotDirectionalShadowMaps>,
    point_shadow_maps: Res<RenderPointShadowMaps>,
    clip_from_world: Res<SpotDirectionalClipFromWorld>,
    lights: Query<&RenderLight>,
    render_mesh_query: Query<(&MainEntity, Option<&RenderSkeletonComponent>)>,
    render_mesh_instances: Res<RenderMeshInstances>,
    buffers: Res<BatchedInstanceBuffers>,
    render_meshes: Res<RenderAssets<RenderMesh>>,
    mut shadow_view_batches: ResMut<ShadowViewBatches>,
    mut draw_stats: ResMut<RenderDrawStats>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    let views = shadow_views(&spot_directional_shadow_maps, &point_shadow_maps);
    draw_stats
        .shadow_layers
        .resize(views.len(), ViewDrawStats::default());
    for (layer, view) in views.iter().enumerate() {
        while shadow_view_batches.0.len() <= layer {
            shadow_view_batches
                .0
                .push(ViewBatches::new(&device, "Shadow View Instances Buffer"));
        }
        let batches = &mut shadow_view_batches.0[layer];

        let Some(light) = lights.get_entity(view.light) else {
            batches.bins = ViewBins::default();
            continue;
        };

        let frustum = Frustum::from_clip_from_world(view.clip_from_world(light, &clip_from_world));
        let candidates = render_mesh_query
            .iter()
            .filter_map(|(main_entity, skeleton)| {
                let instance = render_mesh_instances.get(main_entity)?;
                if !instance
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
        draw_stats.shadow_layers[layer] =
            batches
                .bins
                .rebuild(candidates, &frustum, &buffers.current_input_buffer);
        batches.upload(&device, &queue);
    }
}

pub(crate) fn render_shadow_maps(
    pipeline: Res<ShadowPipeline>,
    mut device: ResMut<RenderDevice>,
    spot_directional_shadow_maps: Res<RenderSpotDirectionalShadowMaps>,
    point_shadow_maps: Res<RenderPointShadowMaps>,
    mut caster_view_projs: ResMut<RenderShadowCasterViewProjs>,
    clip_from_world: Res<SpotDirectionalClipFromWorld>,
    queue: Res<RenderQueue>,
    lights: Query<&RenderLight>,
    render_meshes: Res<RenderAssets<RenderMesh>>,
    skins: Res<SkinUniforms>,
    shadow_view_batches: Res<ShadowViewBatches>,
) {
    for (layer, shadow_view) in shadow_views(&spot_directional_shadow_maps, &point_shadow_maps)
        .iter()
        .enumerate()
    {
        let Some(light) = lights.get_entity(shadow_view.light) else {
            continue;
        };

        let Some(view) = shadow_view.target(&spot_directional_shadow_maps, &point_shadow_maps)
        else {
            continue;
        };

        while caster_view_projs.0.len() <= layer {
            caster_view_projs.0.push(RenderShadowCasterViewProj::new(
                &device,
                &pipeline.bind_group_layout,
            ));
        }
        let view_proj = &caster_view_projs.0[layer];
        view_proj.write(&queue, shadow_view.clip_from_world(light, &clip_from_world));

        let encoder = device.command_encoder();
        let mut render_pass = encoder.begin_render_pass(&RenderPassDescriptor {
            multiview_mask: None,
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

        let Some(batches) = shadow_view_batches
            .0
            .get(layer)
            .filter(|batches| !batches.bins.batches.is_empty())
        else {
            continue;
        };

        render_pass.set_pipeline(&pipeline.pipeline);
        render_pass.set_bind_group(0, &view_proj.bind_group, &[]);
        render_pass.set_vertex_buffer(1, batches.instances.buffer().slice(..));

        for batch in &batches.bins.batches {
            let Some(mesh) = render_meshes.get(&batch.key) else {
                continue;
            };
            render_pass.set_bind_group(1, skins.bind_group(), &[batch.skeleton_offset]);
            render_pass.set_vertex_buffer(0, mesh.vertices.slice(..));
            render_pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
            render_pass.draw_indexed(0..mesh.index_count, 0, batch.instances.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        assets::{material::StandardMaterial, mesh::Mesh},
        components::{
            camera::Camera,
            light::Light,
            material::MaterialComponent,
            render_entity::{MainEntity, RenderEntity},
            visibility::NotShadowCaster,
        },
        draw_target::DrawTarget,
        gpu::Gpu,
        material_plugin::MaterialPlugin,
        plugin::RenderPlugin,
        shadow_pipeline::ShadowPipelinePlugin,
    };
    use concerto_app::{
        main_schedule::MainSchedulePlugin,
        plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
        App,
    };
    use concerto_ecs::World;
    use concerto_foundation::{
        assets::{asset_server::AssetServer, handle::AssetHandle},
        transform::Transform,
    };
    use concerto_mesh::{primitives::cuboid, MeshComponent};
    use concerto_window::plugin::{Window, WindowPlugin};
    use glam::Vec2;

    use crate::layout_test::{float_words, words};

    #[test]
    fn shadow_view_projs_are_packed_column_major_matrices() {
        let view_proj = Mat4::from_cols_array(&std::array::from_fn(|index| 1.0 + index as f32));

        assert_eq!(
            words(&view_proj_bytes(view_proj)),
            float_words(&view_proj.to_cols_array())
        );
        assert_eq!(view_proj_offset(5), 320);
        assert_eq!(
            words(&identity_view_projs()),
            float_words(&Mat4::IDENTITY.to_cols_array()).repeat(MAX_SHADOW_CASTERS as usize)
        );
    }

    struct Fit {
        case: &'static str,
        position: Vec3,
        rotation: glam::Quat,
        sun: Vec3,
    }

    fn cascade_camera(position: Vec3, rotation: glam::Quat) -> CascadeCamera {
        let view = Mat4::from_rotation_translation(rotation, position).inverse();
        let projection = glam::camera::rh::proj::directx::perspective(1.0, 1.6, 0.1, 300.0);
        CascadeCamera {
            world_from_clip: (projection * view).inverse(),
            position,
        }
    }

    #[test]
    fn a_cascade_holds_its_slice_of_the_view_and_moves_in_whole_texels() {
        let cascades = ShadowCascades::new();
        let cases = [
            Fit {
                case: "level, at the origin",
                position: Vec3::ZERO,
                rotation: glam::Quat::IDENTITY,
                sun: Vec3::new(0.3, -1.0, 0.2).normalize(),
            },
            Fit {
                case: "turned and tilted, far away",
                position: Vec3::new(812.5, 40.0, -377.0),
                rotation: glam::Quat::from_euler(glam::EulerRot::YXZ, 2.1, -0.7, 0.3),
                sun: Vec3::new(-0.8, -0.4, 0.5).normalize(),
            },
            Fit {
                case: "under a sun straight overhead",
                position: Vec3::new(-3.0, 2.0, 9.0),
                rotation: glam::Quat::from_rotation_y(-1.2),
                sun: Vec3::NEG_Y,
            },
        ];
        for case in &cases {
            let camera = cascade_camera(case.position, case.rotation);
            let nudged = cascade_camera(
                case.position + Vec3::new(0.013, 0.007, -0.021),
                case.rotation,
            );
            for index in 0..cascades.count {
                let clip_from_world = cascade_view_proj(case.sun, &camera, &cascades, index);
                for distance in [
                    cascades.near_bound(index).max(0.1),
                    cascades.far_bound(index),
                ] {
                    for corner in CLIP_CORNERS {
                        let ray = camera.world_from_clip.project_point3(corner.extend(1.0))
                            - case.position;
                        let forward = case.rotation * Vec3::NEG_Z;
                        let point = case.position + ray * (distance / ray.dot(forward));
                        let clip = clip_from_world.project_point3(point);
                        assert!(
                            clip.x.abs() <= 1.001
                                && clip.y.abs() <= 1.001
                                && (0.0..=1.0).contains(&clip.z),
                            "{}: cascade {index} leaves out {point} at {clip}",
                            case.case
                        );
                    }
                }

                let moved = cascade_view_proj(case.sun, &nudged, &cascades, index);
                let shift = (moved.project_point3(Vec3::ZERO)
                    - clip_from_world.project_point3(Vec3::ZERO))
                    * (SHADOW_MAP_SIZE as f32 / 2.0);
                assert!(
                    (shift.x - shift.x.round()).abs() < 0.02
                        && (shift.y - shift.y.round()).abs() < 0.02,
                    "{}: cascade {index} moved by {shift} texels",
                    case.case
                );
            }
        }
    }

    #[test]
    fn pool_capacity_is_capped_by_texture_array_layers() {
        assert_eq!(pool_max_capacity(true, 6, 256), 42);
        assert_eq!(pool_max_capacity(true, 6, 8192), MAX_SHADOW_CASTERS);
    }

    fn gpu() -> Option<Gpu> {
        Gpu::request_for_test()
    }

    fn app(gpu: &Gpu) -> App {
        let mut app = app_without_camera(gpu);
        app.main_mut()
            .world_mut()
            .spawn((Camera::default(), Transform::IDENTITY));
        app
    }

    fn app_without_camera(gpu: &Gpu) -> App {
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

    fn written(app: &App) -> Vec<RenderLight> {
        app.render()
            .get_resource::<RenderLights>()
            .unwrap()
            .written
            .clone()
    }

    fn render_light(app: &App, main: Entity) -> RenderLight {
        *app.render()
            .world()
            .get_component_for_entity::<RenderLight>(render_entity(app, main))
            .unwrap()
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

        assert!(written(&app) == [render_light(&app, b), render_light(&app, c)]);
    }

    fn spawn_at(app: &mut App, light: Light, translation: Vec3) -> Entity {
        app.main_mut()
            .world_mut()
            .spawn((light, Transform::from_translation(translation)))
    }

    #[test]
    fn only_lights_a_camera_sees_are_sent_and_cast_shadows() {
        let Some(gpu) = gpu() else {
            return;
        };
        let mut app = app_without_camera(&gpu);

        let lamp = || Light::point_light().with_range(5.0).with_shadows();
        let ahead = spawn_at(&mut app, lamp(), Vec3::new(0.0, 0.0, -20.0));
        let behind = spawn_at(&mut app, lamp(), Vec3::new(0.0, 0.0, 20.0));
        let sun = spawn(&mut app, Light::directional_light().with_shadows());

        app.update();
        assert_eq!(
            [ahead, behind, sun].map(|light| layer(&app, light)),
            [-1; 3],
            "with no camera"
        );
        assert!(written(&app).is_empty(), "with no camera");

        let camera = app
            .main_mut()
            .world_mut()
            .spawn((Camera::default(), Transform::IDENTITY));
        app.update();
        app.update();
        assert_eq!(point_slots(&app), vec![ahead]);
        assert_eq!(
            [ahead, behind, sun].map(|light| layer(&app, light)),
            [0, -1, 0]
        );
        assert!(written(&app) == [render_light(&app, ahead), render_light(&app, sun)]);

        *app.main_mut()
            .world_mut()
            .get_component_for_entity_mut::<Transform>(camera)
            .unwrap() = Transform::from_translation_rotation(
            Vec3::ZERO,
            glam::Quat::from_rotation_y(std::f32::consts::PI),
        );
        app.update();
        app.update();
        assert_eq!(point_slots(&app), vec![behind]);
        assert_eq!(
            [ahead, behind, sun].map(|light| layer(&app, light)),
            [-1, 0, 0]
        );
        assert!(written(&app) == [render_light(&app, behind), render_light(&app, sun)]);
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

    fn caster(entity: Entity, light_type: u32, enabled: bool) -> ShadowCandidate {
        ShadowCandidate {
            entity,
            light_type,
            enabled,
            layers: 1,
        }
    }

    fn cascaded(entity: Entity, layers: u32) -> ShadowCandidate {
        ShadowCandidate {
            layers,
            ..caster(entity, directional(), true)
        }
    }

    struct Assignment {
        case: &'static str,
        lights: Vec<ShadowCandidate>,
        spot_limit: u32,
        point_limit: u32,
        spots: Vec<Entity>,
        points: Vec<Entity>,
    }

    fn assignment(
        case: &'static str,
        lights: Vec<ShadowCandidate>,
        spots: Vec<Entity>,
    ) -> Assignment {
        Assignment {
            case,
            lights,
            spot_limit: LIMIT,
            point_limit: LIMIT,
            spots,
            points: Vec::new(),
        }
    }

    #[test]
    fn shadowed_lights_take_layers_in_entity_order_in_their_pool_up_to_its_limit() {
        let mut world = World::new();
        let [a, b, c] = [world.spawn(()), world.spawn(()), world.spawn(())];
        let cases = [
            assignment("shadows off", vec![caster(a, spot(), false)], vec![]),
            assignment("shadows on", vec![caster(a, spot(), true)], vec![a]),
            assignment(
                "a spot and a directional share a pool",
                vec![caster(a, spot(), true), caster(b, directional(), true)],
                vec![a, b],
            ),
            Assignment {
                points: vec![a],
                ..assignment(
                    "a point light in its own pool",
                    vec![caster(a, point(), true), caster(b, directional(), true)],
                    vec![b],
                )
            },
            Assignment {
                point_limit: 1,
                points: vec![a],
                ..assignment(
                    "a point limit of one keeps the first",
                    vec![
                        caster(c, point(), true),
                        caster(a, point(), true),
                        caster(b, point(), true),
                    ],
                    vec![],
                )
            },
            Assignment {
                spot_limit: 2,
                ..assignment(
                    "spots past the limit",
                    vec![
                        caster(a, spot(), true),
                        caster(b, spot(), true),
                        caster(c, spot(), true),
                    ],
                    vec![a, b],
                )
            },
            assignment(
                "a directional takes a layer for each cascade of each camera",
                vec![caster(a, spot(), true), cascaded(b, 3)],
                vec![a, b, b, b],
            ),
            Assignment {
                spot_limit: 3,
                ..assignment(
                    "cascades that do not fit are left out whole",
                    vec![
                        caster(a, spot(), true),
                        cascaded(b, 3),
                        caster(c, spot(), true),
                    ],
                    vec![a, c],
                )
            },
            assignment(
                "a shuffled order",
                vec![
                    caster(c, spot(), true),
                    caster(a, spot(), true),
                    caster(b, spot(), true),
                ],
                vec![a, b, c],
            ),
        ];
        for case in &cases {
            let assigned = assign_layers(&case.lights, case.spot_limit, case.point_limit);
            assert_eq!(assigned.spot_directional, case.spots, "{}", case.case);
            assert_eq!(assigned.point, case.points, "{}", case.case);
        }
    }

    struct CubeTexel {
        face: usize,
        uv: Vec2,
    }

    fn cube_texel(direction: Vec3) -> CubeTexel {
        let abs = direction.abs();
        let (face, sc, tc, major) = if abs.x >= abs.y && abs.x >= abs.z {
            if direction.x > 0.0 {
                (0, -direction.z, -direction.y, abs.x)
            } else {
                (1, direction.z, -direction.y, abs.x)
            }
        } else if abs.y >= abs.z {
            if direction.y > 0.0 {
                (2, direction.x, direction.z, abs.y)
            } else {
                (3, direction.x, -direction.z, abs.y)
            }
        } else if direction.z > 0.0 {
            (4, direction.x, -direction.y, abs.z)
        } else {
            (5, -direction.x, -direction.y, abs.z)
        };
        CubeTexel {
            face,
            uv: (Vec2::new(sc, tc) / major + Vec2::ONE) * 0.5,
        }
    }

    fn shader_point_depth(distance: f32, far: f32) -> f32 {
        far * (distance - POINT_SHADOW_NEAR) / ((far - POINT_SHADOW_NEAR) * distance)
    }

    #[test]
    fn each_cube_face_renders_the_texels_the_shader_samples_without_mirroring() {
        let light = Vec3::new(2.0, 3.0, -1.0);
        let far = 12.0;
        let axes = [
            Vec3::X,
            Vec3::NEG_X,
            Vec3::Y,
            Vec3::NEG_Y,
            Vec3::Z,
            Vec3::NEG_Z,
        ];
        let offsets = [Vec2::new(-0.6, 0.4), Vec2::new(0.5, -0.3), Vec2::ZERO];
        for axis in axes {
            let across = axis.any_orthonormal_vector();
            let along = axis.cross(across);
            for offset in offsets {
                let direction = (axis + across * offset.x + along * offset.y) * 4.0;
                let sampled = cube_texel(direction * Vec3::new(1.0, 1.0, -1.0));
                let clip = point_face_view_proj(light, far, sampled.face)
                    * (light + direction).extend(1.0);
                let ndc = clip.truncate() / clip.w;
                let uv = Vec2::new(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
                assert!(
                    uv.abs_diff_eq(sampled.uv, 1e-5),
                    "{direction}: face {} draws it at {uv}, the shader samples {}",
                    sampled.face,
                    sampled.uv
                );
                let distance = direction.abs().max_element();
                assert!(
                    (ndc.z - shader_point_depth(distance, far)).abs() < 1e-5,
                    "{direction}: depth {} against {}",
                    ndc.z,
                    shader_point_depth(distance, far)
                );
            }
        }

        for (face, cube_face) in CUBE_FACES.iter().enumerate() {
            let ahead = light + cube_face.forward * 3.0;
            let right = cube_face.forward.cross(cube_face.up);
            let facing_the_light = [ahead, ahead + right, ahead + cube_face.up];
            let projected = facing_the_light.map(|point| {
                let clip = point_face_view_proj(light, far, face) * point.extend(1.0);
                clip.truncate().truncate() / clip.w
            });
            let area = (projected[1] - projected[0]).perp_dot(projected[2] - projected[0]);
            assert!(area > 0.0, "face {face} mirrors its triangles");
        }
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

    fn shadow_scene(gpu: &Gpu) -> App {
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
            .register_plugin(ShadowPipelinePlugin)
            .register_plugin(MaterialPlugin::<StandardMaterial>::default());
        app.main_mut().world_mut().register_component::<Transform>();
        let sun = spawn(&mut app, Light::directional_light().with_shadows());
        app.main_mut()
            .world_mut()
            .insert(ShadowCascades::new().with_count(1), sun);
        app.main_mut()
            .world_mut()
            .spawn((Camera::default(), Transform::IDENTITY));
        app.finish_plugin_build();
        app
    }

    fn cube(app: &App) -> AssetHandle<Mesh> {
        app.get_resource::<AssetServer>()
            .unwrap()
            .add(cuboid(Vec3::ONE))
    }

    fn material(app: &App) -> AssetHandle<StandardMaterial> {
        app.get_resource::<AssetServer>()
            .unwrap()
            .add(StandardMaterial::default())
    }

    fn spawn_mesh(
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

    fn settle(app: &mut App, mesh: &AssetHandle<Mesh>) {
        let prepared = |app: &App| {
            app.render()
                .get_resource::<RenderAssets<RenderMesh>>()
                .unwrap()
                .contains(&mesh.id())
        };
        for _ in 0..100 {
            app.update();
            if prepared(app) {
                break;
            }
        }
        assert!(prepared(app));
        app.update();
        app.update();
    }

    fn shadow_stats(app: &App) -> Vec<ViewDrawStats> {
        app.render()
            .get_resource::<RenderDrawStats>()
            .unwrap()
            .shadow_layers
            .clone()
    }

    fn cast_instances(app: &App, layer: usize) -> usize {
        app.render().get_resource::<ShadowViewBatches>().unwrap().0[layer]
            .bins
            .values
            .len()
    }

    #[test]
    fn a_not_shadow_caster_is_left_out_of_every_shadow_layer() {
        let Some(gpu) = gpu() else {
            return;
        };
        let mut app = shadow_scene(&gpu);
        let (mesh, white) = (cube(&app), material(&app));
        spawn_mesh(&mut app, &mesh, &white, Vec3::new(-1.0, 0.0, 0.0));
        let hidden = spawn_mesh(&mut app, &mesh, &white, Vec3::new(1.0, 0.0, 0.0));
        app.main_mut().world_mut().insert(NotShadowCaster, hidden);

        settle(&mut app, &mesh);

        assert_eq!(
            shadow_stats(&app),
            [ViewDrawStats {
                draws: 1,
                culled: 0
            }]
        );
        assert_eq!(cast_instances(&app, 0), 1);
    }

    #[test]
    fn meshes_sharing_a_mesh_with_different_materials_cast_in_one_call() {
        let Some(gpu) = gpu() else {
            return;
        };
        let mut app = shadow_scene(&gpu);
        let mesh = cube(&app);
        let (first, second) = (material(&app), material(&app));
        spawn_mesh(&mut app, &mesh, &first, Vec3::new(-1.0, 0.0, 0.0));
        spawn_mesh(&mut app, &mesh, &second, Vec3::ZERO);
        spawn_mesh(&mut app, &mesh, &first, Vec3::new(1.0, 0.0, 0.0));

        settle(&mut app, &mesh);

        assert_eq!(
            shadow_stats(&app),
            [ViewDrawStats {
                draws: 1,
                culled: 0
            }]
        );
        assert_eq!(cast_instances(&app, 0), 3);
    }
}
