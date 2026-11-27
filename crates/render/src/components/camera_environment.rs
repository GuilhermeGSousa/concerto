use concerto_color::{Color, LinearRgba};
use concerto_ecs::{
    component::Component,
    reflect::{ReflectComponent, ReflectSceneComponent},
};
use concerto_reflect::{Reflect, ReflectDefault};
use glam::Vec4;

use super::camera::CameraUniform;

/// Ambient light the standard material uses when a camera sets none.
pub const DEFAULT_AMBIENT_INTENSITY: f32 = 0.03;

/// Fog and ambient light for everything a camera draws with the standard
/// material. Goes on the same entity as the [`Camera`](super::camera::Camera);
/// a camera without one gets no fog and the default ambient light. A camera
/// with a [`CameraSkyLight`](crate::sky_light::CameraSkyLight) is lit by its
/// sky instead of `ambient`, and its fog takes the sky's color instead of
/// [`Fog::color`].
#[derive(Component, Clone, Copy, Debug, PartialEq, Reflect)]
#[reflect(Component, Default, SceneComponent)]
pub struct CameraEnvironment {
    /// The distance fog, or `None` for none.
    pub fog: Option<Fog>,
    /// Ambient light, as a colour multiplied by each surface's albedo; replaced by the sky's under a sky.
    pub ambient: Color,
}

impl Default for CameraEnvironment {
    fn default() -> Self {
        Self {
            fog: None,
            ambient: Color::rgba(
                DEFAULT_AMBIENT_INTENSITY,
                DEFAULT_AMBIENT_INTENSITY,
                DEFAULT_AMBIENT_INTENSITY,
                1.0,
            ),
        }
    }
}

impl CameraEnvironment {
    /// Sets [`Self::fog`].
    pub fn with_fog(mut self, fog: Fog) -> Self {
        self.fog = Some(fog);
        self
    }

    /// Sets [`Self::ambient`].
    pub fn with_ambient(mut self, ambient: Color) -> Self {
        self.ambient = ambient;
        self
    }

    /// The ambient light the standard material is drawn with, unless the camera has a sky, which lights it instead.
    pub fn ambient_light(&self) -> LinearRgba {
        self.ambient.to_linear()
    }

    /// Writes the fog and ambient fields of `uniform`. A zero fog alpha tells
    /// the shader to skip fog.
    pub(crate) fn fill(&self, uniform: &mut CameraUniform) {
        match self.fog {
            Some(fog) => {
                let c = fog.color.to_linear();
                uniform.fog_color = Vec4::new(c.r, c.g, c.b, 1.0);
                uniform.fog_params = Vec4::new(fog.density, fog.start, 0.0, 0.0);
            }
            None => {
                uniform.fog_color = Vec4::ZERO;
                uniform.fog_params = Vec4::ZERO;
            }
        }
        let c = self.ambient_light();
        uniform.ambient = Vec4::new(c.r, c.g, c.b, 1.0);
    }
}

/// Draws everything this camera sees with the standard material by base colour and emission alone, without lights, shadows, ambient light, occlusion or fog.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UnlitView;

/// Exponential-squared distance fog toward `color`, starting at `start`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect)]
pub struct Fog {
    /// The color fog fades toward; ignored under a sky, where fog takes the sky's color.
    pub color: Color,
    /// How quickly fog thickens past `start`, per world unit; the fog amount is
    /// `1 - exp(-(distance_past_start * density)²)`.
    pub density: f32,
    /// The distance from the camera, in world units, where fog begins.
    pub start: f32,
}

impl Fog {
    /// How much of a surface at `distance` is replaced by fog, `0..=1`.
    pub fn amount(&self, distance: f32) -> f32 {
        let d = (distance - self.start).max(0.0) * self.density;
        1.0 - (-(d * d)).exp()
    }
}

#[cfg(test)]
mod tests {
    use concerto_foundation::transform::GlobalTransform;
    use glam::{Mat4, Vec3};

    use super::*;
    use crate::components::camera::Camera;
    use crate::sky_light::{CameraSkyLight, SphericalHarmonics};

    struct Filled {
        case: &'static str,
        environment: CameraEnvironment,
        sky_light: Option<CameraSkyLight>,
        fog_color: Vec4,
        fog_params: Vec4,
        ambient: Vec4,
        sky_irradiance: [Vec4; 9],
        night_color: Vec4,
        night_ambient: Vec4,
        sky_lit: u32,
    }

