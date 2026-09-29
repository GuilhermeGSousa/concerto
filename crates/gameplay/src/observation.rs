//! Whether the player can see something: a conservative test, run against the
//! main camera every frame, that an entity is in view, not hidden behind
//! physics geometry and, optionally, lit well enough to make out on screen.
//!
//! Useful for anything that must react to being looked at, or must only act
//! unseen: enemies that freeze under the player's gaze, stealth guards that
//! spot a lit player, props that change when nobody is watching.

use concerto_director::MainCamera;
use concerto_ecs::{Component, Entity, Query, Res, ResMut, Resource, With};
use concerto_foundation::transform::GlobalTransform;
use concerto_physics::physics_state::PhysicsState;
use concerto_render::components::{camera::Camera, light::Light};
use glam::{Quat, Vec3};

/// An entity whose visibility from the main camera is tested every frame.
#[derive(Component)]
pub struct Observable {
    /// Points, in the entity's local space, that count as parts of it:
    /// seeing any one of them sees the entity.
    pub sample_points: Vec<Vec3>,
    /// Light level a sample point needs to count as seen (see
    /// [`light_level`]); `None` means lighting is ignored.
    pub min_light: Option<f32>,
    observed: bool,
}

impl Observable {
    pub fn new(sample_points: impl Into<Vec<Vec3>>) -> Self {
        Self {
            sample_points: sample_points.into(),
            min_light: None,
            observed: false,
        }
    }

    /// Only counts the entity as seen where it is lit to at least `min_light`.
    pub fn with_min_light(mut self, min_light: f32) -> Self {
        self.min_light = Some(min_light);
        self
    }

    /// Whether the entity was seen in the last rendered frame, or the one
    /// before it.
    pub fn is_observed(&self) -> bool {
        self.observed
    }
}

#[derive(Resource)]
pub struct ObservationSettings {
    /// Radians added to each side of the camera's field of view, so that
    /// something just off the edge of the screen still counts as seen.
    pub view_margin: f32,
    /// Points closer to the eye than this always count as lit.
    pub near_sight: f32,
    /// Distance from the eye at which occlusion rays start, so the collider
    /// of whatever carries the camera (usually the player) does not block
    /// them.
    pub eye_clearance: f32,
}

impl Default for ObservationSettings {
    fn default() -> Self {
        Self {
            view_margin: 0.06,
            near_sight: 2.5,
            eye_clearance: 0.4,
        }
    }
}

/// A camera's pose and lens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewpoint {
    pub position: Vec3,
    pub rotation: Quat,
    /// Vertical field of view, in radians.
    pub fovy: f32,
    pub aspect: f32,
}

impl Viewpoint {
    pub fn from_camera(camera: &Camera, transform: &GlobalTransform) -> Self {
        Self {
            position: transform.translation(),
            rotation: transform.rotation(),
            fovy: camera.fovy,
            aspect: camera.aspect,
        }
    }

    /// Whether `point` is inside the view frustum widened by `margin` radians
    /// on every side. Points right next to the eye count as in view even
    /// behind the near plane.
    pub fn contains(&self, point: Vec3, margin: f32) -> bool {
        let local = self.rotation.inverse() * (point - self.position);
        let depth = -local.z;
        if depth < 0.02 {
            return local.length() < 0.5;
        }
        let half_v = self.fovy * 0.5;
        let half_h = (half_v.tan() * self.aspect).atan();
        local.y.abs() <= depth * (half_v + margin).tan()
            && local.x.abs() <= depth * (half_h + margin).tan()
    }
}

/// Whether nothing in the physics world blocks the segment from `from` to
/// `to`. A hit on `target` itself, or past `to`, does not block.
pub fn has_line_of_sight(
    physics: &PhysicsState,
    from: Vec3,
    to: Vec3,
    target: Option<Entity>,
) -> bool {
    let distance = from.distance(to);
    if distance < 0.05 {
        return true;
    }
    let Some(hit) = physics.cast_ray(from, to - from) else {
        return true;
    };
    (target.is_some() && hit.entity == target) || hit.fraction * distance >= distance - 0.05
}

