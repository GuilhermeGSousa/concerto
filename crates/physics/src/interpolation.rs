use ecs::{query::Query, resource::Res, Component};
use essential::{time::Time, transform::Transform};
use glam::{Quat, Vec3};

/// The last two fixed-step poses of a physics body.
///
/// Physics advances in [`Time::fixed_delta_time`] increments, so a body's raw
/// pose only changes on frames where a fixed step ran — rendered directly it
/// lurches forward once per step. `step_simulation` records the pose history
/// here and [`interpolate_body_transforms`] blends between the two poses every
/// frame, trading one fixed step of latency for smooth motion.
///
/// Inserted automatically alongside [`BodyId`](crate::body::BodyId) for
/// non-static bodies.
#[derive(Component, Clone, Copy)]
pub struct TransformInterpolation {
    prev_translation: Vec3,
    prev_rotation: Quat,
    curr_translation: Vec3,
    curr_rotation: Quat,
}

impl TransformInterpolation {
    pub(crate) fn from_transform(transform: &Transform) -> Self {
        Self {
            prev_translation: transform.translation,
            prev_rotation: transform.rotation,
            curr_translation: transform.translation,
            curr_rotation: transform.rotation,
        }
    }

    pub(crate) fn push(&mut self, transform: &Transform) {
        self.prev_translation = self.curr_translation;
        self.prev_rotation = self.curr_rotation;
        self.curr_translation = transform.translation;
        self.curr_rotation = transform.rotation;
    }

    pub(crate) fn sample(&self, alpha: f32) -> (Vec3, Quat) {
        (
            self.prev_translation.lerp(self.curr_translation, alpha),
            self.prev_rotation.slerp(self.curr_rotation, alpha),
        )
    }
}

/// Writes the interpolated fixed-step pose into each body's [`Transform`] so
/// frame-rate systems (transform propagation, rendering) see smooth motion
/// instead of the raw once-per-step jumps.
pub(crate) fn interpolate_body_transforms(
    bodies: Query<(&TransformInterpolation, &mut Transform)>,
    time: Res<Time>,
) {
    let alpha = time.fixed_alpha();
    for (interpolation, mut transform) in bodies.iter() {
        let (translation, rotation) = interpolation.sample(alpha);
        transform.translation = translation;
        transform.rotation = rotation;
    }
}

#[cfg(test)]
mod tests {
    use ecs::system::executor::single_thread::SingleThreadedExecutor;
    use ecs::system::schedule::Schedule;
    use ecs::world::World;
    use essential::{time::Time, transform::Transform};
    use glam::Vec3;

    use crate::collider::{register_colliders, Collider};
    use crate::physics_pipeline::PhysicsPipeline;
    use crate::physics_state::PhysicsState;
    use crate::rigid_body::RigidBody;
    use crate::simulation::step_simulation;

    use super::interpolate_body_transforms;

    /// Bodies are created by `register_colliders`, not by `Collider`'s
    /// lifecycle, so it has to run before anything steps the simulation.
    fn register_bodies(world: &mut World) {
        let mut schedule = Schedule::new();
        schedule.add_system(register_colliders);
        schedule.compile::<SingleThreadedExecutor>(world).run(world);
    }

    /// Jolt reports poses with no scale of its own, so stepping and
    /// interpolating a scaled body must leave its scale alone rather than
    /// resetting it to one.
    #[test]
    fn stepping_preserves_transform_scale() {
        let mut world = World::new();
        world.register_component::<Collider>();
        // Inserts the GlobalTransform that `register_colliders` reads.
        world.register_component::<Transform>();
        world.insert_resource(PhysicsState::new());
        world.insert_resource(PhysicsPipeline::new());
        world.insert_resource(Time::new());

        let scale = Vec3::new(2.0, 3.0, 4.0);
        let sphere = world.spawn((
            RigidBody::default(),
            Collider::sphere(1.0),
            Transform::from_translation_rotation_scale(
                Vec3::new(0.0, 10.0, 0.0),
                Default::default(),
                scale,
            ),
        ));
        register_bodies(&mut world);

        let mut schedule = Schedule::new();
        schedule.add_system(step_simulation);
        schedule.add_system(interpolate_body_transforms);
        let mut schedule = schedule.compile::<SingleThreadedExecutor>(&mut world);

        for _ in 0..3 {
            schedule.run(&mut world);
            let transform = world
                .get_component_for_entity::<Transform>(sphere)
                .unwrap()
                .clone();
            assert!(
                (transform.scale - scale).length() < 1e-6,
                "scale should survive the step, was {:?}",
                transform.scale
            );
        }
    }
}
