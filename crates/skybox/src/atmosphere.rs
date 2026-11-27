use std::f32::consts::PI;

use glam::{Vec2, Vec3, Vec4};

use crate::sun::Atmosphere;

/// Height of the viewer above the ground, in kilometres.
pub const OBSERVER_HEIGHT: f32 = 0.2;
/// Width of the transmittance table, over the cosine of the zenith angle.
pub const TRANSMITTANCE_LUT_WIDTH: u32 = 256;
/// Height of the transmittance table, over the height in the atmosphere.
pub const TRANSMITTANCE_LUT_HEIGHT: u32 = 64;
/// Width and height of the multiple-scattering table.
pub const MULTIPLE_SCATTERING_LUT_SIZE: u32 = 32;
/// Samples along a ray when integrating optical depth.
pub const TRANSMITTANCE_STEPS: u32 = 40;
/// Directions over the sphere when building the multiple-scattering table.
pub const MULTIPLE_SCATTERING_DIRECTIONS: u32 = 64;
/// Samples along each ray when building the multiple-scattering table.
pub const MULTIPLE_SCATTERING_STEPS: u32 = 20;
/// Samples along a view ray when marching the sky.
pub const SKY_STEPS: u32 = 16;

/// The air's scattering and extinction at one height, per kilometre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Medium {
    pub rayleigh_scattering: Vec3,
    pub mie_scattering: f32,
    pub extinction: Vec3,
}

impl Medium {
    /// Rayleigh and Mie scattering together.
    pub fn scattering(&self) -> Vec3 {
        self.rayleigh_scattering + Vec3::splat(self.mie_scattering)
    }
}

/// A table of linear values for an `Rgba16Float` texture, row by row from `v = 0`.
///
/// Texel `(x, y)` holds the value at `(x / (width - 1), y / (height - 1))`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lut {
    pub width: u32,
    pub height: u32,
    pub texels: Vec<Vec4>,
}

impl Lut {
    /// Fills a `width` × `height` table with `value` at each texel's unit coordinates.
    pub fn build(width: u32, height: u32, value: impl Fn(Vec2) -> Vec3) -> Self {
        let unit = |index: u32, size: u32| index as f32 / (size.max(2) - 1) as f32;
        let texels = (0..height)
            .flat_map(|y| (0..width).map(move |x| Vec2::new(unit(x, width), unit(y, height))))
            .map(|coordinates| value(coordinates).extend(1.0))
            .collect();
        Self {
            width,
            height,
            texels,
        }
    }

    /// Bilinear sample at unit coordinates, clamped to the edges; zero for an empty table.
    ///
    /// # Panics
    ///
    /// Panics when a non-empty table holds fewer than `width * height` texels.
    pub fn sample(&self, unit: Vec2) -> Vec3 {
        if self.texels.is_empty() {
            return Vec3::ZERO;
        }
        let last = Vec2::new(self.width as f32 - 1.0, self.height as f32 - 1.0);
        let position = unit.clamp(Vec2::ZERO, Vec2::ONE) * last;
        let low = position.floor();
        let fraction = position - low;
        let x0 = low.x as u32;
        let y0 = low.y as u32;
        let x1 = (x0 + 1).min(self.width - 1);
        let y1 = (y0 + 1).min(self.height - 1);
        let texel = |x: u32, y: u32| self.texels[(y * self.width + x) as usize].truncate();
        let top = texel(x0, y0).lerp(texel(x1, y0), fraction.x);
        let bottom = texel(x0, y1).lerp(texel(x1, y1), fraction.x);
        top.lerp(bottom, fraction.y)
    }
}

/// Distance from the planet's centre to the top of the atmosphere.
pub fn top_radius(atmosphere: &Atmosphere) -> f32 {
    atmosphere.planet_radius + atmosphere.atmosphere_height
}

/// Distance from the planet's centre to the viewer.
pub fn observer_radius(atmosphere: &Atmosphere) -> f32 {
    atmosphere.planet_radius + OBSERVER_HEIGHT
}

