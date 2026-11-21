use concerto_ecs::{
    query::{filter::With, Query},
    resource::{Res, ResMut},
};
use concerto_foundation::transform::Transform;

use crate::{
    body::BodyId, interpolation::TransformInterpolation, physics_pipeline::PhysicsPipeline,
    physics_state::PhysicsState, rigid_body::RigidBody, settings::PhysicsSettings,
};

/// Steps the simulation and writes each simulated body's pose to its `Transform`; a static collider's `Transform` is left to whoever moves the entity.
pub fn step_simulation(
    query: Query<(&BodyId, &mut Transform, Option<&mut TransformInterpolation>), With<RigidBody>>,
    mut pipeline: ResMut<PhysicsPipeline>,
    mut state: ResMut<PhysicsState>,
    settings: Option<Res<PhysicsSettings>>,
) {
    {
        profiling::scope!("jolt::step");
        let settings = settings.map(|settings| *settings).unwrap_or_default();
        pipeline.step_with(&mut state, &settings);
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
    use concerto_ecs::system::executor::single_thread::SingleThreadedExecutor;
    use concerto_ecs::system::schedule::Schedule;
    use concerto_ecs::world::World;
    use concerto_foundation::time::Time;
    use concerto_foundation::transform::Transform;
    use glam::Vec3;

    use crate::collider::{register_colliders, Collider};
    use crate::physics_pipeline::PhysicsPipeline;
    use crate::physics_state::PhysicsState;

    use super::step_simulation;

    #[test]
    fn a_step_leaves_a_static_colliders_transform_to_its_owner() {
        let mut world = World::new();
        world.register_component::<Collider>();
        world.register_component::<Transform>();
        world.insert_resource(PhysicsState::new());
        world.insert_resource(PhysicsPipeline::new());
        world.insert_resource(Time::new());
        let door = world.spawn((Collider::cuboid(0.5, 1.0, 0.05), Transform::default()));
        let mut registration = Schedule::new();
        registration.add_system(register_colliders);
        registration
            .compile::<SingleThreadedExecutor>(&mut world)
            .run(&mut world);
        let moved = Vec3::new(0.5, 0.0, 0.5);
        world
            .get_component_for_entity_mut::<Transform>(door)
            .unwrap()
            .translation = moved;

        let mut schedule = Schedule::new();
        schedule.add_system(step_simulation);
        schedule
            .compile::<SingleThreadedExecutor>(&mut world)
            .run(&mut world);

        assert_eq!(
            world
                .get_component_for_entity::<Transform>(door)
                .unwrap()
                .translation,
            moved
        );
    }
}