/// How brightly the standard material would light a white surface at `point`
/// facing every light, seen from `camera`: the camera's ambient term plus each
/// light's diffuse contribution, dimmed by the camera's fog. `0.0` is black.
/// Shadows are ignored, so this errs on the side of "lit".
pub fn light_level<'a>(
    point: Vec3,
    camera: &Camera,
    eye: Vec3,
    lights: impl IntoIterator<Item = (&'a Light, &'a GlobalTransform)>,
) -> f32 {
    let direct: f32 = lights
        .into_iter()
        .map(|(light, transform)| {
            light.color.to_linear().luminance()
                * light.intensity
                * light.attenuation_at(transform, point)
        })
        .sum();
    let level = camera.ambient_light().luminance() + direct / std::f32::consts::PI;
    let fog = camera
        .fog
        .map_or(0.0, |fog| fog.amount(eye.distance(point)));
    level * (1.0 - fog)
}

/// The main camera's viewpoint last frame, so an observable glimpsed just
/// before a quick turn still counts as seen.
#[derive(Resource, Default)]
pub(crate) struct PreviousViewpoint(Option<Viewpoint>);

pub(crate) fn update_observation(
    cameras: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    lights: Query<(&Light, &GlobalTransform)>,
    observables: Query<(Entity, &mut Observable, &GlobalTransform)>,
    physics: Res<PhysicsState>,
    settings: Res<ObservationSettings>,
    mut previous: ResMut<PreviousViewpoint>,
) {
    let Some((camera, camera_transform)) = cameras.iter().next() else {
        for (_, mut observable, _) in observables.iter() {
            observable.observed = false;
        }
        return;
    };
    let current = Viewpoint::from_camera(camera, camera_transform);
    let views = [Some(current), previous.0.replace(current)];
    let views = views.iter().flatten();
    let lights: Vec<_> = lights.iter().collect();

    for (entity, mut observable, transform) in observables.iter() {
        let matrix = transform.matrix();
        let observed = observable.sample_points.iter().any(|local| {
            let point = matrix.transform_point3(*local);
            let lit = observable.min_light.is_none_or(|min_light| {
                current.position.distance(point) < settings.near_sight
                    || light_level(point, camera, current.position, lights.iter().copied())
                        >= min_light
            });
            lit && views.clone().any(|view| {
                if !view.contains(point, settings.view_margin) {
                    return false;
                }
                let to_point = point - view.position;
                let clearance = settings.eye_clearance.min(to_point.length());
                let from = view.position + to_point.normalize_or_zero() * clearance;
                has_line_of_sight(&physics, from, point, Some(entity))
            })
        });
        observable.observed = observed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use concerto_color::Color;
    use concerto_render::components::camera::Fog;
    use glam::Mat4;

    fn viewpoint(aspect: f32) -> Viewpoint {
        Viewpoint {
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            fovy: 1.2,
            aspect,
        }
    }

    #[test]
    fn viewpoint_contains_matches_the_frustum() {
        let view = viewpoint(16.0 / 9.0);
        assert!(view.contains(Vec3::new(0.0, 0.0, -5.0), 0.0));
        assert!(!view.contains(Vec3::new(0.0, 0.0, 5.0), 0.0));

        let half_h = ((view.fovy * 0.5).tan() * view.aspect).atan();
        let just_outside = Vec3::new((half_h + 0.03).tan() * 5.0, 0.0, -5.0);
        assert!(!view.contains(just_outside, 0.0));
        assert!(view.contains(just_outside, 0.05));

        let above = Vec3::new(0.0, (view.fovy * 0.5 + 0.1).tan() * 5.0, -5.0);
        assert!(!view.contains(above, 0.05));
    }

    #[test]
    fn points_at_the_eye_are_in_view() {
        let view = viewpoint(1.0);
        assert!(view.contains(Vec3::new(0.0, 0.0, 0.3), 0.0));
        assert!(!view.contains(Vec3::new(0.0, 0.0, 3.0), 0.0));
    }

    #[test]
    fn viewpoint_follows_rotation() {
        let view = Viewpoint {
            rotation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
            ..viewpoint(1.0)
        };
        assert!(view.contains(Vec3::new(-5.0, 0.0, 0.0), 0.0));
        assert!(!view.contains(Vec3::new(0.0, 0.0, -5.0), 0.0));
    }

    fn dark_camera() -> Camera {
        Camera {
            ambient: Some(Color::rgba(0.0, 0.0, 0.0, 1.0)),
            ..Default::default()
        }
    }

    #[test]
    fn light_level_adds_lights_to_ambient() {
        let camera = dark_camera();
        let light = Light::point_light().with_intensity(10.0).with_range(6.0);
        let transform = GlobalTransform::new(Mat4::from_translation(Vec3::Y * 3.0));
        let lights = [(&light, &transform)];

        assert_eq!(light_level(Vec3::ZERO, &camera, Vec3::ZERO, []), 0.0);
        let under = light_level(Vec3::ZERO, &camera, Vec3::ZERO, lights);
        let out_of_range = light_level(Vec3::X * 8.0, &camera, Vec3::ZERO, lights);
        assert!(under > 0.3);
        assert_eq!(out_of_range, 0.0);
    }

    #[test]
    fn fog_hides_distant_light() {
        let camera = Camera {
            fog: Some(Fog {
                color: Color::BLACK,
                density: 0.5,
                start: 1.0,
            }),
            ..Default::default()
        };
        let near = light_level(Vec3::NEG_Z, &camera, Vec3::ZERO, []);
        let far = light_level(Vec3::NEG_Z * 20.0, &camera, Vec3::ZERO, []);
        assert!(near > 0.0);
        assert!(far < near * 1e-3);
    }

    mod systems {
        use super::super::*;
        use concerto_color::Color;
        use concerto_ecs::{
            system::{executor::single_thread::SingleThreadedExecutor, schedule::Schedule},
            world::World,
        };
        use concerto_foundation::transform::Transform;
        use concerto_physics::{
            collider::{Collider, register_colliders},
            rigid_body::{MotionType, RigidBody},
        };

        /// A main camera at the origin looking down -Z, and an observable
        /// 6 m in front of it.
        fn world() -> (World, Entity, Entity) {
            let mut world = World::new();
            world.register_component::<Collider>();
            world.register_component::<Transform>();
            world.insert_resource(PhysicsState::new());
            world.insert_resource(ObservationSettings::default());
            world.insert_resource(PreviousViewpoint::default());
            let camera = world.spawn((
                MainCamera,
                Camera::perspective(1.2, 1.0),
                Transform::from_translation(Vec3::Y),
            ));
            let observable = world.spawn((
                Observable::new([Vec3::ZERO]),
                Transform::from_translation(Vec3::new(0.0, 1.0, -6.0)),
            ));
            (world, camera, observable)
        }

        fn observed(world: &mut World, observable: Entity) -> bool {
            let mut schedule = Schedule::new();
            schedule.add_system(register_colliders);
            schedule.add_system(update_observation);
            schedule.compile::<SingleThreadedExecutor>(world).run(world);
            world
                .get_component_for_entity::<Observable>(observable)
                .unwrap()
                .is_observed()
        }

        #[test]
        fn an_observable_in_plain_view_is_observed() {
            let (mut world, _, observable) = world();
            assert!(observed(&mut world, observable));
        }

        #[test]
        fn a_wall_hides_it() {
            let (mut world, _, observable) = world();
            world.spawn((
                Collider::cuboid(3.0, 3.0, 0.1),
                Transform::from_translation(Vec3::new(0.0, 1.0, -3.0)),
            ));
            // The previous frame's view is only remembered once there was one.
            assert!(!observed(&mut world, observable));
            assert!(!observed(&mut world, observable));
        }

        #[test]
        fn its_own_collider_and_the_viewers_do_not_hide_it() {
            let (mut world, _, observable) = world();
            world.insert(Collider::sphere(0.5), observable);
            world.spawn((
                RigidBody {
                    motion_type: MotionType::Kinematic,
                    ..Default::default()
                },
                Collider::capsule(0.6, 0.3),
                Transform::from_translation(Vec3::Y),
            ));
            assert!(observed(&mut world, observable));
        }

        #[test]
        fn darkness_hides_it_unless_it_is_close() {
            let (mut world, camera, observable) = world();
            world
                .get_component_for_entity_mut::<Camera>(camera)
                .unwrap()
                .ambient = Some(Color::BLACK);
            world
                .get_component_for_entity_mut::<Observable>(observable)
                .unwrap()
                .min_light = Some(0.01);
            assert!(!observed(&mut world, observable));

            world.spawn((
                Light::point_light().with_intensity(5.0),
                Transform::from_translation(Vec3::new(0.0, 2.5, -6.0)),
            ));
            assert!(observed(&mut world, observable));
        }

        #[test]
        fn looking_away_hides_it_after_a_frame() {
            let (mut world, camera, observable) = world();
            assert!(observed(&mut world, observable));
            // Transforms are not propagated here, so turn the global pose.
            let turned = Transform::from_translation_rotation(
                Vec3::Y,
                Quat::from_rotation_y(std::f32::consts::PI),
            );
            *world
                .get_component_for_entity_mut::<GlobalTransform>(camera)
                .unwrap() = GlobalTransform::new(turned.compute_matrix());
            assert!(
                observed(&mut world, observable),
                "seen in the previous frame"
            );
            assert!(!observed(&mut world, observable));
        }
    }
}