/// The medium at `height` kilometres above the ground.
pub fn medium(atmosphere: &Atmosphere, height: f32) -> Medium {
    let height = height.max(0.0);
    let rayleigh = (-height / atmosphere.rayleigh_scale_height).exp();
    let mie = (-height / atmosphere.mie_scale_height).exp();
    let ozone = (1.0
        - (height - atmosphere.ozone_center_height).abs()
            / (0.5 * atmosphere.ozone_width).max(f32::EPSILON))
    .max(0.0);
    let rayleigh_scattering = atmosphere.rayleigh_scattering * rayleigh;
    let mie_scattering = atmosphere.mie_scattering * mie;
    let mie_extinction = (atmosphere.mie_scattering + atmosphere.mie_absorption) * mie;
    Medium {
        rayleigh_scattering,
        mie_scattering,
        extinction: rayleigh_scattering
            + Vec3::splat(mie_extinction)
            + atmosphere.ozone_absorption * ozone,
    }
}

/// Rayleigh phase function for the cosine of the scattering angle.
pub fn rayleigh_phase(cos_theta: f32) -> f32 {
    3.0 / (16.0 * PI) * (1.0 + cos_theta * cos_theta)
}

/// Cornette-Shanks Mie phase function with asymmetry `g`.
pub fn mie_phase(g: f32, cos_theta: f32) -> f32 {
    let g2 = g * g;
    let k = 3.0 / (8.0 * PI) * (1.0 - g2) / (2.0 + g2);
    let denominator = (1.0 + g2 - 2.0 * g * cos_theta).max(1e-6);
    k * (1.0 + cos_theta * cos_theta) / (denominator * denominator.sqrt())
}

/// Distance along a ray at `radius` with zenith cosine `mu` to the top of the atmosphere.
pub fn distance_to_top(atmosphere: &Atmosphere, radius: f32, mu: f32) -> f32 {
    let top = top_radius(atmosphere);
    let discriminant = radius * radius * (mu * mu - 1.0) + top * top;
    (-radius * mu + discriminant.max(0.0).sqrt()).max(0.0)
}

/// Distance along a ray at `radius` with zenith cosine `mu` to the ground, if it hits it.
pub fn distance_to_ground(atmosphere: &Atmosphere, radius: f32, mu: f32) -> Option<f32> {
    let bottom = atmosphere.planet_radius;
    let discriminant = radius * radius * (mu * mu - 1.0) + bottom * bottom;
    (mu < 0.0 && discriminant >= 0.0).then(|| (-radius * mu - discriminant.sqrt()).max(0.0))
}

/// Distance from the planet's centre after `distance` along a ray at `radius` with zenith cosine `mu`.
pub fn radius_along(radius: f32, mu: f32, distance: f32) -> f32 {
    (distance * distance + 2.0 * radius * mu * distance + radius * radius).sqrt()
}

/// Extinction integrated over `length` kilometres of a ray at `radius` with zenith cosine `mu`.
pub fn optical_depth(atmosphere: &Atmosphere, radius: f32, mu: f32, length: f32) -> Vec3 {
    let step = length / TRANSMITTANCE_STEPS as f32;
    (0..TRANSMITTANCE_STEPS)
        .map(|index| radius_along(radius, mu, (index as f32 + 0.5) * step))
        .map(|sample| medium(atmosphere, sample - atmosphere.planet_radius).extinction * step)
        .sum()
}

/// Light left after `length` kilometres of a ray at `radius` with zenith cosine `mu`, ignoring the ground.
pub fn transmittance_along(atmosphere: &Atmosphere, radius: f32, mu: f32, length: f32) -> Vec3 {
    (-optical_depth(atmosphere, radius, mu, length)).exp()
}

/// Light left on a ray from `radius` with zenith cosine `mu` out of the atmosphere; zero through the planet.
pub fn transmittance_to_space(atmosphere: &Atmosphere, radius: f32, mu: f32) -> Vec3 {
    if distance_to_ground(atmosphere, radius, mu).is_some() {
        return Vec3::ZERO;
    }
    transmittance_along(
        atmosphere,
        radius,
        mu,
        distance_to_top(atmosphere, radius, mu),
    )
}

