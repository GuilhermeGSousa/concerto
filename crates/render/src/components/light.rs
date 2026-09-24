use concerto_app::extractor::Extracted;
use concerto_color::{Color, LinearRgba};
use concerto_ecs::{
    command::CommandQueue,
    component::scene::{SceneComponent, SceneSpawnContext},
    component::Component,
    entity::Entity,
    query::Query,
    resource::{Res, Resource},
    Changed,
};
use derive_more::Deref;
use serde::{Deserialize, Serialize};

use concerto_core::transform::GlobalTransform;
use encase::{ShaderSize, ShaderType, UniformBuffer};
use glam::Vec3;
use wgpu::{util::DeviceExt, Buffer};

use crate::{
    components::{
        render_entity::RenderEntity,
        shadows::{
            RenderPointShadowMaps, RenderShadowCasterSlot, RenderShadowCasterViewProj,
            RenderSpotDirectionalShadowMaps,
        },
    },
    device::RenderDevice,
    queue::RenderQueue,
    shadow_pipeline::ShadowPipeline,
};

const MAX_LIGHTS: usize = 128;

/// Sentinel [`RenderLight::shadow_layer`] value set by [`light_added`] to ask
/// [`RenderLight::on_add`] to allocate a shadow-caster slot. Never observed
/// outside that same tick — resolved to either a real layer index or `-1`.
const SHADOW_LAYER_REQUESTED: i32 = -2;

#[derive(Component, Serialize, Deserialize)]
pub struct Light {
    pub color: Color,
    pub intensity: f32,
    pub shadowmaps_enabled: bool,
    pub light_type: LightType,
}

impl SceneComponent for Light {
    fn apply(self, entity: Entity, ctx: &mut SceneSpawnContext<'_>) {
        ctx.insert(self, entity);
    }
}

impl Light {
    pub fn point_light() -> Self {
        Self {
            color: Color::WHITE,
            intensity: 1.0,
            shadowmaps_enabled: false,
            light_type: LightType::Point,
        }
    }

    pub fn spot_light(cone_angle: f32) -> Self {
        Self {
            color: Color::WHITE,
            intensity: 1.0,
            shadowmaps_enabled: false,
            light_type: LightType::Spot { cone_angle },
        }
    }

    pub fn directional_light() -> Self {
        Self {
            color: Color::WHITE,
            intensity: 1.0,
            shadowmaps_enabled: false,
            light_type: LightType::Directional,
        }
    }

    pub fn with_intensity(mut self, intensity: f32) -> Self {
        self.intensity = intensity;
        self
    }

    pub fn with_color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    pub fn with_shadows(mut self) -> Self {
        self.shadowmaps_enabled = true;
        self
    }
}

#[derive(Serialize, Deserialize)]
pub enum LightType {
    Point,
    Spot { cone_angle: f32 },
    Directional,
}

impl LightType {
    pub fn index(&self) -> u32 {
        match *self {
            LightType::Point => 0,
            LightType::Spot { .. } => 1,
            LightType::Directional => 2,
        }
    }
}

#[derive(ShaderType)]
pub(crate) struct LightsUniform {
    pub(crate) lights: [RenderLight; MAX_LIGHTS],
    pub(crate) light_count: i32,
}

#[derive(Component, Clone, Copy, Deref)]
pub struct RenderLightSlot(u32);

impl RenderLightSlot {
    pub(crate) fn update_slot(&mut self, new_slot: u32) {
        self.0 = new_slot;
    }
}

// Re-uploads `entity`'s `RenderLight` immediately. Needed anywhere a
// `RenderLight` field is mutated through `RestrictedWorld` (component
// lifecycle callbacks) rather than a `Query<&mut RenderLight>` — the former
// bypasses `Mut`'s change-tick marking, so `update_changed_lights`'s
// `Changed<RenderLight>` filter would never pick the write up otherwise.
// `pub(crate)`: also called from `RenderShadowCasterSlot::on_remove`
// (components/shadows.rs).
pub(crate) fn push_render_light_to_gpu(
    world: &concerto_ecs::world::RestrictedWorld<'_>,
    entity: Entity,
) {
    if let (Some(render_light), Some(render_light_slot), Some(lights), Some(queue)) = (
        world.get_component_for_entity::<RenderLight>(entity),
        world.get_component_for_entity::<RenderLightSlot>(entity),
        world.get_resource::<RenderLights>(),
        world.get_resource::<RenderQueue>(),
    ) {
        lights.write_buffer(queue, render_light, *render_light_slot);
    }
}

