use concerto_app::extractor::Extracted;
use concerto_color::{Color, LinearRgba};
use concerto_ecs::{
    command::CommandQueue,
    component::Component,
    entity::Entity,
    query::Query,
    reflect::{ReflectComponent, ReflectSceneComponent},
    resource::{Res, ResMut, Resource},
    system::input::SystemLocal,
    world::{FromWorld, World},
};
use concerto_reflect::{Reflect, ReflectDefault};

use concerto_foundation::transform::GlobalTransform;
use encase::{ShaderType, UniformBuffer};
use glam::Vec3;
use wgpu::{util::DeviceExt, Buffer};

use crate::{
    components::{camera::RenderCamera, render_entity::RenderEntity},
    frustum::Frustum,
    queue::RenderQueue,
};

const MAX_LIGHTS: usize = 128;

#[derive(Component, Reflect)]
#[reflect(Component, Default, SceneComponent)]
#[require(crate::components::render_entity::SyncWithRenderWorld)]
pub struct Light {
    pub color: Color,
    pub intensity: f32,
    pub shadowmaps_enabled: bool,
    pub light_type: LightType,
    pub range: f32,
}

impl Default for Light {
    fn default() -> Self {
        Self::spot_light(45.0)
    }
}

impl Light {
    pub fn point_light() -> Self {
        Self {
            color: Color::WHITE,
            intensity: 1.0,
            shadowmaps_enabled: false,
            light_type: LightType::Point,
            range: 0.0,
        }
    }

    pub fn spot_light(cone_angle: f32) -> Self {
        Self {
            color: Color::WHITE,
            intensity: 1.0,
            shadowmaps_enabled: false,
            light_type: LightType::Spot { cone_angle },
            range: 0.0,
        }
    }