fn horizon_distance(atmosphere: &Atmosphere) -> f32 {
    let top = top_radius(atmosphere);
    let bottom = atmosphere.planet_radius;
    (top * top - bottom * bottom).max(0.0).sqrt()
}

/// Unit coordinates of `(radius, mu)` in the transmittance table (Bruneton's mapping); rays below the horizon clamp to it.
pub fn transmittance_lut_unit(atmosphere: &Atmosphere, radius: f32, mu: f32) -> Vec2 {
    let horizon = horizon_distance(atmosphere);
    let bottom = atmosphere.planet_radius;
    let rho = (radius * radius - bottom * bottom).max(0.0).sqrt();
    let distance = distance_to_top(atmosphere, radius, mu);
    let shortest = top_radius(atmosphere) - radius;
    let longest = rho + horizon;
    let u = (distance - shortest) / (longest - shortest).max(f32::EPSILON);
    let v = rho / horizon.max(f32::EPSILON);
    Vec2::new(u, v).clamp(Vec2::ZERO, Vec2::ONE)
}

struct ZenithRay {
    radius: f32,
    mu: f32,
}

fn transmittance_lut_ray(atmosphere: &Atmosphere, unit: Vec2) -> ZenithRay {
    let horizon = horizon_distance(atmosphere);
    let bottom = atmosphere.planet_radius;
    let rho = horizon * unit.y;
    let radius = (rho * rho + bottom * bottom).sqrt();
    let shortest = top_radius(atmosphere) - radius;
    let longest = rho + horizon;
    let distance = shortest + unit.x * (longest - shortest);
    let mu = if distance == 0.0 {
        1.0
    } else {
        (horizon * horizon - rho * rho - distance * distance) / (2.0 * radius * distance)
    };
    ZenithRay {
        radius,
        mu: mu.clamp(-1.0, 1.0),
    }
}

/// Transmittance to space over every height and zenith angle, mapped by [`transmittance_lut_unit`].
pub fn transmittance_lut(atmosphere: &Atmosphere) -> Lut {
    Lut::build(TRANSMITTANCE_LUT_WIDTH, TRANSMITTANCE_LUT_HEIGHT, |unit| {
        let ray = transmittance_lut_ray(atmosphere, unit);
        transmittance_along(
            atmosphere,
            ray.radius,
            ray.mu,
            distance_to_top(atmosphere, ray.radius, ray.mu),
        )
    })
}

/// The transmittance table's light toward a sun at zenith cosine `mu` from `radius`; zero when the planet hides it.
pub fn sun_transmittance(
    atmosphere: &Atmosphere,
    transmittance: &Lut,
    radius: f32,
    mu: f32,
) -> Vec3 {
    if distance_to_ground(atmosphere, radius, mu).is_some() {
        return Vec3::ZERO;
    }
    transmittance.sample(transmittance_lut_unit(atmosphere, radius, mu))
}

/// Unit coordinates of `(radius, sun_mu)` in the multiple-scattering table.
pub fn multiple_scattering_lut_unit(atmosphere: &Atmosphere, radius: f32, sun_mu: f32) -> Vec2 {
    Vec2::new(
        sun_mu * 0.5 + 0.5,
        (radius - atmosphere.planet_radius) / atmosphere.atmosphere_height.max(f32::EPSILON),
    )
    .clamp(Vec2::ZERO, Vec2::ONE)
}

/// The `index`th of `count` directions spread evenly over the sphere.
pub fn fibonacci_direction(index: u32, count: u32) -> Vec3 {
    let golden_angle = PI * (3.0 - 5.0_f32.sqrt());
    let y = 1.0 - 2.0 * (index as f32 + 0.5) / count as f32;
    let ring = (1.0 - y * y).max(0.0).sqrt();
    let phi = index as f32 * golden_angle;
    Vec3::new(phi.cos() * ring, y, phi.sin() * ring)
}

fn integrate_step(source: Vec3, extinction: Vec3, step_transmittance: Vec3) -> Vec3 {
    (source - source * step_transmittance) / extinction.max(Vec3::splat(1e-9))
}

struct IsotropicScattering {
    luminance: Vec3,
    transfer: Vec3,
}

