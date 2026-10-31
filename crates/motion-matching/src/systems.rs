use concerto_animation::player::AnimationPlayer;
use concerto_ecs::{entity::hierarchy::ChildOf, Entity, Query, Res, ResMut};
use concerto_foundation::time::Time;
use concerto_physics::{body::BodyId, physics_state::PhysicsState};

use crate::{
    displacement::DisplacementSimulation,
    node::{MotionMatchingTrajectory, TRAJECTORY_KEY},
    simulation::{CharacterSimulation, DesiredMotion},
};

/// Advances the springs one fixed step towards the desired motion and sets the body's horizontal velocity to the spring's, plus half of any pull, capped at `max_pull_speed`.
pub fn drive_body(
    query: Query<(&mut CharacterSimulation, &DesiredMotion, &BodyId)>,
    mut physics: ResMut<PhysicsState>,
) {
    let dt = Time::fixed_delta_time().as_secs_f32();
    for (mut simulation, desired, body_id) in query.iter() {
        simulation.previous_facing = simulation.facing.value;
        let velocity = simulation.advance(desired, dt);
        let pull = std::mem::take(&mut simulation.pull) / (2.0 * dt);
        let pull = pull.clamp_length_max(simulation.max_pull_speed);
        let vertical = physics.linear_velocity(*body_id).y;
        physics.set_linear_velocity(*body_id, (velocity + pull).with_y(vertical));
    }
}

fn displaced_capsule(
    entity: Entity,
    links: &Query<&DisplacementSimulation>,
    parents: &Query<&ChildOf>,
) -> Option<Entity> {
    let mut current = entity;
    loop {
        if let Some(link) = links.get_entity(current) {
            return Some(link.0);
        }
        current = parents.get_entity(current)?.parent();
    }
}

