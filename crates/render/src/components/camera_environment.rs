use concerto_color::{Color, LinearRgba};
use concerto_ecs::{
    component::scene::{SceneComponent, SceneSpawnContext},
    component::Component,
    Entity,
};
use glam::Vec4;
use serde::{Deserialize, Serialize};

/// Ambient light the standard material uses when a camera sets none; matches
/// `AMBIENT_INTENSITY` in `shader.wgsl`.
pub const DEFAULT_AMBIENT_INTENSITY: f32 = 0.03;

/// Fog and ambient light for everything a camera draws with the standard
/// material. Goes on the same entity as the [`Camera`](super::camera::Camera);
/// a camera without one gets no fog and the default ambient light.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CameraEnvironment {
    #[serde(default)]
    pub fog: Option<Fog>,
    /// Ambient light, as a colour multiplied by each surface's albedo;
    /// `None` uses [`DEFAULT_AMBIENT_INTENSITY`].
    #[serde(default)]
    pub ambient: Option<Color>,
}

impl SceneComponent for CameraEnvironment {
    fn apply(self, entity: Entity, ctx: &mut SceneSpawnContext<'_>) {
        ctx.insert(self, entity);
    }
}

impl CameraEnvironment {
    pub fn with_fog(mut self, fog: Fog) -> Self {
        self.fog = Some(fog);
        self
    }

    pub fn with_ambient(mut self, ambient: Color) -> Self {
        self.ambient = Some(ambient);
        self
    }

    /// The ambient light the standard material is drawn with:
    /// [`CameraEnvironment::ambient`], or [`DEFAULT_AMBIENT_INTENSITY`] when
    /// unset.
    pub fn ambient_light(&self) -> LinearRgba {
        match self.ambient {
            Some(color) => color.to_linear(),
            None => LinearRgba::new(
                DEFAULT_AMBIENT_INTENSITY,
                DEFAULT_AMBIENT_INTENSITY,
                DEFAULT_AMBIENT_INTENSITY,
                1.0,
            ),
        }
    }

    /// The `(fog_color, fog_params, ambient)` camera-uniform fields. A zero
    /// alpha tells the shader to skip fog or use its default ambient.
    pub(crate) fn uniform_fields(environment: Option<&Self>) -> (Vec4, Vec4, Vec4) {
        let environment = environment.copied().unwrap_or_default();
        let (fog_color, fog_params) = match environment.fog {
            Some(fog) => {
                let c = fog.color.to_linear();
                (
                    Vec4::new(c.r, c.g, c.b, 1.0),
                    Vec4::new(fog.density, fog.start, 0.0, 0.0),
                )
            }
            None => (Vec4::ZERO, Vec4::ZERO),
        };
        let ambient = match environment.ambient {
            Some(color) => {
                let c = color.to_linear();
                Vec4::new(c.r, c.g, c.b, 1.0)
            }
            None => Vec4::ZERO,
        };
        (fog_color, fog_params, ambient)
    }
}

/// Exponential-squared distance fog toward `color`, starting at `start`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
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
    use super::*;

    #[test]
    fn no_environment_means_no_fog_and_default_ambient() {
        let (fog_color, _, ambient) = CameraEnvironment::uniform_fields(None);
        assert_eq!(fog_color.w, 0.0);
        assert_eq!(ambient.w, 0.0);
        assert_eq!(
            CameraEnvironment::default().ambient_light().r,
            DEFAULT_AMBIENT_INTENSITY
        );
    }

    #[test]
    fn fog_and_ambient_reach_the_uniform() {
        let environment = CameraEnvironment::default()
            .with_fog(Fog {
                color: Color::BLACK,
                density: 0.1,
                start: 4.0,
            })
            .with_ambient(Color::WHITE);
        let (fog_color, fog_params, ambient) =
            CameraEnvironment::uniform_fields(Some(&environment));
        assert_eq!(fog_color.w, 1.0);
        assert_eq!(fog_params.x, 0.1);
        assert_eq!(fog_params.y, 4.0);
        assert_eq!(ambient, Vec4::ONE);
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
