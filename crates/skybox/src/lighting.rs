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
        Lut, distance_to_top, fibonacci_direction, multiple_scattering_lut, observer_radius,
        sky_radiance, transmittance_along, transmittance_lut,
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

pub(crate) type SunQuery<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static Sun,
        &'static Light,
        &'static GlobalTransform,
    ),
>;

pub(crate) type SkyQuery<'w, 's> = Query<'w, 's, (Entity, &'static Sky)>;

fn is_directional(light: &Light) -> bool {
    matches!(light.light_type, LightType::Directional)
}

pub(crate) fn sky_sun(sky: &Sky, suns: &SunQuery) -> Option<Entity> {
    sky.sun.entity().filter(|entity| {
        suns.get_entity(*entity)
            .is_some_and(|(_, _, light, _)| is_directional(light))
    })
}

pub(crate) fn active_sun(skies: &SkyQuery, suns: &SunQuery) -> Option<Entity> {
    skies
        .iter()
        .filter(|(_, sky)| sky_sun(sky, suns).is_some())
        .min_by_key(|(camera, _)| camera.index())
        .and_then(|(_, sky)| sky_sun(sky, suns))
}

pub(crate) fn build_atmosphere_luts(
    skies: SkyQuery,
    suns: SunQuery,
    mut luts: ResMut<AtmosphereLuts>,
) {
    let Some((_, sun, _, _)) = active_sun(&skies, &suns).and_then(|entity| suns.get_entity(entity))
    else {
        return;
    };
    if !luts.is_built_for(&sun.atmosphere) {
        *luts = AtmosphereLuts::build(&sun.atmosphere);
    }
}

pub(crate) struct SunWarnings {
    undirected_suns: EntityHashSet,
    dark_skies: EntityHashSet,
    redirected_skies: EntityHashSet,
}

impl FromWorld for SunWarnings {
    fn from_world(_world: &World) -> Self {
        Self {
            undirected_suns: EntityHashSet::default(),
            dark_skies: EntityHashSet::default(),
            redirected_skies: EntityHashSet::default(),
        }
    }
}

impl SunWarnings {
    fn warn(&mut self, suns: &SunQuery, skies: &SkyQuery, active: Option<Entity>) {
        let undirected: EntityHashSet = suns
            .iter()
            .filter(|(_, _, light, _)| !is_directional(light))
            .map(|(entity, ..)| entity)
            .collect();
        for entity in undirected.difference(&self.undirected_suns) {
            log::warn!("Sun on {entity:?} is ignored: its Light is not directional");
        }
        self.undirected_suns = undirected;

        let mut dark = EntityHashSet::default();
        let mut redirected = EntityHashSet::default();
        for (camera, sky) in skies.iter() {
            match (sky_sun(sky, suns), active) {
                (None, _) => {
                    if !self.dark_skies.contains(&camera) {
                        log::warn!(
                            "Sky on {camera:?} names no directional light with Sun, so it shows night: set its sun"
                        );
                    }
                    dark.insert(camera);
                }
                (Some(sun), Some(active)) if sun != active => {
                    if !self.redirected_skies.contains(&camera) {
                        log::warn!(
                            "Sky on {camera:?} names {sun:?} but shows {active:?}, the sun of the Sky with the lowest entity index: one sun is lit per frame"
                        );
                    }
                    redirected.insert(camera);
                }
                _ => {}
            }
        }
        self.dark_skies = dark;
        self.redirected_skies = redirected;
    }
}

fn smoothstep(low: f32, high: f32, x: f32) -> f32 {
    let t = ((x - low) / (high - low)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn sun_tint(atmosphere: &Atmosphere, disk_radius: f32, toward_sun: Vec3) -> Vec3 {
    let observer = observer_radius(atmosphere);
    let ground = atmosphere.planet_radius / observer;
    let horizon = -(1.0 - ground * ground).max(0.0).sqrt();
    let disk = disk_radius.to_radians().sin().max(1e-4);
    let visible = smoothstep(horizon - disk, horizon + disk, toward_sun.y);
    let mu = toward_sun.y.max(horizon);
    transmittance_along(
        atmosphere,
        observer,
        mu,
        distance_to_top(atmosphere, observer, mu),
    ) * visible
}

fn tint_of(sun: &Sun, transform: &GlobalTransform) -> LinearRgba {
    let tint = sun_tint(&sun.atmosphere, sun.disk_radius, toward_sun(transform));
    LinearRgba::new(tint.x, tint.y, tint.z, 1.0)
}

pub(crate) fn rgb(color: LinearRgba) -> Vec3 {
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

pub(crate) fn toward_sun(transform: &GlobalTransform) -> Vec3 {
    (transform.rotation() * Vec3::Z).normalize_or(Vec3::Y)
}

fn sun_irradiance(
    luts: &AtmosphereLuts,
    sun: &Sun,
    light: &Light,
    transform: &GlobalTransform,
) -> SphericalHarmonics {
    let toward_sun = toward_sun(transform);
    let tint = sun_tint(&luts.atmosphere, sun.disk_radius, toward_sun);
    let illuminance = rgb(light.color.to_linear()) * light.intensity;
    sky_irradiance(luts, toward_sun, illuminance, tint)
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
    skies: SkyQuery,
    lit: Query<(Entity, &Sky, Option<&CameraSkyLight>)>,
    changed_skies: Query<(), Changed<Sky>>,
    mut removed_suns: RemovedComponents<Sun>,
    mut removed_skies: RemovedComponents<Sky>,
    mut warnings: SystemLocal<SunWarnings>,
    mut cmd: CommandQueue,
) {
    let removed = removed_suns.read().count() + removed_skies.read().count() > 0;
    let changed = luts.is_changed()
        || removed
        || changed_suns.iter().next().is_some()
        || changed_skies.iter().next().is_some();
    if !changed {
        return;
    }

    let active = active_sun(&skies, &suns);
    warnings.warn(&suns, &skies, active);

    for (entity, sun, light, transform) in suns.iter() {
        let current = tints.get_entity(entity).map(|(_, tint)| tint.0);
        if !is_directional(light) {
            if current.is_some() {
                cmd.remove::<LightTint>(entity);
            }
            continue;
        }
        let tint = tint_of(sun, transform);
        if current != Some(tint) {
            cmd.insert(LightTint(tint), entity);
        }
    }

    let irradiance = active
        .and_then(|entity| suns.get_entity(entity))
        .map(|(_, sun, light, transform)| sun_irradiance(&luts, sun, light, transform))
        .unwrap_or_default();
    for (camera, sky, current) in lit.iter() {
        let sky_light = CameraSkyLight {
            irradiance: if sky_sun(sky, &suns).is_some() {
                irradiance
            } else {
                SphericalHarmonics::default()
            },
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