/// Predicts each character's path from where its body ended the step and writes it to its animation player's blackboard, whether the player shares the simulation's entity or sits on or under a character naming it.
pub fn publish_trajectory(
    players: Query<(Entity, &mut AnimationPlayer)>,
    simulations: Query<(&CharacterSimulation, &DesiredMotion, Option<&BodyId>)>,
    links: Query<&DisplacementSimulation>,
    parents: Query<&ChildOf>,
    physics: Res<PhysicsState>,
) {
    for (entity, mut player) in players.iter() {
        let capsule = if simulations.contains_entity(entity) {
            Some(entity)
        } else {
            displaced_capsule(entity, &links, &parents)
        };
        let Some((simulation, desired, Some(body_id))) =
            capsule.and_then(|capsule| simulations.get_entity(capsule))
        else {
            continue;
        };
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
    use concerto_ecs::entity::hierarchy::ChildOf;
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
    use crate::displacement::DisplacementSimulation;
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

    fn pulled_body(
        simulation: CharacterSimulation,
        desired: DesiredMotion,
    ) -> (World, Entity, BodyId) {
        let (mut world, entity) = world_with_body(Vec3::new(0.0, 5.0, 0.0), simulation, desired);
        let body = body_of(&world, entity);
        world
            .get_resource_mut::<PhysicsState>()
            .unwrap()
            .set_linear_velocity(body, Vec3::new(0.0, -3.0, 0.0));
        (world, entity, body)
    }

    fn velocity_of(world: &World, body: BodyId) -> Vec3 {
        world
            .get_resource::<PhysicsState>()
            .unwrap()
            .linear_velocity(body)
    }

    fn walking() -> DesiredMotion {
        DesiredMotion {
            velocity: Vec3::new(2.0, 0.0, -1.0),
            rotation: Quat::from_rotation_y(1.5),
        }
    }

    #[test]
    fn drive_body_adds_half_the_pull_and_leaves_the_springs_alone() {
        let dt = Time::fixed_delta_time().as_secs_f32();
        let desired = walking();
        let pull = Vec3::new(0.02, 0.0, -0.03);
        let simulation = CharacterSimulation {
            pull,
            ..Default::default()
        };
        let mut advanced = simulation.clone();
        let spring = advanced.advance(&desired, dt);
        let (mut world, entity, body) = pulled_body(simulation, desired);

        run_once(&mut world, drive_body);

        let velocity = velocity_of(&world, body);
        let expected = spring + pull / (2.0 * dt);
        assert!(velocity.with_y(0.0).distance(expected) < EPS);
        assert!((velocity.y + 3.0).abs() < EPS);
        let simulation = world
            .get_component_for_entity::<CharacterSimulation>(entity)
            .unwrap();
        assert_eq!(simulation.pull, Vec3::ZERO);
        assert_eq!(simulation.velocity, advanced.velocity);
        assert_eq!(simulation.facing, advanced.facing);
    }

    #[test]
    fn drive_body_caps_the_pull_but_not_the_springs() {
        let dt = Time::fixed_delta_time().as_secs_f32();
        let desired = DesiredMotion {
            velocity: Vec3::new(8.0, 0.0, 0.0),
            ..DesiredMotion::default()
        };
        let mut simulation = CharacterSimulation {
            pull: Vec3::new(30.0, 0.0, 40.0),
            max_pull_speed: 3.0,
            ..Default::default()
        };
        simulation.velocity.value = desired.velocity;
        let spring = simulation.clone().advance(&desired, dt);
        let (mut world, _, body) = pulled_body(simulation, desired);

        run_once(&mut world, drive_body);

        let expected = spring + Vec3::new(1.8, 0.0, 2.4);
        assert!(velocity_of(&world, body).with_y(0.0).distance(expected) < EPS);
    }

    #[test]
    fn a_second_step_in_a_frame_gets_no_pull() {
        let dt = Time::fixed_delta_time().as_secs_f32();
        let desired = walking();
        let simulation = CharacterSimulation {
            pull: Vec3::new(0.1, 0.0, 0.0),
            ..Default::default()
        };
        let mut advanced = simulation.clone();
        advanced.advance(&desired, dt);
        let spring = advanced.advance(&desired, dt);
        let (mut world, _, body) = pulled_body(simulation, desired);

        run_once(&mut world, drive_body);
        run_once(&mut world, drive_body);

        assert!(velocity_of(&world, body).with_y(0.0).distance(spring) < EPS);
    }

    #[test]
    fn drive_body_records_the_facing_it_advanced_from() {
        let mut simulation = CharacterSimulation::default();
        simulation.facing.value = Quat::from_rotation_y(0.5);
        let (mut world, entity, _) = pulled_body(simulation, walking());

        run_once(&mut world, drive_body);

        let simulation = world
            .get_component_for_entity::<CharacterSimulation>(entity)
            .unwrap();
        assert_eq!(simulation.previous_facing, Quat::from_rotation_y(0.5));
        assert_ne!(simulation.facing.value, simulation.previous_facing);
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

    fn moving_capsule() -> (World, Entity, CharacterSimulation, DesiredMotion) {
        let mut simulation = CharacterSimulation::default();
        simulation.velocity.value = Vec3::new(1.0, 0.0, 2.0);
        simulation.facing.value = Quat::from_rotation_y(0.5);
        let desired = DesiredMotion {
            velocity: Vec3::new(3.0, 0.0, 4.0),
            rotation: Quat::from_rotation_y(1.0),
        };
        let (world, capsule) =
            world_with_body(Vec3::new(3.0, 5.0, -2.0), simulation.clone(), desired);
        (world, capsule, simulation, desired)
    }

    #[test]
    fn publish_trajectory_climbs_from_a_player_to_the_capsule_its_character_names() {
        let (mut world, capsule, simulation, desired) = moving_capsule();
        let character = world.spawn((DisplacementSimulation(capsule), AnimationPlayer::new(1)));
        let armature = world.spawn(ChildOf::new(character));
        let skeleton = world.spawn((ChildOf::new(armature), AnimationPlayer::new(1)));

        run_once(&mut world, publish_trajectory);

        for player in [character, skeleton] {
            let trajectory = published(&world, player).unwrap();
            assert!((trajectory.translation.x - 3.0).abs() < EPS);
            assert!((trajectory.translation.z + 2.0).abs() < EPS);
            assert_eq!(trajectory.rotation, Quat::from_rotation_y(0.5));
            assert_eq!(
                trajectory.samples,
                simulation.predict(trajectory.translation, &desired)
            );
        }
    }

    #[test]
    fn a_player_with_a_simulation_of_its_own_does_not_climb() {
        let (mut world, capsule, ..) = moving_capsule();
        let character = world.spawn(DisplacementSimulation(capsule));
        let player = world.spawn((
            ChildOf::new(character),
            CharacterSimulation::default(),
            AnimationPlayer::new(1),
        ));

        run_once(&mut world, publish_trajectory);

        assert_eq!(published(&world, player), None);
    }

    #[test]
    fn a_player_with_no_capsule_above_it_gets_no_trajectory() {
        let (mut world, ..) = moving_capsule();
        let plain = world.spawn(());
        let unlinked = world.spawn((ChildOf::new(plain), AnimationPlayer::new(1)));
        let dangling = world.spawn(DisplacementSimulation(plain));
        let under_dangling = world.spawn((ChildOf::new(dangling), AnimationPlayer::new(1)));

        run_once(&mut world, publish_trajectory);

        assert_eq!(published(&world, unlinked), None);
        assert_eq!(published(&world, under_dangling), None);
    }
}