fn isotropic_scattering(
    atmosphere: &Atmosphere,
    transmittance: &Lut,
    radius: f32,
    direction: Vec3,
    sun: Vec3,
) -> IsotropicScattering {
    let isotropic_phase = 1.0 / (4.0 * PI);
    let origin = Vec3::new(0.0, radius, 0.0);
    let ground = distance_to_ground(atmosphere, radius, direction.y);
    let length = ground.unwrap_or_else(|| distance_to_top(atmosphere, radius, direction.y));
    let step = length / MULTIPLE_SCATTERING_STEPS as f32;
    let mut throughput = Vec3::ONE;
    let mut luminance = Vec3::ZERO;
    let mut transfer = Vec3::ZERO;
    for index in 0..MULTIPLE_SCATTERING_STEPS {
        let position = origin + direction * (index as f32 + 0.5) * step;
        let sample_radius = position.length();
        let sun_mu = position.dot(sun) / sample_radius;
        let air = medium(atmosphere, sample_radius - atmosphere.planet_radius);
        let step_transmittance = (-air.extinction * step).exp();
        let lit = sun_transmittance(atmosphere, transmittance, sample_radius, sun_mu);
        let scattering = air.scattering();
        luminance += throughput
            * integrate_step(
                scattering * lit * isotropic_phase,
                air.extinction,
                step_transmittance,
            );
        transfer += throughput * integrate_step(scattering, air.extinction, step_transmittance);
        throughput *= step_transmittance;
    }
    if let Some(distance) = ground {
        let position = origin + direction * distance;
        let ground_mu = (position.dot(sun) / position.length()).max(0.0);
        let albedo = atmosphere.ground_albedo.to_linear();
        let albedo = Vec3::new(albedo.r, albedo.g, albedo.b);
        luminance += throughput
            * sun_transmittance(
                atmosphere,
                transmittance,
                atmosphere.planet_radius,
                ground_mu,
            )
            * ground_mu
            * albedo
            / PI;
    }
    IsotropicScattering {
        luminance,
        transfer,
    }
}

/// Hillaire's multiple-scattering term over every height and sun angle, mapped by [`multiple_scattering_lut_unit`].
pub fn multiple_scattering_lut(atmosphere: &Atmosphere, transmittance: &Lut) -> Lut {
    Lut::build(
        MULTIPLE_SCATTERING_LUT_SIZE,
        MULTIPLE_SCATTERING_LUT_SIZE,
        |unit| {
            let sun_mu = unit.x * 2.0 - 1.0;
            let sun = Vec3::new((1.0 - sun_mu * sun_mu).max(0.0).sqrt(), sun_mu, 0.0);
            let radius = atmosphere.planet_radius
                + (unit.y * atmosphere.atmosphere_height).max(OBSERVER_HEIGHT * 0.5);
            let mut second_order = Vec3::ZERO;
            let mut transfer = Vec3::ZERO;
            for index in 0..MULTIPLE_SCATTERING_DIRECTIONS {
                let direction = fibonacci_direction(index, MULTIPLE_SCATTERING_DIRECTIONS);
                let scattered =
                    isotropic_scattering(atmosphere, transmittance, radius, direction, sun);
                second_order += scattered.luminance;
                transfer += scattered.transfer;
            }
            let count = MULTIPLE_SCATTERING_DIRECTIONS as f32;
            let transfer = (transfer / count).min(Vec3::splat(0.999));
            second_order / count / (Vec3::ONE - transfer)
        },
    )
}