    fn filled(
        environment: Option<&CameraEnvironment>,
        sky_light: Option<&CameraSkyLight>,
    ) -> CameraUniform {
        let mut uniform = CameraUniform::new();
        uniform.update_view_proj(
            &Camera::default(),
            environment,
            sky_light,
            &GlobalTransform::new(Mat4::IDENTITY),
        );
        uniform
    }

    #[test]
    fn an_environment_fills_the_uniform_with_its_fog_and_ambient_and_none_restores_the_default() {
        let default_ambient = Vec4::new(
            DEFAULT_AMBIENT_INTENSITY,
            DEFAULT_AMBIENT_INTENSITY,
            DEFAULT_AMBIENT_INTENSITY,
            1.0,
        );
        let sky_light = CameraSkyLight {
            irradiance: SphericalHarmonics {
                coefficients: std::array::from_fn(|index| Vec3::splat(index as f32)),
            },
            night_color: LinearRgba::new(0.1, 0.2, 0.3, 1.0),
            night_ambient: LinearRgba::new(0.4, 0.5, 0.6, 1.0),
        };
        let cases = [
            Filled {
                case: "the default environment",
                environment: CameraEnvironment::default(),
                sky_light: None,
                fog_color: Vec4::ZERO,
                fog_params: Vec4::ZERO,
                ambient: default_ambient,
                sky_irradiance: [Vec4::ZERO; 9],
                night_color: Vec4::ZERO,
                night_ambient: Vec4::ZERO,
                sky_lit: 0,
            },
            Filled {
                case: "fog and a white ambient",
                environment: CameraEnvironment::default()
                    .with_fog(Fog {
                        color: Color::BLACK,
                        density: 0.1,
                        start: 4.0,
                    })
                    .with_ambient(Color::WHITE),
                sky_light: None,
                fog_color: Vec4::new(0.0, 0.0, 0.0, 1.0),
                fog_params: Vec4::new(0.1, 4.0, 0.0, 0.0),
                ambient: Vec4::ONE,
                sky_irradiance: [Vec4::ZERO; 9],
                night_color: Vec4::ZERO,
                night_ambient: Vec4::ZERO,
                sky_lit: 0,
            },
            Filled {
                case: "a sky light",
                environment: CameraEnvironment::default(),
                sky_light: Some(sky_light),
                fog_color: Vec4::ZERO,
                fog_params: Vec4::ZERO,
                ambient: default_ambient,
                sky_irradiance: std::array::from_fn(|index| Vec3::splat(index as f32).extend(0.0)),
                night_color: Vec4::new(0.1, 0.2, 0.3, 1.0),
                night_ambient: Vec4::new(0.4, 0.5, 0.6, 1.0),
                sky_lit: 1,
            },
        ];
        for case in &cases {
            let uniform = filled(Some(&case.environment), case.sky_light.as_ref());
            assert_eq!(uniform.fog_color, case.fog_color, "{}", case.case);
            assert_eq!(uniform.fog_params, case.fog_params, "{}", case.case);
            assert_eq!(uniform.ambient, case.ambient, "{}", case.case);
            assert_eq!(uniform.sky_irradiance, case.sky_irradiance, "{}", case.case);
            assert_eq!(uniform.night_color, case.night_color, "{}", case.case);
            assert_eq!(uniform.night_ambient, case.night_ambient, "{}", case.case);
            assert_eq!(uniform.sky_lit, case.sky_lit, "{}", case.case);
        }
        let mut uniform = filled(Some(&cases[1].environment), Some(&sky_light));
        uniform.update_view_proj(
            &Camera::default(),
            None,
            None,
            &GlobalTransform::new(Mat4::IDENTITY),
        );
        assert_eq!(uniform.ambient, default_ambient, "no environment");
        assert_eq!(uniform.sky_lit, 0, "no sky light");
        assert_eq!(uniform.sky_irradiance, [Vec4::ZERO; 9], "no sky light");
    }

    #[test]
    fn fog_starts_at_start_and_thickens() {
        let fog = Fog {
            color: Color::BLACK,
            density: 0.5,
            start: 2.0,
        };
        assert_eq!(fog.amount(1.0), 0.0);
        assert_eq!(fog.amount(2.0), 0.0);
        assert!(fog.amount(4.0) > 0.5);
        assert!(fog.amount(20.0) > 0.999);
    }
}
