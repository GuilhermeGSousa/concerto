use std::marker::PhantomData;

use derive_more::Deref;
use ecs::{
    component::Component,
    entity::Entity,
    resource::{Res, ResMut, Resource},
    Changed, Query,
};
use encase::{ShaderSize, UniformBuffer};
use glam::{Mat4, Vec3};
use wgpu::{
    util::DeviceExt, BindGroupDescriptor, Operations, RenderPassDepthStencilAttachment,
    RenderPassDescriptor, StoreOp, TextureView,
};

use crate::{
    components::{
        light::{push_render_light_to_gpu, LightType, RenderLight, RenderLights},
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

// Not `#[derive(Component)]`: needs a custom `on_remove` to free its slot
// (in `RenderSpotDirectionalShadowMaps` or `RenderPointShadowMaps`, depending
// on the light's type) and compact that pool, mirroring `RenderLight`
// (components/light.rs).
#[derive(Clone, Copy, Deref)]
pub struct RenderShadowCasterSlot(pub(crate) u32);

impl RenderShadowCasterSlot {
    pub(crate) fn update_slot(&mut self, new_slot: u32) {
        self.0 = new_slot;
    }
}

impl Component for RenderShadowCasterSlot {
    fn on_remove() -> Option<ecs::component::ComponentLifecycleCallback> {
        Some(|mut world, context| {
            let Some(&slot) =
                world.get_component_for_entity::<RenderShadowCasterSlot>(context.entity)
            else {
                return;
            };

            let is_point = world
                .get_component_for_entity::<RenderLight>(context.entity)
                .is_some_and(|light| light.light_type == LightType::Point.index());

            let moved_entity = if is_point {
                let Some(shadow_maps) = world.get_resource_mut::<RenderPointShadowMaps>() else {
                    return;
                };
                shadow_maps.swap_remove_caster(&slot)
            } else {
                let Some(shadow_maps) = world.get_resource_mut::<RenderSpotDirectionalShadowMaps>()
                else {
                    return;
                };
                shadow_maps.swap_remove_caster(&slot)
            };

            if let Some(render_light) =
                world.get_component_for_entity_mut::<RenderLight>(context.entity)
            {
                render_light.shadow_layer = -1;
            }
            push_render_light_to_gpu(&world, context.entity);

            if let Some(moved_entity) = moved_entity {
                if let Some(moved_shadow_slot) =
                    world.get_component_for_entity_mut::<RenderShadowCasterSlot>(moved_entity)
                {
                    moved_shadow_slot.update_slot(*slot);
                }
                if let Some(moved_light) =
                    world.get_component_for_entity_mut::<RenderLight>(moved_entity)
                {
                    moved_light.shadow_layer = *slot as i32;
                }
                push_render_light_to_gpu(&world, moved_entity);
            }
        })
    }
}

// Distinguishes the two shadow-map pool "shapes" that `ShadowMapPool<K>`
// supports. Spot and directional lights each need a single 2D view per
// caster; point lights are omnidirectional and need a full cube (6 views)
// per caster. Everything else — slot allocation, swap-remove compaction,
// grow/shrink-with-hysteresis — is identical between them, so that's all
// implemented once against this trait instead of twice.
pub(crate) trait ShadowMapKind: 'static {
    const VIEWS_PER_CASTER: u32;
    const ARRAY_VIEW_DIMENSION: wgpu::TextureViewDimension;
    const LABEL: &'static str;
}

pub(crate) struct SpotDirectionalShadowKind;

impl ShadowMapKind for SpotDirectionalShadowKind {
    const VIEWS_PER_CASTER: u32 = 1;
    const ARRAY_VIEW_DIMENSION: wgpu::TextureViewDimension = wgpu::TextureViewDimension::D2Array;
    const LABEL: &'static str = "spot_directional_shadow_maps";
}

pub(crate) struct PointShadowKind;

impl ShadowMapKind for PointShadowKind {
    const VIEWS_PER_CASTER: u32 = 6;
    const ARRAY_VIEW_DIMENSION: wgpu::TextureViewDimension = wgpu::TextureViewDimension::CubeArray;
    const LABEL: &'static str = "point_shadow_maps";
}

// Slot-allocated shadow-map depth texture — see `ShadowMapKind` for what
// varies between `RenderSpotDirectionalShadowMaps` and
// `RenderPointShadowMaps`. Slots are allocated on demand and kept packed via
// swap-remove, same as `RenderLights::slots` (components/light.rs).
// `capacity` (the actual GPU texture's per-caster view count) tracks real
// demand: it grows immediately when `slots` outgrows it, but only shrinks
// after `SHADOW_MAP_SHRINK_DELAY_FRAMES` consecutive frames of lower usage
// — see `reconcile_capacity`.
//
// Doesn't own a bind group itself — both pools are sampled together via
// `RenderLighting`'s combined `@group(2)` bind group, which is rebuilt
// whenever `reconcile_capacity` reports an actual resize.
pub(crate) struct ShadowMapPool<K: ShadowMapKind> {
    pub(crate) texture: wgpu::Texture,
    // One view per underlying texture layer: a single caster view for
    // `SpotDirectionalShadowKind`, or one cube-face view for `PointShadowKind`.
    pub(crate) views: Vec<wgpu::TextureView>,
    pub(crate) slots: Vec<Entity>,
    capacity: u32,
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
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let (texture, views) = Self::build_gpu_resources(device, 1);

        Self {
            texture,
            views,
            slots: Vec::new(),
            capacity: 1,
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
            dimension: Some(K::ARRAY_VIEW_DIMENSION),
            ..Default::default()
        })
    }

    // `None` once all `MAX_SHADOW_CASTERS` casters are in use.
    pub(crate) fn push_caster(&mut self, entity: Entity) -> Option<u32> {
        if self.slots.len() as u32 >= MAX_SHADOW_CASTERS {
            return None;
        }
        self.slots.push(entity);
        Some(self.slots.len() as u32 - 1)
    }

    pub(crate) fn swap_remove_caster(&mut self, slot: &RenderShadowCasterSlot) -> Option<Entity> {
        let index = **slot as usize;
        self.slots.swap_remove(index);
        self.slots.get(index).copied()
    }

    // Grows immediately to fit `slots`; shrinks only after
    // `SHADOW_MAP_SHRINK_DELAY_FRAMES` consecutive frames of lower usage.
    // Returns whether it actually resized this call, so callers can tell
    // whether anything referencing the old texture/views (namely
    // `RenderLighting`'s bind group) needs rebuilding too.
    pub(crate) fn reconcile_capacity(&mut self, device: &wgpu::Device) -> bool {
        let needed = (self.slots.len() as u32).max(1);

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

pub(crate) type RenderSpotDirectionalShadowMaps = ShadowMapPool<SpotDirectionalShadowKind>;
pub(crate) type RenderPointShadowMaps = ShadowMapPool<PointShadowKind>;

// Shared array of shadow view-proj matrices for *spot/directional* casters
// only, indexed by `RenderLight::shadow_layer` — read at `@group(2)
// @binding(5)` by the main lighting shader to project a fragment's world
// position into light-clip-space for shadow sampling.
//
// Deliberately separate from `RenderShadowCasterViewProj` below, which the
// shadow *depth* pass uses instead: that's a dedicated single-matrix buffer
// per light, chosen specifically so the depth pass never has to pass an
// index anywhere. This one exists only because the main pass already knows
// `shadow_layer` for the light it's iterating, so plain WGSL array indexing
// (the same pattern already used for `lights.lights[i]`) is all that's
// needed here — no per-light buffer, no indexing mechanism to invent.
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

// Reconciles both shadow-map pools' GPU texture capacity to actual demand
// once per frame, and rebuilds `RenderLighting`'s combined bind group if
// either pool actually resized. Runs after commands from
// `RenderLight::on_add`/`RenderShadowCasterSlot::on_remove` have flushed, so
// if several casters are added or removed in the same tick, this only
// resizes once for the lot — not once per entity.
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

#[derive(Component)]
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

pub(crate) fn update_shadow_view_proj(
    lights: Query<(&RenderLight, &RenderShadowCasterViewProj), Changed<RenderLight>>,
    shadow_view_projs: Res<RenderShadowViewProjs>,
    queue: Res<RenderQueue>,
) {
    for (light, view_proj) in lights.iter() {
        let matrix = shadow_view_proj(light);
        view_proj.write(&queue, matrix);

        // `shadow_layer` for a point light indexes into the *point* shadow
        // pool, not the spot/directional one this array covers — writing it
        // here regardless would clobber an unrelated spot/directional
        // light's entry sharing that same slot number.
        if light.light_type != LightType::Point.index() {
            if let Ok(slot) = u32::try_from(light.shadow_layer) {
                shadow_view_projs.write(&queue, slot, matrix);
            }
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

// Renders one depth-only pass per shadow-casting spot/directional light into
// its slot in `RenderSpotDirectionalShadowMaps`. Every mesh instance is
// redrawn into every caster with no culling — same as `material_renderpass`,
// which redraws every instance per camera today.
pub(crate) fn render_shadow_maps(
    pipeline: Res<ShadowPipeline>,
    mut device: ResMut<RenderDevice>,
    spot_directional_shadow_maps: Res<RenderSpotDirectionalShadowMaps>,
    _point_shadow_maps: Res<RenderPointShadowMaps>,
    lights: Query<(&RenderLight, &RenderShadowCasterViewProj)>,
    render_mesh_query: Query<(&RenderMeshInstance, Option<&RenderSkeletonComponent>)>,
    render_meshes: Res<RenderAssets<RenderMesh>>,
    skins: Res<SkinUniforms>,
) {
    for (light, view_proj) in lights.iter() {
        if light.light_type == LightType::Point.index() {
            // TODO: point-light (cube) shadows.
            continue;
        }

        let Ok(layer) = usize::try_from(light.shadow_layer) else {
            continue;
        };

        let Some(view) = spot_directional_shadow_maps.get_view(layer) else {
            continue;
        };

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