#[derive(ShaderType, Clone, Copy)]
pub struct RenderLight {
    pub(crate) translation: Vec3,
    pub(crate) intensity: f32,
    pub(crate) color: LinearRgba,
    pub(crate) direction: Vec3,
    pub(crate) light_type: u32,

    // Spotlight
    pub(crate) cos_cone_angle: f32,
    pub(crate) shadow_layer: i32,
}

impl RenderLight {
    pub(crate) fn zeroed() -> Self {
        Self {
            translation: Vec3::ZERO,
            intensity: 0.0,
            color: LinearRgba::TRANSPARENT,
            direction: Vec3::ZERO,
            light_type: 0,
            cos_cone_angle: 0.0,
            shadow_layer: -1,
        }
    }
}

impl Component for RenderLight {
    fn on_add() -> Option<concerto_ecs::component::ComponentLifecycleCallback> {
        Some(|mut world, context| {
            let slot = if let Some(lights) = world.get_resource_mut::<RenderLights>() {
                lights.push_light(context.entity);
                RenderLightSlot(lights.len() as u32 - 1)
            } else {
                return;
            };

            world.insert(slot, context.entity);

            let casts_shadows = world
                .get_component_for_entity::<RenderLight>(context.entity)
                .is_some_and(|light| light.shadow_layer == SHADOW_LAYER_REQUESTED);

            if casts_shadows {
                let is_point = world
                    .get_component_for_entity::<RenderLight>(context.entity)
                    .is_some_and(|light| light.light_type == LightType::Point.index());

                let shadow_slot = if is_point {
                    world
                        .get_resource_mut::<RenderPointShadowMaps>()
                        .and_then(|shadow_maps| shadow_maps.push_caster(context.entity))
                } else {
                    world
                        .get_resource_mut::<RenderSpotDirectionalShadowMaps>()
                        .and_then(|shadow_maps| shadow_maps.push_caster(context.entity))
                };

                match shadow_slot {
                    Some(shadow_slot) => {
                        world.insert(RenderShadowCasterSlot(shadow_slot), context.entity);
                        if let Some(render_light) =
                            world.get_component_for_entity_mut::<RenderLight>(context.entity)
                        {
                            render_light.shadow_layer = shadow_slot as i32;
                        }

                        if let (Some(device), Some(shadow_pipeline)) = (
                            world.get_resource::<RenderDevice>(),
                            world.get_resource::<ShadowPipeline>(),
                        ) {
                            let view_proj = RenderShadowCasterViewProj::new(
                                device,
                                &shadow_pipeline.bind_group_layout,
                            );
                            world.insert(view_proj, context.entity);
                        }
                    }
                    // Shadow-caster pool exhausted; fall back to unshadowed.
                    None => {
                        if let Some(render_light) =
                            world.get_component_for_entity_mut::<RenderLight>(context.entity)
                        {
                            render_light.shadow_layer = -1;
                        }
                    }
                }
            }

            if let (Some(lights), Some(queue)) = (
                world.get_resource::<RenderLights>(),
                world.get_resource::<RenderQueue>(),
            ) {
                lights.write_count(queue);
            }
        })
    }