/// Radiance reaching `radius` along unit `view` from the sky, per unit of sun illuminance from unit `sun`.
pub fn sky_radiance(
    atmosphere: &Atmosphere,
    transmittance: &Lut,
    multiple_scattering: &Lut,
    radius: f32,
    view: Vec3,
    sun: Vec3,
) -> Vec3 {
    let origin = Vec3::new(0.0, radius, 0.0);
    let length = distance_to_ground(atmosphere, radius, view.y)
        .unwrap_or_else(|| distance_to_top(atmosphere, radius, view.y));
    let cos_theta = view.dot(sun);
    let rayleigh = rayleigh_phase(cos_theta);
    let mie = mie_phase(atmosphere.mie_asymmetry, cos_theta);
    let mut throughput = Vec3::ONE;
    let mut radiance = Vec3::ZERO;
    for index in 0..SKY_STEPS {
        let start = (index as f32 / SKY_STEPS as f32).powi(2) * length;
        let end = ((index + 1) as f32 / SKY_STEPS as f32).powi(2) * length;
        let step = end - start;
        let position = origin + view * (start + 0.3 * step);
        let sample_radius = position.length();
        let sun_mu = position.dot(sun) / sample_radius;
        let air = medium(atmosphere, sample_radius - atmosphere.planet_radius);
        let lit = sun_transmittance(atmosphere, transmittance, sample_radius, sun_mu);
        let multiple = multiple_scattering.sample(multiple_scattering_lut_unit(
            atmosphere,
            sample_radius,
            sun_mu,
        ));
        let source = air.rayleigh_scattering * (rayleigh * lit + multiple)
            + air.mie_scattering * (mie * lit + multiple);
        let step_transmittance = (-air.extinction * step).exp();
        radiance += throughput * integrate_step(source, air.extinction, step_transmittance);
        throughput *= step_transmittance;
    }
    radiance
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::*;
    use crate::sun::Sun;

    struct Case {
        name: &'static str,
        holds: bool,
    }

    fn elevated(degrees: f32) -> Vec3 {
        let radians = degrees.to_radians();
        Vec3::new(radians.cos(), radians.sin(), 0.0)
    }

    fn is_decreasing(values: &[Vec3]) -> bool {
        values.windows(2).all(|pair| pair[1].cmplt(pair[0]).all())
    }

    #[test]
    fn transmittance_and_scattering_behave_physically() {
        let atmosphere = Sun::default().atmosphere;
        let observer = observer_radius(&atmosphere);
        let toward_horizon: Vec<Vec3> = (0..=20)
            .map(|step| 1.0 - step as f32 / 20.0)
            .map(|mu| transmittance_to_space(&atmosphere, observer, mu))
            .collect();
        let horizon = transmittance_to_space(&atmosphere, observer, 0.0);
        let through_planet = transmittance_to_space(&atmosphere, observer, -0.5);

        let transmittance = transmittance_lut(&atmosphere);
        let no_multiple_scattering = Lut::default();
        let sun = elevated(60.0);
        let zenith = sky_radiance(
            &atmosphere,
            &transmittance,
            &no_multiple_scattering,
            observer,
            Vec3::Y,
            sun,
        );
        let near_sun = sky_radiance(
            &atmosphere,
            &transmittance,
            &no_multiple_scattering,
            observer,
            elevated(57.0),
            sun,
        );
        let multiple_scattering = multiple_scattering_lut(&atmosphere, &transmittance);
        let values: Vec<Vec3> = toward_horizon
            .iter()
            .copied()
            .chain([horizon, through_planet, zenith, near_sun])
            .chain(transmittance.texels.iter().map(|texel| texel.truncate()))
            .chain(
                multiple_scattering
                    .texels
                    .iter()
                    .map(|texel| texel.truncate()),
            )
            .collect();

        let cases = [
            Case {
                name: "transmittance is 1 at zero path length",
                holds: transmittance_along(&atmosphere, observer, 0.3, 0.0) == Vec3::ONE,
            },
            Case {
                name: "transmittance falls monotonically toward the horizon",
                holds: is_decreasing(&toward_horizon),
            },
            Case {
                name: "at the horizon transmittance is lower for blue than for red",
                holds: horizon.z < horizon.x,
            },
            Case {
                name: "transmittance is 0 through the planet",
                holds: through_planet == Vec3::ZERO,
            },
            Case {
                name: "single scattering is bluer at the zenith than near the sun",
                holds: zenith.z / zenith.x > near_sun.z / near_sun.x,
            },
            Case {
                name: "every value is non-negative",
                holds: values.iter().all(|value| value.cmpge(Vec3::ZERO).all()),
            },
        ];
        for case in cases {
            assert!(case.holds, "{}", case.name);
        }
    }
}