    pub fn directional_light() -> Self {
        Self {
            color: Color::WHITE,
            intensity: 1.0,
            shadowmaps_enabled: false,
            light_type: LightType::Directional,
            range: 0.0,
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

    /// Limits the light's reach; see [`Light::range`].
    pub fn with_range(mut self, range: f32) -> Self {
        self.range = range.max(0.0);
        self
    }

    pub fn with_shadows(mut self) -> Self {
        self.shadowmaps_enabled = true;
        self
    }

    /// How much of this light reaches `point`, as the standard material
    /// computes it: inverse-square falloff windowed by [`Light::range`], times
    /// the soft cone edge for spot lights. Directional lights return `1.0`.
    /// Shadows and surface orientation are not taken into account.
    pub fn attenuation_at(&self, transform: &GlobalTransform, point: Vec3) -> f32 {
        let cone_angle = match self.light_type {
            LightType::Directional => return 1.0,
            LightType::Point => None,
            LightType::Spot { cone_angle } => Some(cone_angle),
        };

        let to_point = point - transform.translation();
        let distance_sq = to_point.length_squared();
        let mut attenuation = 1.0 / distance_sq.max(1e-4);
        if self.range > 0.0 {
            let ratio = distance_sq / (self.range * self.range);
            let window = (1.0 - ratio * ratio).clamp(0.0, 1.0);
            attenuation *= window * window;
        }
        if let Some(cone_angle) = cone_angle {
            let forward = -(transform.rotation() * Vec3::Z);
            let cos_cone = cone_angle.cos();
            let soft_edge = cos_cone + (1.0 - cos_cone) * 0.2;
            let angle_cos = to_point.normalize_or_zero().dot(forward);
            attenuation *= smoothstep(cos_cone, soft_edge, angle_cos);
        }
        attenuation
    }
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub(crate) const POINT_SHADOW_NEAR: f32 = 0.1;
const POINT_SHADOW_DEFAULT_FAR: f32 = 20.0;

fn point_shadow_far(range: f32) -> f32 {
    if range > POINT_SHADOW_NEAR {
        range
    } else {
        POINT_SHADOW_DEFAULT_FAR
    }
}

#[derive(Reflect, Default)]
pub enum LightType {
    #[default]
    Point,
    Spot {
        cone_angle: f32,
    },
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

#[derive(Component)]
pub(crate) struct RenderShadowsEnabled(pub(crate) bool);

#[derive(Component, ShaderType, Clone, Copy, PartialEq)]
pub struct RenderLight {
    pub(crate) translation: Vec3,
    pub(crate) intensity: f32,
    pub(crate) color: LinearRgba,
    pub(crate) direction: Vec3,
    pub(crate) light_type: u32,

    // Spotlight
    pub(crate) cos_cone_angle: f32,
    pub(crate) shadow_layer: i32,
    pub(crate) range: f32,
    pub(crate) shadow_far: f32,
}

impl RenderLight {
    fn synced_with(mut self, light: &Light, transform: &GlobalTransform) -> Self {
        self.direction = -(transform.rotation() * Vec3::Z);
        self.color = light.color.to_linear();
        self.translation = transform.translation();
        self.intensity = light.intensity;
        self.light_type = light.light_type.index();
        self.cos_cone_angle = match &light.light_type {
            LightType::Spot { cone_angle } => f32::cos(*cone_angle),
            _ => 0.0,
        };
        self.range = light.range;
        self.shadow_far = match light.light_type {
            LightType::Point => point_shadow_far(light.range),
            _ => 0.0,
        };
        self
    }

    pub(crate) fn zeroed() -> Self {
        Self {
            translation: Vec3::ZERO,
            intensity: 0.0,
            color: LinearRgba::TRANSPARENT,
            direction: Vec3::ZERO,
            light_type: 0,
            cos_cone_angle: 0.0,
            shadow_layer: -1,
            range: 0.0,
            shadow_far: 0.0,
        }
    }
}

#[derive(Resource)]
pub(crate) struct RenderLights {
    pub(crate) buffer: Buffer,
    pub(crate) written: Vec<RenderLight>,
}

impl RenderLights {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let lights_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("lights_buffer"),
            contents: &lights_uniform_bytes(&[]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        Self {
            buffer: lights_buffer,
            written: Vec::new(),
        }
    }

    fn write(&mut self, queue: &wgpu::Queue, lights: Vec<RenderLight>) {
        queue.write_buffer(&self.buffer, 0, &lights_uniform_bytes(&lights));
        self.written = lights;
    }
}

fn lights_uniform_bytes(listed: &[RenderLight]) -> Vec<u8> {
    let mut lights = [RenderLight::zeroed(); MAX_LIGHTS];
    lights[..listed.len()].copy_from_slice(listed);
    let uniform = LightsUniform {
        lights,
        light_count: listed.len() as i32,
    };

    let mut buffer = UniformBuffer::new(Vec::new());
    buffer.write(&uniform).unwrap();
    buffer.into_inner()
}

#[derive(Resource, Default)]
pub(crate) struct VisibleLights {
    pub(crate) entities: Vec<Entity>,
}

impl VisibleLights {
    pub(crate) fn contains(&self, entity: Entity) -> bool {
        self.entities
            .binary_search_by_key(&entity_order(&entity), entity_order)
            .is_ok()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct EntityOrder {
    index: u32,
    generation: u32,
}

pub(crate) fn entity_order(entity: &Entity) -> EntityOrder {
    EntityOrder {
        index: entity.index(),
        generation: entity.generation().get(),
    }
}

fn sees(frustum: &Frustum, light: &RenderLight) -> bool {
    light.light_type == LightType::Directional.index()
        || light.range <= 0.0
        || frustum.intersects_sphere(light.translation, light.range)
}

pub(crate) fn cull_lights(
    lights: Query<(Entity, &RenderLight)>,
    cameras: Query<&RenderCamera>,
    mut visible: ResMut<VisibleLights>,
) {
    let views: Vec<Frustum> = cameras
        .iter()
        .map(|camera| Frustum::from_clip_from_world(camera.camera_uniform.view_proj()))
        .collect();

    let mut entities: Vec<Entity> = lights
        .iter()
        .filter(|(_, light)| views.iter().any(|view| sees(view, light)))
        .map(|(entity, _)| entity)
        .collect();
    entities.sort_by_key(entity_order);
    if visible.entities != entities {
        visible.entities = entities;
    }
}

pub(crate) struct LightOverflow {
    warned: bool,
}

impl FromWorld for LightOverflow {
    fn from_world(_: &World) -> Self {
        Self { warned: false }
    }
}

pub(crate) fn write_visible_lights(
    lights: Query<&RenderLight>,
    visible: Res<VisibleLights>,
    mut render_lights: ResMut<RenderLights>,
    queue: Res<RenderQueue>,
    mut overflow: SystemLocal<LightOverflow>,
) {
    let mut listed: Vec<RenderLight> = visible
        .entities
        .iter()
        .filter_map(|&entity| lights.get_entity(entity).copied())
        .collect();

    let overflowing = listed.len() > MAX_LIGHTS;
    if overflowing && !overflow.warned {
        log::warn!(
            "{} lights are visible and the limit is {MAX_LIGHTS}; the later ones are not rendered",
            listed.len()
        );
    }
    overflow.warned = overflowing;
    listed.truncate(MAX_LIGHTS);

    if render_lights.written != listed {
        render_lights.write(&queue, listed);
    }
}

pub(crate) fn extract_lights(
    lights: Extracted<Query<(&Light, &GlobalTransform, &RenderEntity)>>,
    render_lights: Query<(&mut RenderLight, &mut RenderShadowsEnabled)>,
    mut cmd: CommandQueue,
) {
    for (light, transform, render_entity) in lights.iter() {
        let render_entity = **render_entity;

        if let Some((mut render_light, mut shadows_enabled)) =
            render_lights.get_entity(render_entity)
        {
            let synced = render_light.synced_with(light, transform);
            render_light.set_if_neq(synced);
            if shadows_enabled.0 != light.shadowmaps_enabled {
                shadows_enabled.0 = light.shadowmaps_enabled;
            }
            continue;
        }

        let render_light = RenderLight::zeroed().synced_with(light, transform);

        cmd.insert(
            (render_light, RenderShadowsEnabled(light.shadowmaps_enabled)),
            render_entity,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout_test::{float_words, uniform_words, words};
    use encase::ShaderSize;
    use glam::{Mat4, Quat};

    fn at(translation: Vec3, rotation: Quat) -> GlobalTransform {
        GlobalTransform::new(Mat4::from_rotation_translation(rotation, translation))
    }

    #[test]
    fn render_light_keeps_its_wgsl_layout() {
        let light = RenderLight {
            translation: Vec3::new(1.0, 2.0, 3.0),
            intensity: 4.0,
            color: LinearRgba::new(5.0, 6.0, 7.0, 8.0),
            direction: Vec3::new(9.0, 10.0, 11.0),
            light_type: 12,
            cos_cone_angle: 13.0,
            shadow_layer: -2,
            range: 14.0,
            shadow_far: 15.0,
        };

        let mut expected = float_words(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0]);
        expected.push(12);
        expected.push(13.0f32.to_bits());
        expected.push((-2i32) as u32);
        expected.push(14.0f32.to_bits());
        expected.push(15.0f32.to_bits());

        assert_eq!(uniform_words(&light)[..16], expected);
        assert_eq!(RenderLight::SHADER_SIZE.get(), 64);
        assert_eq!(light.size().get(), 64);
    }

    #[test]
    fn lights_uniform_puts_the_count_after_every_light() {
        let listed = [RenderLight::zeroed(); 3];

        let words = words(&lights_uniform_bytes(&listed));

        assert_eq!(words.len(), 2052);
        assert_eq!(words[2048], 3);
        assert_eq!(words[13], (-1i32) as u32);
        assert_eq!(words[16 * 127 + 13], (-1i32) as u32);
    }

    struct Reach {
        case: &'static str,
        light: fn() -> Light,
        position: Vec3,
        rotation: Quat,
        point: Vec3,
        between: [f32; 2],
    }

    fn reach(case: &'static str, light: fn() -> Light, point: Vec3, between: [f32; 2]) -> Reach {
        Reach {
            case,
            light,
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            point,
            between,
        }
    }

    #[test]
    fn attenuation_falls_off_with_distance_to_the_range_and_stays_inside_a_spot_cone() {
        let ranged = || Light::point_light().with_range(7.0);
        let spot = || Light::spot_light(0.4);
        let cases = [
            reach(
                "a point light at 1",
                Light::point_light,
                Vec3::X,
                [1.0, 1.0],
            ),
            reach(
                "a point light at 2",
                Light::point_light,
                Vec3::new(0.0, 2.0, 0.0),
                [0.25, 0.25],
            ),
            reach("at the range", ranged, Vec3::X * 7.0, [0.0, 0.0]),
            reach("just inside the range", ranged, Vec3::X * 6.9, [1e-6, 1.0]),
            reach("well inside the range", ranged, Vec3::X, [0.99, 1.0]),
            reach("down a spot's -Z", spot, Vec3::NEG_Z, [1.0, 1.0]),
            reach("behind a spot", spot, Vec3::Z, [0.0, 0.0]),
            reach("beside a spot", spot, Vec3::X, [0.0, 0.0]),
            Reach {
                rotation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
                ..reach("down a turned spot", spot, Vec3::NEG_X, [1.0, 1.0])
            },
            Reach {
                position: Vec3::new(3.0, 50.0, -2.0),
                ..reach(
                    "a directional light anywhere",
                    Light::directional_light,
                    Vec3::ZERO,
                    [1.0, 1.0],
                )
            },
        ];
        for case in &cases {
            let transform = at(case.position, case.rotation);
            let attenuation = (case.light)().attenuation_at(&transform, case.point);
            assert!(
                attenuation >= case.between[0] - 1e-6 && attenuation <= case.between[1] + 1e-6,
                "{}: {attenuation} outside {:?}",
                case.case,
                case.between
            );
        }
    }
}