    fn on_remove() -> Option<concerto_ecs::component::ComponentLifecycleCallback> {
        Some(|mut world, context| {
            let Some(&slot) = world.get_component_for_entity::<RenderLightSlot>(context.entity)
            else {
                return;
            };

            let moved_entity = if let Some(lights) = world.get_resource_mut::<RenderLights>() {
                lights.swap_remove_light(&slot)
            } else {
                return;
            };

            world.remove_component::<RenderLightSlot>(context.entity);

            if let Some(moved_entity) = moved_entity {
                if let Some(moved_slot) =
                    world.get_component_for_entity_mut::<RenderLightSlot>(moved_entity)
                {
                    moved_slot.update_slot(*slot);
                }

                push_render_light_to_gpu(&world, moved_entity);
            }

            if let (Some(lights), Some(queue)) = (
                world.get_resource::<RenderLights>(),
                world.get_resource::<RenderQueue>(),
            ) {
                lights.write_count(queue);
            }
        })
    }
}

#[derive(Resource)]
pub(crate) struct RenderLights {
    pub(crate) buffer: Buffer,
    pub(crate) slots: Vec<Entity>,
}

impl RenderLights {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let lights = LightsUniform {
            lights: [RenderLight::zeroed(); MAX_LIGHTS],
            light_count: 0,
        };

        let mut buffer = UniformBuffer::new(Vec::new());
        buffer.write(&lights).unwrap();

        let lights_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("lights_buffer"),
            contents: &buffer.into_inner(),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        Self {
            buffer: lights_buffer,
            slots: Vec::new(),
        }
    }

    pub(crate) fn write_buffer(
        &self,
        queue: &wgpu::Queue,
        light: &RenderLight,
        offset: RenderLightSlot,
    ) {
        let slot_offset = light.size().get() * *offset as u64;

        let mut buffer = UniformBuffer::new(Vec::new());
        buffer.write(light).unwrap();
        queue.write_buffer(&self.buffer, slot_offset, &buffer.into_inner());
    }

    pub(crate) fn write_count(&self, queue: &wgpu::Queue) {
        let count_offset = RenderLight::SHADER_SIZE.get() * MAX_LIGHTS as u64;
        let count = self.slots.len() as i32;

        let mut buffer = UniformBuffer::new(Vec::new());
        buffer.write(&count).unwrap();
        queue.write_buffer(&self.buffer, count_offset, &buffer.into_inner());
    }

    pub(crate) fn push_light(&mut self, light: Entity) {
        self.slots.push(light);
    }

    pub(crate) fn swap_remove_light(&mut self, slot: &RenderLightSlot) -> Option<Entity> {
        let index = **slot as usize;
        self.slots.swap_remove(index);
        self.slots.get(index).copied()
    }

    pub(crate) fn len(&self) -> usize {
        self.slots.len()
    }
}

pub(crate) fn update_changed_lights(
    lights: Query<(&RenderLight, &RenderLightSlot), Changed<RenderLight>>,
    lights_buffer: Res<RenderLights>,
    queue: Res<RenderQueue>,
) {
    for (light, slot) in lights.iter() {
        lights_buffer.write_buffer(&queue, light, *slot);
    }
}

pub(crate) fn extract_lights(
    lights: Extracted<Query<(&Light, &GlobalTransform, &RenderEntity)>>,
    render_lights: Query<&mut RenderLight>,
    mut cmd: CommandQueue,
) {
    for (light, transform, render_entity) in lights.iter() {
        let render_entity = **render_entity;
        let local_z = transform.rotation() * Vec3::Z;
        let cos_cone_angle = match &light.light_type {
            LightType::Spot { cone_angle } => f32::cos(*cone_angle),
            _ => 0.0,
        };

        if let Some(mut render_light) = render_lights.get_entity(render_entity) {
            render_light.direction = -local_z;
            render_light.color = light.color.to_linear();
            render_light.translation = transform.translation();
            render_light.intensity = light.intensity;
            render_light.light_type = light.light_type.index();
            render_light.cos_cone_angle = cos_cone_angle;
            continue;
        }

        let render_light = RenderLight {
            translation: transform.translation(),
            color: light.color.to_linear(),
            intensity: light.intensity,
            direction: -local_z,
            light_type: light.light_type.index(),
            cos_cone_angle,
            shadow_layer: if light.shadowmaps_enabled {
                SHADOW_LAYER_REQUESTED
            } else {
                -1
            },
        };

        cmd.insert(render_light, render_entity);
    }
}
