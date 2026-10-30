use concerto_ecs::{Query, Res, ResMut};
use concerto_foundation::{
    math::spring_damper::{simple_spring_damper_exact_quat, spring_character_update},
    time::Time,
};
use concerto_physics::{body::BodyId, physics_state::PhysicsState};
use glam::Vec3;

use crate::simulation::{predict_trajectory, CharacterSimulation, DesiredMotion};

/// Advances the spring one fixed step towards the desired motion and sets the body's horizontal velocity to it.
pub fn drive_body(
    query: Query<(&mut CharacterSimulation, &DesiredMotion, &BodyId)>,
    mut physics: ResMut<PhysicsState>,
) {
    let dt = Time::fixed_delta_time().as_secs_f32();
    for (mut simulation, desired, body_id) in query.iter() {
        let desired_velocity = Vec3::new(desired.velocity.x, 0.0, desired.velocity.z);
        let state = spring_character_update(
            simulation.state,
            desired_velocity,
            simulation.velocity_halflife,
            dt,
        );
        let (rotation, angular_velocity) = simple_spring_damper_exact_quat(
            simulation.rotation,
            simulation.angular_velocity,
            desired.rotation,
            simulation.rotation_halflife,
            dt,
        );
        let vertical = physics.linear_velocity(*body_id).y;
        physics.set_linear_velocity(
            *body_id,
            Vec3::new(state.velocity.x, vertical, state.velocity.z),
        );
        simulation.state = state;
        simulation.rotation = rotation;
        simulation.angular_velocity = angular_velocity;
    }
}

/// Sets the simulation's horizontal position to the body's position after collisions.
pub fn read_back_body(
    query: Query<(&mut CharacterSimulation, &BodyId)>,
    physics: Res<PhysicsState>,
) {
    for (mut simulation, body_id) in query.iter() {
        let position = physics.body_transform(*body_id).translation;
        simulation.state.position.x = position.x;
        simulation.state.position.z = position.z;
    }
}

/// Fills each simulation's trajectory from its current state and desired motion.
pub fn predict_trajectory_system(
    query: Query<(&mut CharacterSimulation, &DesiredMotion, &BodyId)>,
) {
    for (mut simulation, desired, _) in query.iter() {
        let trajectory = predict_trajectory(&simulation, desired);
        simulation.trajectory = trajectory;
    }
}

#[cfg(test)]
mod tests {
    use concerto_ecs::system::executor::single_thread::SingleThreadedExecutor;
    use concerto_ecs::system::schedule::Schedule;
    use concerto_ecs::world::World;
    use concerto_ecs::Entity;
    use concerto_foundation::transform::Transform;
    use concerto_physics::collider::{register_colliders, Collider};
    use concerto_physics::rigid_body::RigidBody;
    use glam::Quat;

    use super::*;
    use crate::simulation::CharacterSimulation;

    const EPS: f32 = 1.0e-4;

    fn world_with_body(
        position: Vec3,
        simulation: CharacterSimulation,
        desired: DesiredMotion,
    ) -> (World, Entity) {
        let mut world = World::new();
        world.register_component::<Collider>();
        world.register_component::<Transform>();
        world.insert_resource(PhysicsState::new());
        let entity = world.spawn((
            RigidBody::default(),
            Collider::capsule(0.5, 0.3),
            Transform::from_translation_rotation(position, Quat::IDENTITY),
            simulation,
            desired,
        ));

        let mut registration = Schedule::new();
        registration.add_system(register_colliders);
        registration
            .compile::<SingleThreadedExecutor>(&mut world)
            .run(&mut world);
        (world, entity)
    }

    fn run_once<M>(world: &mut World, system: impl concerto_ecs::IntoSystemConfig<M>) {
        let mut schedule = Schedule::new();
        schedule.add_system(system);
        schedule.compile::<SingleThreadedExecutor>(world).run(world);
    }

    fn body_of(world: &World, entity: Entity) -> BodyId {
        *world.get_component_for_entity::<BodyId>(entity).unwrap()
    }

