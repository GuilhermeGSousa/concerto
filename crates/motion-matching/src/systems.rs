use concerto_animation::player::AnimationPlayer;
use concerto_ecs::{Query, Res, ResMut};
use concerto_foundation::time::Time;
use concerto_physics::{body::BodyId, physics_state::PhysicsState};

use crate::{
    node::{MotionMatchingTrajectory, TRAJECTORY_KEY},
    simulation::{CharacterSimulation, DesiredMotion},
};

/// Advances the springs one fixed step towards the desired motion and sets the body's horizontal velocity to the spring's.
pub fn drive_body(
    query: Query<(&mut CharacterSimulation, &DesiredMotion, &BodyId)>,
    mut physics: ResMut<PhysicsState>,
) {
    let dt = Time::fixed_delta_time().as_secs_f32();
    for (mut simulation, desired, body_id) in query.iter() {
        let velocity = simulation.advance(desired, dt);
        let vertical = physics.linear_velocity(*body_id).y;
        physics.set_linear_velocity(*body_id, velocity.with_y(vertical));
    }
}

/// Predicts each character's path from where its body ended the step and writes it to its animation player's blackboard.
pub fn publish_trajectory(
    query: Query<(
        &CharacterSimulation,
        &DesiredMotion,
        &BodyId,
        &mut AnimationPlayer,
    )>,
    physics: Res<PhysicsState>,
) {
    for (simulation, desired, body_id, mut player) in query.iter() {
        let translation = physics.body_transform(*body_id).translation;
        player.set_param(
            TRAJECTORY_KEY,
            MotionMatchingTrajectory {
                translation,
                rotation: simulation.facing.value,
                samples: simulation.predict(translation, desired),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use concerto_ecs::system::executor::single_thread::SingleThreadedExecutor;
    use concerto_ecs::system::schedule::Schedule;
    use concerto_ecs::world::World;
    use concerto_ecs::Entity;
    use concerto_foundation::math::spring_damper::halflife_to_damping;
    use concerto_foundation::transform::Transform;
    use concerto_physics::collider::{register_colliders, Collider};
    use concerto_physics::rigid_body::RigidBody;
    use glam::{Quat, Vec3};

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

    fn published(world: &World, entity: Entity) -> Option<MotionMatchingTrajectory> {
        world
            .get_component_for_entity::<AnimationPlayer>(entity)
            .unwrap()
            .get_param::<MotionMatchingTrajectory>(TRAJECTORY_KEY)
            .cloned()
    }

    #[test]
    fn drive_body_keeps_vertical_velocity_and_sets_horizontal_to_the_spring() {
        let desired = DesiredMotion {
            velocity: Vec3::new(4.0, 7.0, -2.0),
            rotation: Quat::IDENTITY,
        };
        let simulation = CharacterSimulation::default();
        let rate = halflife_to_damping(simulation.velocity_halflife) / 2.0
            * Time::fixed_delta_time().as_secs_f32();
        let covered = 1.0 - (1.0 + rate) * (-rate).exp();
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
        assert!((velocity.x - 4.0 * covered).abs() < EPS);
        assert!((velocity.y + 3.0).abs() < EPS);
        assert!((velocity.z + 2.0 * covered).abs() < EPS);
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
        assert!(simulation.facing.value.angle_between(desired.rotation) < before);
        assert!(simulation.facing.velocity.length() > 0.0);
    }

    #[test]
    fn entity_without_a_body_is_skipped() {
        let mut world = World::new();
        world.insert_resource(PhysicsState::new());
        let entity = world.spawn((CharacterSimulation::default(), AnimationPlayer::new(1)));

        run_once(&mut world, drive_body);
        run_once(&mut world, publish_trajectory);

        let simulation = world
            .get_component_for_entity::<CharacterSimulation>(entity)
            .unwrap();
        assert_eq!(simulation.velocity.value, Vec3::ZERO);
        assert_eq!(published(&world, entity), None);
    }

    #[test]
    fn publish_trajectory_starts_the_path_at_the_body_and_keeps_the_facing() {
        let mut simulation = CharacterSimulation::default();
        simulation.velocity.value = Vec3::new(1.0, 0.0, 2.0);
        simulation.facing.value = Quat::from_rotation_y(0.5);
        let desired = DesiredMotion {
            velocity: Vec3::new(3.0, 0.0, 4.0),
            rotation: Quat::from_rotation_y(1.0),
        };
        let (mut world, entity) =
            world_with_body(Vec3::new(3.0, 5.0, -2.0), simulation.clone(), desired);
        world.insert(AnimationPlayer::new(1), entity);

        run_once(&mut world, publish_trajectory);

        let trajectory = published(&world, entity).unwrap();
        assert!((trajectory.translation.x - 3.0).abs() < EPS);
        assert!((trajectory.translation.z + 2.0).abs() < EPS);
        assert_eq!(trajectory.rotation, Quat::from_rotation_y(0.5));
        assert_eq!(
            trajectory.samples,
            simulation.predict(trajectory.translation, &desired)
        );
    }
}
