use std::f32::consts::PI;

use concerto_color::LinearRgba;
use concerto_ecs::{
    command::CommandQueue,
    component::removed::RemovedComponents,
    entity::{Entity, EntityHashSet},
    query::{
        Query,
        change_detection::DetectChanges,
        filter::{Changed, Or, With},
    },
    resource::{Res, ResMut, Resource},
    signal::{On, lifecycle::Remove},
    system::input::SystemLocal,
    world::{FromWorld, World},
};
use concerto_foundation::transform::GlobalTransform;
use concerto_render::{
    CameraSkyLight, LightTint, RadianceSample, SphericalHarmonics,
    components::light::{Light, LightType},
};
use glam::Vec3;

use crate::{
    atmosphere::{
        Lut, fibonacci_direction, multiple_scattering_lut, observer_radius, sky_radiance,
        transmittance_lut, transmittance_lut_unit,
    },
    sun::{Atmosphere, Sky, Sun},
};

/// Directions over the sphere integrated into a sky's spherical harmonics.
pub const IRRADIANCE_DIRECTIONS: u32 = 256;

/// The sun's atmosphere as lookup tables, rebuilt on the CPU when it changes; empty until a [`Sun`] exists.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct AtmosphereLuts {
    /// The atmosphere the tables were built from.
    pub atmosphere: Atmosphere,
    /// Transmittance to space, at [`transmittance_lut_unit`](crate::atmosphere::transmittance_lut_unit).
    pub transmittance: Lut,
    /// Hillaire's multiple scattering, at [`multiple_scattering_lut_unit`](crate::atmosphere::multiple_scattering_lut_unit).
    pub multiple_scattering: Lut,
}

impl AtmosphereLuts {
    /// Builds both tables for `atmosphere`.
    pub fn build(atmosphere: &Atmosphere) -> Self {
        let transmittance = transmittance_lut(atmosphere);
        let multiple_scattering = multiple_scattering_lut(atmosphere, &transmittance);
        Self {
            atmosphere: atmosphere.clone(),
            transmittance,
            multiple_scattering,
        }
    }

    fn is_built_for(&self, atmosphere: &Atmosphere) -> bool {
        !self.transmittance.texels.is_empty() && self.atmosphere == *atmosphere
    }
}

type SunQuery<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static Sun,
        &'static Light,
        &'static GlobalTransform,
    ),
>;

fn pick_sun(suns: &SunQuery) -> Option<Entity> {
    suns.iter()
        .filter(|(_, _, light, _)| matches!(light.light_type, LightType::Directional))
        .map(|(entity, ..)| entity)
        .min_by_key(|entity| entity.index())
}

pub(crate) fn build_atmosphere_luts(suns: SunQuery, mut luts: ResMut<AtmosphereLuts>) {
    let Some((_, sun, _, _)) = pick_sun(&suns).and_then(|entity| suns.get_entity(entity)) else {
        return;
    };
    if !luts.is_built_for(&sun.atmosphere) {
        *luts = AtmosphereLuts::build(&sun.atmosphere);
    }
}

pub(crate) struct SunWarnings {
    ignored: EntityHashSet,
    without_sun: bool,
}

impl FromWorld for SunWarnings {
    fn from_world(_world: &World) -> Self {
        Self {
            ignored: EntityHashSet::default(),
            without_sun: false,
        }
    }
}

impl SunWarnings {
    fn warn(&mut self, suns: &SunQuery, picked: Option<Entity>, has_skies: bool) {
        let mut ignored = EntityHashSet::default();
        for (entity, _, light, _) in suns.iter().filter(|(entity, ..)| Some(*entity) != picked) {
            if !self.ignored.contains(&entity) {
                match (picked, &light.light_type) {
                    (Some(sun), LightType::Directional) => log::warn!(
                        "Sun on {entity:?} is ignored: {sun:?} is the sun, the first directional Sun"
                    ),
                    _ => log::warn!("Sun on {entity:?} is ignored: its Light is not directional"),
                }
            }
            ignored.insert(entity);
        }
        self.ignored = ignored;
        let without_sun = has_skies && picked.is_none();
        if without_sun && !self.without_sun {
            log::warn!(
                "A Sky has no Sun, so it shows night: add Sun to a directional light to light it"
            );
        }
        self.without_sun = without_sun;
    }
}

struct SunLighting {
    entity: Entity,
    tint: LinearRgba,
    irradiance: SphericalHarmonics,
}