    #[test]
    fn drive_body_keeps_vertical_velocity_and_sets_horizontal_to_the_spring() {
        let desired = DesiredMotion {
            velocity: Vec3::new(4.0, 7.0, -2.0),
            rotation: Quat::IDENTITY,
        };
        let simulation = CharacterSimulation::default();
        let expected = spring_character_update(
            simulation.state,
            Vec3::new(4.0, 0.0, -2.0),
            simulation.velocity_halflife,
            Time::fixed_delta_time().as_secs_f32(),
        );
        let (mut world, entity) = world_with_body(Vec3::new(0.0, 5.0, 0.0), simulation, desired);
        let body = body_of(&world, entity);
        world
            .get_resource_mut::<PhysicsState>()
            .unwrap()
            .set_linear_velocity(body, Vec3::new(0.0, -3.0, 0.0));

        run_once(&mut world, drive_body);

        let velocity = world
            .get_resource::<PhysicsState>()
            .unwrap()
            .linear_velocity(body);
        assert!((velocity.x - expected.velocity.x).abs() < EPS);
        assert!((velocity.y + 3.0).abs() < EPS);
        assert!((velocity.z - expected.velocity.z).abs() < EPS);

        let simulation = world
            .get_component_for_entity::<CharacterSimulation>(entity)
            .unwrap();
        assert!((simulation.state.velocity - expected.velocity).length() < EPS);
        assert!((simulation.state.position - expected.position).length() < EPS);
    }

    #[test]
    fn drive_body_turns_the_facing_towards_the_desired_rotation() {
        let desired = DesiredMotion {
            velocity: Vec3::ZERO,
            rotation: Quat::from_rotation_y(1.5),
        };
        let (mut world, entity) = world_with_body(
            Vec3::new(0.0, 5.0, 0.0),
            CharacterSimulation::default(),
            desired,
        );

        run_once(&mut world, drive_body);

        let simulation = world
            .get_component_for_entity::<CharacterSimulation>(entity)
            .unwrap();
        let before = Quat::IDENTITY.angle_between(desired.rotation);
        assert!(simulation.rotation.angle_between(desired.rotation) < before);
        assert!(simulation.angular_velocity.length() > 0.0);
    }

    #[test]
    fn read_back_body_replaces_horizontal_position_and_keeps_velocity() {
        let mut simulation = CharacterSimulation::default();
        simulation.state.velocity = Vec3::new(1.0, 0.0, 2.0);
        simulation.state.position = Vec3::new(9.0, 0.0, 9.0);
        let (mut world, entity) = world_with_body(
            Vec3::new(3.0, 5.0, -2.0),
            simulation,
            DesiredMotion::default(),
        );

        run_once(&mut world, read_back_body);

        let simulation = world
            .get_component_for_entity::<CharacterSimulation>(entity)
            .unwrap();
        assert!((simulation.state.position.x - 3.0).abs() < EPS);
        assert!(simulation.state.position.y.abs() < EPS);
        assert!((simulation.state.position.z + 2.0).abs() < EPS);
        assert_eq!(simulation.state.velocity, Vec3::new(1.0, 0.0, 2.0));
    }

    #[test]
    fn predict_trajectory_system_fills_the_trajectory() {
        let desired = DesiredMotion {
            velocity: Vec3::new(3.0, 0.0, 4.0),
            rotation: Quat::from_rotation_y(1.0),
        };
        let simulation = CharacterSimulation::default();
        let expected = predict_trajectory(&simulation, &desired);
        let (mut world, entity) = world_with_body(Vec3::new(0.0, 5.0, 0.0), simulation, desired);

        run_once(&mut world, predict_trajectory_system);

        let simulation = world
            .get_component_for_entity::<CharacterSimulation>(entity)
            .unwrap();
        assert_eq!(simulation.trajectory, expected);
    }

    #[test]
    fn entity_without_a_body_is_skipped() {
        let mut world = World::new();
        world.insert_resource(PhysicsState::new());
        let entity = world.spawn((CharacterSimulation::default(),));

        run_once(&mut world, drive_body);
        run_once(&mut world, predict_trajectory_system);

        let simulation = world
            .get_component_for_entity::<CharacterSimulation>(entity)
            .unwrap();
        assert!(simulation.trajectory.is_empty());
        assert_eq!(simulation.state.velocity, Vec3::ZERO);
    }
}
