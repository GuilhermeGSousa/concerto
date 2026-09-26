//! The renderer's light falloff, reproduced on the CPU so the observation
//! rule can ask "is this point lit well enough to see?" and get the same
//! answer the screen gives.
use glam::Vec3;

/// Irradiance below this reads as black on screen: a mannequin lit more
/// faintly than this can move without the player seeing it.
pub const VISIBLE_IRRADIANCE: f32 = 0.035;

/// The store's ambient light (shader units: a factor on albedo).
pub const AMBIENT_LIGHT: f32 = 0.004;
/// The ambient term expressed as irradiance, for comparison with lights (the
/// shader divides a light's diffuse contribution by pi; ambient it does not).
pub const AMBIENT_IRRADIANCE: f32 = AMBIENT_LIGHT * std::f32::consts::PI;

/// Inverse-square falloff with the shader's smooth range window,
/// `(1 - (d/r)^4)^2`. A `range` of 0 is unbounded.
pub fn falloff(distance_sq: f32, range: f32) -> f32 {
    let mut attenuation = 1.0 / distance_sq.max(1e-4);
    if range > 0.0 {
        let ratio = distance_sq / (range * range);
        let window = (1.0 - ratio * ratio).clamp(0.0, 1.0);
        attenuation *= window * window;
    }
    attenuation
}

/// A spot light's cone factor, matching the shader's soft edge.
pub fn cone(cos_angle_to_point: f32, cone_angle: f32) -> f32 {
    let cos_cone = cone_angle.cos();
    let soft = cos_cone + (1.0 - cos_cone) * 0.2;
    smoothstep(cos_cone, soft, cos_angle_to_point)
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A light as the observation test sees it.
#[derive(Clone, Copy, Debug)]
pub struct PointLight {
    pub position: Vec3,
    pub intensity: f32,
    pub range: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct SpotLight {
    pub position: Vec3,
    pub direction: Vec3,
    pub intensity: f32,
    pub range: f32,
    pub cone: f32,
}

/// Unshadowed irradiance (up to the shared BRDF factor) at `point`.
pub fn irradiance(point: Vec3, points: &[PointLight], spot: Option<&SpotLight>) -> f32 {
    let mut total: f32 = AMBIENT_IRRADIANCE
        + points
            .iter()
            .map(|l| l.intensity * falloff((point - l.position).length_squared(), l.range))
            .sum::<f32>();
    if let Some(spot) = spot {
        let to_point = point - spot.position;
        let cos = to_point.normalize_or_zero().dot(spot.direction);
        total +=
            spot.intensity * falloff(to_point.length_squared(), spot.range) * cone(cos, spot.cone);
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falloff_reaches_zero_at_range_and_is_unbounded_without_one() {
        assert_eq!(falloff(49.0, 7.0), 0.0);
        assert!(falloff(48.0, 7.0) > 0.0);
        assert!((falloff(4.0, 0.0) - 0.25).abs() < 1e-6);
        // Close in, the window barely changes inverse-square.
        assert!((falloff(1.0, 7.0) / falloff(1.0, 0.0)) > 0.99);
    }

    #[test]
    fn spot_cone_is_hard_outside_and_full_on_axis() {
        assert_eq!(cone(0.0, 0.4), 0.0);
        assert!((cone(1.0, 0.4) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn ambient_alone_is_too_dark_to_see() {
        assert!(irradiance(Vec3::ZERO, &[], None) < VISIBLE_IRRADIANCE);
    }

    #[test]
    fn a_ceiling_light_does_not_reach_the_next_aisle_but_one() {
        let light = PointLight {
            position: Vec3::new(0.0, 3.3, 0.0),
            intensity: 9.0,
            range: 7.0,
        };
        let under = irradiance(Vec3::new(0.0, 1.1, 0.0), &[light], None);
        let two_cells_over = irradiance(Vec3::new(8.0, 1.1, 0.0), &[light], None);
        assert!(under > VISIBLE_IRRADIANCE * 10.0);
        assert!(two_cells_over < VISIBLE_IRRADIANCE);
    }
}
