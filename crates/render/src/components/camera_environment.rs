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
/// a camera without one gets no fog and the default ambient light.
#[derive(Component, Clone, Copy, Debug, PartialEq, Reflect)]
#[reflect(Component, Default, SceneComponent)]
pub struct CameraEnvironment {
    pub fog: Option<Fog>,
    /// Ambient light, as a colour multiplied by each surface's albedo.
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
    pub fn with_fog(mut self, fog: Fog) -> Self {
        self.fog = Some(fog);
        self
    }

    pub fn with_ambient(mut self, ambient: Color) -> Self {
        self.ambient = ambient;
        self
    }

    /// The ambient light the standard material is drawn with.
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

/// Exponential-squared distance fog toward `color`, starting at `start`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect)]
pub struct Fog {
    pub color: Color,
    pub density: f32,
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
    use glam::Mat4;

    use super::*;
    use crate::components::camera::Camera;

    struct Filled {
        case: &'static str,
        environment: CameraEnvironment,
        fog_color: Vec4,
        fog_params: Vec4,
        ambient: Vec4,
    }

    #[test]
    fn an_environment_fills_the_uniform_with_its_fog_and_ambient_and_none_restores_the_default() {
        let default_ambient = Vec4::new(
            DEFAULT_AMBIENT_INTENSITY,
            DEFAULT_AMBIENT_INTENSITY,
            DEFAULT_AMBIENT_INTENSITY,
            1.0,
        );
        let cases = [
            Filled {
                case: "the default environment",
                environment: CameraEnvironment::default(),
                fog_color: Vec4::ZERO,
                fog_params: Vec4::ZERO,
                ambient: default_ambient,
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
                fog_color: Vec4::new(0.0, 0.0, 0.0, 1.0),
                fog_params: Vec4::new(0.1, 4.0, 0.0, 0.0),
                ambient: Vec4::ONE,
            },
        ];
        for case in &cases {
            let mut uniform = CameraUniform::new();
            case.environment.fill(&mut uniform);
            assert_eq!(uniform.fog_color, case.fog_color, "{}", case.case);
            assert_eq!(uniform.fog_params, case.fog_params, "{}", case.case);
            assert_eq!(uniform.ambient, case.ambient, "{}", case.case);
        }
        let mut uniform = CameraUniform::new();
        cases[1].environment.fill(&mut uniform);
        uniform.update_view_proj(
            &Camera::default(),
            None,
            &GlobalTransform::new(Mat4::IDENTITY),
        );
        assert_eq!(uniform.ambient, default_ambient, "no environment");
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
