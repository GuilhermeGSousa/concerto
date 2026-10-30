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
    use std::any::TypeId;

    use concerto_ecs::{reflect::AppTypeRegistry, world::World};
    use concerto_foundation::transform::GlobalTransform;
    use concerto_reflect::{apply_value, to_value};
    use glam::Mat4;

    use super::*;
    use crate::components::camera::Camera;

    #[test]
    fn no_environment_means_no_fog_and_default_ambient() {
        let mut uniform = CameraUniform::new();
        CameraEnvironment::default().fill(&mut uniform);
        assert_eq!(uniform.fog_color.w, 0.0);
        assert_eq!(uniform.ambient.w, 1.0);
        assert_eq!(
            CameraEnvironment::default().ambient_light().r,
            DEFAULT_AMBIENT_INTENSITY
        );
    }

    #[test]
    fn a_default_environment_has_the_default_ambient_not_white() {
        assert_eq!(
            CameraEnvironment::default().ambient.to_linear(),
            LinearRgba::new(
                DEFAULT_AMBIENT_INTENSITY,
                DEFAULT_AMBIENT_INTENSITY,
                DEFAULT_AMBIENT_INTENSITY,
                1.0,
            )
        );
    }

    #[test]
    fn a_uniform_with_no_environment_has_the_default_ambient() {
        let default = Vec4::new(
            DEFAULT_AMBIENT_INTENSITY,
            DEFAULT_AMBIENT_INTENSITY,
            DEFAULT_AMBIENT_INTENSITY,
            1.0,
        );
        let mut uniform = CameraUniform::new();
        assert_eq!(uniform.ambient, default);

        CameraEnvironment::default()
            .with_ambient(Color::WHITE)
            .fill(&mut uniform);
        uniform.update_view_proj(
            &Camera::default(),
            None,
            &GlobalTransform::new(Mat4::IDENTITY),
        );
        assert_eq!(uniform.ambient, default);
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
        let mut uniform = CameraUniform::new();
        environment.fill(&mut uniform);
        assert_eq!(uniform.fog_color.w, 1.0);
        assert_eq!(uniform.fog_params.x, 0.1);
        assert_eq!(uniform.fog_params.y, 4.0);
        assert_eq!(uniform.ambient, Vec4::ONE);
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

    #[test]
    fn editing_a_camera_environment_changes_the_filled_uniform() {
        let mut world = World::new();
        world.register_component::<CameraEnvironment>();
        world.register_type::<CameraEnvironment>();
        let camera = world.spawn(CameraEnvironment::default());

        let mut before = CameraUniform::new();
        world
            .get_component_for_entity::<CameraEnvironment>(camera)
            .unwrap()
            .fill(&mut before);
        assert_eq!(before.fog_color.w, 0.0);
        assert_eq!(before.ambient.w, 1.0);

        let replacement = CameraEnvironment::default()
            .with_fog(Fog {
                color: Color::BLUE,
                density: 0.2,
                start: 4.0,
            })
            .with_ambient(Color::WHITE);
        let stored = to_value(&replacement).unwrap();
        let reflect = world
            .get_resource::<AppTypeRegistry>()
            .unwrap()
            .get_type_data::<ReflectComponent>(TypeId::of::<CameraEnvironment>())
            .expect("a camera environment is an editable component");
        assert!((reflect.edit)(&mut world, camera, &mut |value| {
            apply_value(value, &stored);
        }));

        let mut after = CameraUniform::new();
        world
            .get_component_for_entity::<CameraEnvironment>(camera)
            .unwrap()
            .fill(&mut after);
        assert_eq!(after.fog_color.w, 1.0);
        assert_eq!(after.fog_params.x, 0.2);
        assert_eq!(after.fog_params.y, 4.0);
        assert_eq!(after.ambient, Vec4::ONE);
    }
}