fn smoothstep(low: f32, high: f32, x: f32) -> f32 {
    let t = ((x - low) / (high - low)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn sun_tint(luts: &AtmosphereLuts, disk_radius: f32, toward_sun: Vec3) -> Vec3 {
    let atmosphere = &luts.atmosphere;
    let observer = observer_radius(atmosphere);
    let ground = atmosphere.planet_radius / observer;
    let horizon = -(1.0 - ground * ground).max(0.0).sqrt();
    let disk = disk_radius.to_radians().sin().max(1e-4);
    let visible = smoothstep(horizon - disk, horizon + disk, toward_sun.y);
    luts.transmittance
        .sample(transmittance_lut_unit(atmosphere, observer, toward_sun.y))
        * visible
}

fn rgb(color: LinearRgba) -> Vec3 {
    Vec3::new(color.r, color.g, color.b)
}

fn sky_irradiance(
    luts: &AtmosphereLuts,
    toward_sun: Vec3,
    illuminance: Vec3,
    tint: Vec3,
) -> SphericalHarmonics {
    let atmosphere = &luts.atmosphere;
    let observer = observer_radius(atmosphere);
    let solid_angle = 4.0 * PI / IRRADIANCE_DIRECTIONS as f32;
    let directions: Vec<Vec3> = (0..IRRADIANCE_DIRECTIONS)
        .map(|index| fibonacci_direction(index, IRRADIANCE_DIRECTIONS))
        .collect();
    let mut samples: Vec<RadianceSample> = directions
        .iter()
        .filter(|direction| direction.y > 0.0)
        .map(|&direction| RadianceSample {
            direction,
            radiance: sky_radiance(
                atmosphere,
                &luts.transmittance,
                &luts.multiple_scattering,
                observer,
                direction,
                toward_sun,
            ) * illuminance,
            solid_angle,
        })
        .collect();
    let sky_on_ground: Vec3 = samples
        .iter()
        .map(|sample| sample.radiance * sample.direction.y * sample.solid_angle)
        .sum();
    let sun_on_ground = tint * toward_sun.y.max(0.0) * illuminance;
    let ground = rgb(atmosphere.ground_albedo.to_linear()) * (sun_on_ground + sky_on_ground) / PI;
    samples.extend(
        directions
            .iter()
            .filter(|direction| direction.y <= 0.0)
            .map(|&direction| RadianceSample {
                direction,
                radiance: ground,
                solid_angle,
            }),
    );
    SphericalHarmonics::from_radiance(&samples)
}

fn light_sun(
    entity: Entity,
    luts: &AtmosphereLuts,
    sun: &Sun,
    light: &Light,
    transform: &GlobalTransform,
) -> SunLighting {
    let toward_sun = (transform.rotation() * Vec3::Z).normalize_or(Vec3::Y);
    let tint = sun_tint(luts, sun.disk_radius, toward_sun);
    let illuminance = rgb(light.color.to_linear()) * light.intensity;
    SunLighting {
        entity,
        tint: LinearRgba::new(tint.x, tint.y, tint.z, 1.0),
        irradiance: sky_irradiance(luts, toward_sun, illuminance, tint),
    }
}

pub(crate) fn update_sun_lighting(
    luts: Res<AtmosphereLuts>,
    suns: SunQuery,
    changed_suns: Query<
        (),
        (
            With<Sun>,
            Or<(Changed<Sun>, Changed<Light>, Changed<GlobalTransform>)>,
        ),
    >,
    tints: Query<(Entity, &LightTint), With<Sun>>,
    skies: Query<(Entity, &Sky, Option<&CameraSkyLight>)>,
    changed_skies: Query<(), Changed<Sky>>,
    mut removed_suns: RemovedComponents<Sun>,
    mut warnings: SystemLocal<SunWarnings>,
    mut cmd: CommandQueue,
) {
    let removed = removed_suns.read().count() > 0;
    let changed = luts.is_changed()
        || removed
        || changed_suns.iter().next().is_some()
        || changed_skies.iter().next().is_some();
    if !changed {
        return;
    }

    let picked = pick_sun(&suns);
    warnings.warn(&suns, picked, skies.iter().next().is_some());
    let lighting = picked
        .and_then(|entity| suns.get_entity(entity))
        .map(|(entity, sun, light, transform)| light_sun(entity, &luts, sun, light, transform));

    for (entity, _) in tints.iter() {
        if Some(entity) != picked {
            cmd.remove::<LightTint>(entity);
        }
    }
    if let Some(lighting) = &lighting
        && tints.get_entity(lighting.entity).map(|(_, tint)| tint.0) != Some(lighting.tint)
    {
        cmd.insert(LightTint(lighting.tint), lighting.entity);
    }

    let irradiance = lighting
        .map(|lighting| lighting.irradiance)
        .unwrap_or_default();
    for (camera, sky, current) in skies.iter() {
        let sky_light = CameraSkyLight {
            irradiance,
            night_color: sky.night_color.to_linear(),
            night_ambient: sky.night_ambient.to_linear(),
        };
        if current != Some(&sky_light) {
            cmd.insert(sky_light, camera);
        }
    }
}

pub(crate) fn forget_sky_light(
    on: On<Remove<Sky>>,
    lit: Query<(), With<CameraSkyLight>>,
    mut cmd: CommandQueue,
) {
    let camera = on.signal().entity;
    if lit.contains_entity(camera) {
        cmd.remove::<CameraSkyLight>(camera);
    }
}

pub(crate) fn forget_sun_tint(
    on: On<Remove<Sun>>,
    tinted: Query<(), With<LightTint>>,
    mut cmd: CommandQueue,
) {
    let light = on.signal().entity;
    if tinted.contains_entity(light) {
        cmd.remove::<LightTint>(light);
    }
}
