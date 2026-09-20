use ecs::{query::Query, resource::ResMut};
use essential::transform::Transform;

use crate::{
    body::BodyId, interpolation::TransformInterpolation, physics_pipeline::PhysicsPipeline,
    physics_state::PhysicsState,
};

pub fn step_simulation(
    query: Query<(&BodyId, &mut Transform, Option<&mut TransformInterpolation>)>,
    mut pipeline: ResMut<PhysicsPipeline>,
    mut state: ResMut<PhysicsState>,
) {
    {
        profiling::scope!("jolt::step");
        pipeline.step(&mut state);
    }

    {
        profiling::scope!("jolt::write_back_transforms");
        for (body_id, mut transform, interpolation) in query.iter() {
            let mut stepped_transform = state.body_transform(*body_id);

            stepped_transform.scale = transform.scale;
            if let Some(mut interpolation) = interpolation {
                interpolation.push(&stepped_transform);
            }
            **transform = stepped_transform;
        }
    }
}

#[cfg(test)]
mod tests {
    use ecs::system::executor::single_thread::SingleThreadedExecutor;
    use ecs::system::schedule::Schedule;
    use ecs::world::World;
    use essential::time::Time;
    use essential::transform::Transform;
    use glam::Vec3;

    use crate::collider::{register_colliders, Collider};
    use crate::physics_pipeline::PhysicsPipeline;
    use crate::physics_state::PhysicsState;
    use crate::rigid_body::RigidBody;
    use crate::shape::{PhysicsShape, SharedPhysicsShape};

    use super::step_simulation;

    /// A flat quad in the XZ plane, with `indices` spelled out by the caller
    /// so a test can choose the winding.
    fn plane_mesh(half_extent: f32, indices: Vec<u32>) -> mesh::Mesh {
        let corners = [
            [-half_extent, 0.0, -half_extent],
            [half_extent, 0.0, -half_extent],
            [half_extent, 0.0, half_extent],
            [-half_extent, 0.0, half_extent],
        ];
        mesh::Mesh {
            vertices: corners
                .into_iter()
                .map(|pos_coords| mesh::vertex::Vertex {
                    pos_coords,
                    normal: [0.0, 1.0, 0.0],
                    ..Default::default()
                })
                .collect(),
            indices,
        }
    }

    fn drop_sphere_onto(mesh: mesh::Mesh) -> f32 {
        let mut world = World::new();
        world.register_component::<Collider>();
        // Inserts the GlobalTransform that `register_colliders` reads.
        world.register_component::<Transform>();
        world.insert_resource(PhysicsState::new());
        world.insert_resource(PhysicsPipeline::new());
        world.insert_resource(Time::new());

        let shape = SharedPhysicsShape::new(PhysicsShape::from_mesh(&mesh).unwrap());
        world.spawn((Collider::from_shape(shape), Transform::default()));
        let sphere = world.spawn((
            RigidBody::default(),
            Collider::sphere(0.5),
            Transform::from_translation_rotation(Vec3::new(0.0, 4.0, 0.0), Default::default()),
        ));

        let mut registration = Schedule::new();
        registration.add_system(register_colliders);
        registration
            .compile::<SingleThreadedExecutor>(&mut world)
            .run(&mut world);

        let mut schedule = Schedule::new();
        schedule.add_system(step_simulation);
        let mut schedule = schedule.compile::<SingleThreadedExecutor>(&mut world);
        for _ in 0..240 {
            schedule.run(&mut world);
        }

        world
            .get_component_for_entity::<Transform>(sphere)
            .unwrap()
            .translation
            .y
    }

    /// A mesh collider wound counter-clockwise (the glTF convention, and the
    /// front face for both backends) stops a body landing on it.
    #[test]
    fn mesh_collider_stops_a_falling_body() {
        let landed = drop_sphere_onto(plane_mesh(20.0, vec![0, 2, 1, 0, 3, 2]));
        assert!(
            (landed - 0.5).abs() < 0.1,
            "sphere should rest on an up-facing mesh floor (y ~= 0.5), was {landed}"
        );
    }

    /// A collider's geometry must follow its entity's `Transform` scale.
    ///
    /// The floor here is a 1×1×1 half-extent box scaled 5×, so its top sits at
    /// y = 4 rather than y = 0. A sphere dropped onto it settles just above 4
    /// when the scale is applied, and falls all the way to 0.5 when it is
    /// ignored — which is what Sponza's 0.008 root scale hit in practice.
    #[test]
    fn collider_geometry_follows_transform_scale() {
        let mut world = World::new();
        world.register_component::<Collider>();
        // Inserts the GlobalTransform that `register_colliders` reads.
        world.register_component::<Transform>();
        world.insert_resource(PhysicsState::new());
        world.insert_resource(PhysicsPipeline::new());
        world.insert_resource(Time::new());

        world.spawn((
            Collider::cuboid(1.0, 1.0, 1.0),
            Transform::from_translation_rotation_scale(
                Vec3::new(0.0, -1.0, 0.0),
                Default::default(),
                Vec3::splat(5.0),
            ),
        ));
        let sphere = world.spawn((
            RigidBody::default(),
            Collider::sphere(0.5),
            Transform::from_translation_rotation(Vec3::new(0.0, 8.0, 0.0), Default::default()),
        ));

        let mut registration = Schedule::new();
        registration.add_system(register_colliders);
        registration
            .compile::<SingleThreadedExecutor>(&mut world)
            .run(&mut world);

        let mut schedule = Schedule::new();
        schedule.add_system(step_simulation);
        let mut schedule = schedule.compile::<SingleThreadedExecutor>(&mut world);
        for _ in 0..240 {
            schedule.run(&mut world);
        }

        let y = world
            .get_component_for_entity::<Transform>(sphere)
            .unwrap()
            .translation
            .y;
        assert!(
            (y - 4.5).abs() < 0.1,
            "sphere should rest on the scaled box top (y ~= 4.5), was {y}"
        );
    }
}
