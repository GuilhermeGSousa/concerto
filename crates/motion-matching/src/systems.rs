use concerto_animation::{player::AnimationPlayer, pose::RootMotion, root::AnimationRootBone};
use concerto_ecs::{
    signal::{lifecycle::Remove, On},
    Entity, Query, Res, ResMut, Without,
};
use concerto_foundation::{
    time::Time,
    transform::{GlobalTransform, Transform},
};
use concerto_mesh::skeleton::SkeletonComponent;
use concerto_physics::{body::BodyId, carried::Carried, physics_state::PhysicsState};

use crate::{
    displacement::{yaw_of, CharacterDisplacement, CharacterLink, Placement},
    node::{trajectory_key, MotionMatchingTrajectory},
    simulation::{CharacterSimulation, DesiredMotion},
};

/// Advances the springs one fixed step towards the desired motion and sets the body's horizontal velocity.
///
/// The velocity is the spring's plus half of any pull, at no more than
/// `max_pull_speed` or the spring's own speed, whichever is higher. A
/// `Carried` body is left alone.
pub fn drive_body(
    mut query: Query<(&mut CharacterSimulation, &DesiredMotion, &BodyId), Without<Carried>>,
    mut physics: ResMut<PhysicsState>,
) {
    let dt = Time::fixed_delta_time().as_secs_f32();
    for (mut simulation, desired, body_id) in query.iter_mut() {
        simulation.previous_facing = simulation.facing.value;
        let velocity = simulation.advance(desired, dt);
        let pull = std::mem::take(&mut simulation.pull) / (2.0 * dt);
        let limit = velocity.length().max(simulation.max_pull_speed);
        let vertical = physics.linear_velocity(*body_id).y;
        let moved = (velocity + pull).clamp_length_max(limit);
        physics.set_linear_velocity(*body_id, moved.with_y(vertical));
    }
}

/// Predicts each linked character's path from where its body ended the step and writes it to the blackboard of the animation player on its skeleton.
pub fn publish_trajectory(
    links: Query<&CharacterLink>,
    mut players: Query<&mut AnimationPlayer>,
    simulations: Query<(&CharacterSimulation, &DesiredMotion, &BodyId)>,
    physics: Res<PhysicsState>,
) {
    for link in links.iter() {
        let (Some(capsule), Some(skeleton)) = (link.capsule.entity(), link.skeleton.entity())
        else {
            continue;
        };
        let Some(mut player) = players.get_entity_mut(skeleton) else {
            continue;
        };
        let Some((simulation, desired, body_id)) = simulations.get_entity(capsule) else {
            continue;
        };
        let translation = physics.body_transform(*body_id).translation;
        player.set_param(
            trajectory_key(),
            MotionMatchingTrajectory {
                translation,
                rotation: simulation.facing.value,
                samples: simulation.predict(translation, desired),
            },
        );
    }
}

/// Restarts a body's springs from the velocity and facing it was carried at, when it is let go.
pub fn restart_released_simulation(
    on: On<Remove<Carried>>,
    carried: Query<&Carried>,
    mut simulations: Query<&mut CharacterSimulation>,
) {
    let entity = on.signal().entity;
    let (Some(carried), Some(mut simulation)) = (
        carried.get_entity(entity),
        simulations.get_entity_mut(entity),
    ) else {
        return;
    };
    simulation.restart(carried.velocity.with_y(0.0), carried.facing);
}

fn in_world_space(delta: RootMotion, space: &GlobalTransform) -> RootMotion {
    let rotation = space.rotation();
    RootMotion {
        translation: rotation * (space.scale() * delta.translation),
        rotation: rotation * delta.rotation * rotation.inverse(),
    }
}

fn root_motion_of(
    skeleton: Entity,
    skeletons: &Query<(&SkeletonComponent, &GlobalTransform)>,
    root_bones: &Query<&AnimationRootBone>,
) -> Option<RootMotion> {
    let (component, space) = skeletons.get_entity(skeleton)?;
    let bone = root_bones.get_entity(component.root()?)?;
    Some(in_world_space(bone.root_motion, space))
}

/// Places each character by the root motion of its skeleton and its [`CharacterDisplacement`].
///
/// A [`Synchronization`](crate::displacement::Synchronization)'s target is
/// given to the capsule. A character whose capsule is `Carried` is put on the
/// capsule, with no root motion.
pub fn apply_character_displacement(
    characters: Query<(Entity, &CharacterDisplacement, &CharacterLink)>,
    skeletons: Query<(&SkeletonComponent, &GlobalTransform)>,
    root_bones: Query<&AnimationRootBone>,
    mut simulations: Query<&mut CharacterSimulation>,
    mut transforms: Query<&mut Transform>,
    carried: Query<&Carried>,
    time: Res<Time>,
) {
    let dt = time.delta().as_secs_f32();
    let alpha = time.fixed_alpha();
    for (entity, method, link) in characters.iter() {
        let Some(capsule) = link.capsule.entity() else {
            continue;
        };
        if let Some(carried) = carried.get_entity(capsule) {
            let position = transforms
                .get_entity_mut(capsule)
                .map(|capsule| capsule.translation);
            if let (Some(position), Some(mut character)) =
                (position, transforms.get_entity_mut(entity))
            {
                character.translation = position;
                character.rotation = carried.facing;
            }
            continue;
        }
        let Some(mut simulation) = simulations.get_entity_mut(capsule) else {
            continue;
        };
        let Some(position) = transforms
            .get_entity_mut(capsule)
            .map(|capsule| capsule.translation)
        else {
            continue;
        };
        let Some(mut character) = transforms.get_entity_mut(entity) else {
            continue;
        };
        let rotation = simulation.interpolated_facing(alpha);
        let displaced = method.displace(
            dt,
            Placement { position, rotation },
            Placement {
                position: character.translation,
                rotation: character.rotation,
            },
            link.skeleton
                .entity()
                .and_then(|skeleton| root_motion_of(skeleton, &skeletons, &root_bones))
                .unwrap_or(RootMotion::IDENTITY),
        );
        character.translation = displaced.character.position;
        character.rotation = displaced.character.rotation;
        simulation.pull = displaced.pull;
        if matches!(method, CharacterDisplacement::Synchronization(_)) {
            let turn = yaw_of(displaced.character.rotation) * yaw_of(rotation).inverse();
            simulation.previous_facing = (turn * simulation.previous_facing).normalize();
            simulation.facing.value = (turn * simulation.facing.value).normalize();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::FRAC_PI_2;
    use std::thread;
    use std::time::Duration;

    use concerto_animation::clip::{AnimationChanelOutput, AnimationChannel, AnimationClip};
    use concerto_animation::evaluation::AnimationGraphContext;
    use concerto_animation::graph::AnimationGraph;
    use concerto_animation::node::{
        AnimationClipNode, AnimationNode, EditorAnimationNode, InputSlots, ReflectAnimationNode,
    };
    use concerto_animation::player::AnimationHandleComponent;
    use concerto_animation::plugin::{AnimationAppExt, AnimationPlugin};
    use concerto_animation::pose::{AnimationFrame, EvaluatedFrame, FramePool};
    use concerto_app::main_schedule::MainSchedulePlugin;
    use concerto_app::plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin};
    use concerto_app::App;
    use concerto_ecs::component::scene::SceneEntityRef;
    use concerto_ecs::entity::hierarchy::ChildOf;
    use concerto_ecs::system::executor::single_thread::SingleThreadedExecutor;
    use concerto_ecs::system::schedule::Schedule;
    use concerto_ecs::world::World;
    use concerto_ecs::{Entity, IntoSystemConfig};
    use concerto_foundation::assets::asset_server::AssetServer;
    use concerto_foundation::assets::asset_store::AssetStore;
    use concerto_foundation::assets::handle::AssetHandle;
    use concerto_foundation::math::spring_damper::halflife_to_damping;
    use concerto_foundation::transform::Transform;
    use concerto_mesh::skeleton::{JointPose, Skeleton, SkeletonComponent};
    use concerto_name::name;
    use concerto_physics::collider::{register_colliders, Collider, ColliderOffset};
    use concerto_physics::physics_pipeline::PhysicsPipeline;
    use concerto_physics::plugin::PhysicsPlugin;
    use concerto_physics::rigid_body::{AllowedDofs, MotionType, RigidBody};
    use concerto_physics::step_simulation;
    use concerto_reflect::Reflect;
    use glam::{Affine3A, Mat4, Quat, Vec3};
    use uuid::Uuid;

    use super::*;
    use crate::displacement::{CharacterDisplacement, CharacterLink, Clamp, Synchronization};
    use crate::plugin::MotionMatchingPlugin;
    use crate::simulation::CharacterSimulation;

    const EPS: f32 = 1.0e-4;

    struct Body {
        world: World,
        entity: Entity,
    }

    fn world_with_body(
        position: Vec3,
        simulation: CharacterSimulation,
        desired: DesiredMotion,
    ) -> Body {
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
        Body { world, entity }
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
            .get_param::<MotionMatchingTrajectory>(trajectory_key())
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
        let Body { mut world, entity } =
            world_with_body(Vec3::new(0.0, 5.0, 0.0), simulation, desired);
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
    fn publish_trajectory_writes_the_capsules_path_to_the_skeletons_player() {
        let mut simulation = CharacterSimulation::default();
        simulation.velocity.value = Vec3::new(1.0, 0.0, 2.0);
        simulation.facing.value = Quat::from_rotation_y(0.5);
        let desired = DesiredMotion {
            velocity: Vec3::new(3.0, 0.0, 4.0),
            rotation: Quat::from_rotation_y(1.0),
        };
        let Body {
            mut world,
            entity: capsule,
        } = world_with_body(Vec3::new(3.0, 5.0, -2.0), simulation.clone(), desired);
        let skeleton = world.spawn(AnimationPlayer::new(1));
        world.spawn(CharacterLink {
            capsule: SceneEntityRef::Entity(capsule),
            skeleton: SceneEntityRef::Entity(skeleton),
        });

        run_once(&mut world, publish_trajectory);

        let trajectory = published(&world, skeleton).unwrap();
        assert!((trajectory.translation.x - 3.0).abs() < EPS);
        assert!((trajectory.translation.z + 2.0).abs() < EPS);
        assert_eq!(trajectory.rotation, Quat::from_rotation_y(0.5));
        assert_eq!(
            trajectory.samples,
            simulation.predict(trajectory.translation, &desired)
        );
    }

    const YAW_EPS: f32 = 1.0e-3;

    fn displacement_world() -> World {
        let mut world = World::new();
        world.register_component::<Transform>();
        world.insert_resource(Time::new());
        world
    }

    fn spawn_capsule(world: &mut World, position: Vec3) -> Entity {
        world.spawn((
            CharacterSimulation::default(),
            Transform::from_translation(position),
        ))
    }

    fn spawn_turning_capsule(world: &mut World, previous_yaw: f32, yaw: f32) -> Entity {
        let mut simulation = CharacterSimulation {
            previous_facing: Quat::from_rotation_y(previous_yaw),
            ..CharacterSimulation::default()
        };
        simulation.facing.value = Quat::from_rotation_y(yaw);
        simulation.facing.velocity = Vec3::new(0.0, 0.7, 0.0);
        world.spawn((simulation, Transform::IDENTITY))
    }

    fn spawn_linked_character(
        world: &mut World,
        capsule: Entity,
        skeleton: Entity,
        method: CharacterDisplacement,
        position: Vec3,
    ) -> Entity {
        world.spawn((
            method,
            CharacterLink {
                capsule: SceneEntityRef::Entity(capsule),
                skeleton: SceneEntityRef::Entity(skeleton),
            },
            Transform::from_translation(position),
        ))
    }

    fn spawn_character(
        world: &mut World,
        capsule: Entity,
        method: CharacterDisplacement,
        position: Vec3,
    ) -> Entity {
        let skeleton = world.spawn(());
        spawn_linked_character(world, capsule, skeleton, method, position)
    }

    fn rooted_skeleton(world: &mut World, root_motion: RootMotion) -> SkeletonComponent {
        let root = world.spawn(AnimationRootBone { root_motion });
        SkeletonComponent::new(
            AssetHandle::default(),
            Vec::new(),
            Some(SceneEntityRef::Entity(root)),
        )
    }

    fn spawn_skeleton(world: &mut World, space: Mat4, root_motion: RootMotion) -> Entity {
        let skeleton = rooted_skeleton(world, root_motion);
        world.spawn((skeleton, GlobalTransform::new(space)))
    }

    fn spawn_moving_character(
        world: &mut World,
        capsule: Entity,
        method: CharacterDisplacement,
        position: Vec3,
        root_motion: RootMotion,
    ) -> Entity {
        let skeleton = spawn_skeleton(world, Mat4::IDENTITY, root_motion);
        spawn_linked_character(world, capsule, skeleton, method, position)
    }

    fn forward_by(distance: f32) -> RootMotion {
        RootMotion {
            translation: Vec3::new(distance, 0.0, 0.0),
            rotation: Quat::IDENTITY,
        }
    }

    fn synchronization(data_factor: f32) -> CharacterDisplacement {
        CharacterDisplacement::Synchronization(Synchronization { data_factor })
    }

    fn clamp(max_distance: f32, max_angle: f32) -> CharacterDisplacement {
        CharacterDisplacement::Clamp(Clamp {
            max_distance,
            max_angle,
        })
    }

    fn position_of(world: &World, entity: Entity) -> Vec3 {
        world
            .get_component_for_entity::<Transform>(entity)
            .unwrap()
            .translation
    }

    fn rotation_of(world: &World, entity: Entity) -> Quat {
        world
            .get_component_for_entity::<Transform>(entity)
            .unwrap()
            .rotation
    }

    fn simulation_of(world: &World, capsule: Entity) -> &CharacterSimulation {
        world
            .get_component_for_entity::<CharacterSimulation>(capsule)
            .unwrap()
    }

    fn assert_near(actual: Vec3, expected: Vec3) {
        assert!(
            actual.distance(expected) < EPS,
            "{actual:?} vs {expected:?}"
        );
    }

    fn assert_yaw(actual: Quat, yaw: f32) {
        let forward = actual * Vec3::Z;
        let expected = Quat::from_rotation_y(yaw) * Vec3::Z;
        assert!(
            forward.distance(expected) < YAW_EPS,
            "{forward:?} vs a yaw of {yaw}"
        );
    }

    #[test]
    fn a_rotated_skeleton_keeps_the_translation_and_the_yaw_of_the_delta() {
        let mut world = displacement_world();
        let capsule = spawn_capsule(&mut world, Vec3::ZERO);
        let z_up = Mat4::from_scale_rotation_translation(
            Vec3::splat(2.0),
            Quat::from_rotation_x(-FRAC_PI_2),
            Vec3::new(5.0, 0.0, 5.0),
        );
        let delta = RootMotion {
            translation: Vec3::new(0.0, 1.0, 0.3),
            rotation: Quat::from_rotation_z(0.4),
        };
        let skeleton = spawn_skeleton(&mut world, z_up, delta);
        let character = spawn_linked_character(
            &mut world,
            capsule,
            skeleton,
            synchronization(1.0),
            Vec3::ZERO,
        );

        run_once(&mut world, apply_character_displacement);

        assert_near(position_of(&world, character), Vec3::new(0.0, 0.0, -2.0));
        assert_yaw(rotation_of(&world, character), 0.4);
    }

    #[test]
    fn a_character_follows_the_simulations_yaw_between_fixed_steps() {
        let mut world = displacement_world();
        let capsule = spawn_turning_capsule(&mut world, 0.0, 0.8);
        let character = spawn_character(&mut world, capsule, clamp(1.0, 0.0), Vec3::ZERO);
        thread::sleep(Duration::from_millis(5));
        let alpha = {
            let time = world.get_resource_mut::<Time>().unwrap();
            time.update();
            time.accumulate_fixed_time();
            while time.expend_fixed_time() {}
            time.fixed_alpha()
        };
        assert!(alpha > 0.0);

        run_once(&mut world, apply_character_displacement);

        assert_yaw(rotation_of(&world, character), 0.8 * alpha);
    }

    #[test]
    fn synchronization_pulls_the_capsule_to_its_target_and_other_methods_do_not() {
        let mut world = displacement_world();
        let synchronized = spawn_capsule(&mut world, Vec3::ZERO);
        spawn_moving_character(
            &mut world,
            synchronized,
            synchronization(1.0),
            Vec3::new(1.0, 0.0, 0.0),
            forward_by(0.5),
        );
        let clamped = spawn_capsule(&mut world, Vec3::ZERO);
        spawn_character(
            &mut world,
            clamped,
            clamp(0.1, 1.0),
            Vec3::new(1.0, 0.0, 0.0),
        );

        run_once(&mut world, apply_character_displacement);

        assert_near(
            simulation_of(&world, synchronized).pull,
            Vec3::new(1.5, 0.0, 0.0),
        );
        assert_eq!(simulation_of(&world, clamped).pull, Vec3::ZERO);
    }

    #[test]
    fn synchronization_at_one_turns_the_simulation_to_the_character_and_keeps_the_spin() {
        let mut world = displacement_world();
        let capsule = spawn_turning_capsule(&mut world, 0.2, 0.6);
        let character = spawn_character(&mut world, capsule, synchronization(1.0), Vec3::ZERO);
        world
            .get_component_for_entity_mut::<Transform>(character)
            .unwrap()
            .rotation = Quat::from_rotation_y(1.0);

        run_once(&mut world, apply_character_displacement);

        let simulation = simulation_of(&world, capsule);
        assert_yaw(simulation.previous_facing, 1.0);
        assert_yaw(simulation.facing.value, 1.4);
        assert_eq!(simulation.facing.velocity, Vec3::new(0.0, 0.7, 0.0));
    }

    #[derive(Default, Reflect)]
    #[reflect(AnimationNode)]
    struct ModelProbe {}

    impl AnimationNode for ModelProbe {
        type State = Option<Affine3A>;
        type CreateInput = ();
        type UpdateInput = ();
        type EvaluateInput = ();
        type DriveInput = ();

        fn create_state(&self, _: &(), _: &AnimationGraphContext<'_>) -> Option<Affine3A> {
            None
        }

        fn update(
            &self,
            _: &mut Option<Affine3A>,
            _: f32,
            _: &(),
            _: &mut AnimationGraphContext<'_>,
        ) {
        }

        fn evaluate(
            &self,
            state: &mut Option<Affine3A>,
            _: &(),
            context: &AnimationGraphContext<'_>,
            _: &[Uuid],
            evaluated_inputs: &[EvaluatedFrame],
            _: &mut FramePool,
            output: &mut AnimationFrame,
        ) {
            if let Some(input) = evaluated_inputs.first() {
                output.copy_from(&input.frame);
            }
            *state = context.model_to_world();
        }

        fn reset(&self, state: &mut Option<Affine3A>) {
            *state = None;
        }
    }

    impl EditorAnimationNode for ModelProbe {
        fn input_slots(&self) -> InputSlots {
            InputSlots::Fixed(vec![name!("pose")])
        }
    }

    fn probed_model_to_world(app: &App, holder: Entity) -> Option<Affine3A> {
        let world = app.main().world();
        let graphs = world.get_resource::<AssetStore<AnimationGraph>>().unwrap();
        let mut probed = None;
        world
            .get_component_for_entity::<AnimationPlayer>(holder)
            .unwrap()
            .for_each_state::<ModelProbe>(graphs, |_, state, _| probed = *state);
        probed
    }

    fn app_with_every_plugin() -> App {
        let mut app = App::new();
        app.register_plugin(MainSchedulePlugin)
            .register_plugin(TimePlugin)
            .register_plugin(AssetManagerPlugin)
            .register_plugin(TransformPlugin)
            .register_plugin(PhysicsPlugin)
            .register_plugin(MotionMatchingPlugin)
            .register_plugin(AnimationPlugin);
        app.register_animation_node::<ModelProbe>();
        app.finish_plugin_build();
        app
    }

    #[test]
    fn a_clips_forward_motion_moves_the_character_its_vertical_motion_does_not_and_its_pose_is_evaluated_where_it_moved_to(
    ) {
        let mut app = app_with_every_plugin();
        let root_id = Uuid::from_u128(21);
        let server = app.get_resource::<AssetServer>().unwrap().clone();
        let mut clip = AnimationClip::default();
        clip.add_channel(
            root_id,
            AnimationChannel::new(
                vec![0.0, 10.0],
                AnimationChanelOutput::Translation(vec![Vec3::ZERO, Vec3::new(0.0, 5.0, -10.0)]),
            ),
        );
        let clip = server.add(clip);
        let mut probed = AnimationGraph::new();
        probed
            .result_node()
            .input(ModelProbe {})
            .add_input(AnimationClipNode::new(clip));
        let graph = server.add(probed);
        let skeleton = server.add(
            Skeleton::new(
                vec![Mat4::IDENTITY],
                vec![None],
                vec!["Root".into()],
                vec![JointPose::IDENTITY],
                vec![root_id],
            )
            .unwrap(),
        );
        app.update();

        let height = 1.5;
        let world = app.main_mut().world_mut();
        let capsule = spawn_capsule(world, Vec3::new(0.0, height, 0.0));
        let holder = world.spawn(Transform::IDENTITY);
        let character =
            spawn_linked_character(world, capsule, holder, synchronization(1.0), Vec3::ZERO);
        world.insert(ChildOf::new(character), holder);
        let root = world.spawn((
            Transform::IDENTITY,
            AnimationRootBone::default(),
            ChildOf::new(holder),
        ));
        world.insert(
            (
                AnimationPlayer::new(1),
                AnimationHandleComponent::new(graph),
                SkeletonComponent::new(
                    skeleton,
                    vec![SceneEntityRef::Entity(root)],
                    Some(SceneEntityRef::Entity(root)),
                ),
            ),
            holder,
        );

        let mut travelled = 0.0;
        for frame in 0..4 {
            thread::sleep(Duration::from_millis(5));
            app.update();
            let world = app.main().world();
            let bone = world.get_component_for_entity::<AnimationRootBone>(root);
            travelled += bone.unwrap().root_motion.translation.z;
            if frame == 0 {
                continue;
            }
            let model = world
                .get_component_for_entity::<GlobalTransform>(holder)
                .unwrap()
                .translation();
            let probed = probed_model_to_world(&app, holder).expect("the probe evaluated");
            assert!(
                Vec3::from(probed.translation).distance(model) < EPS,
                "evaluated at {:?}, placed at {model:?}",
                probed.translation
            );
        }

        let world = app.main().world();
        let moved = position_of(world, character);
        assert!(
            (moved.z - travelled).abs() < EPS,
            "{moved:?} vs {travelled}"
        );
        let placed = position_of(world, character);
        assert!(placed.z < 0.0, "{placed:?}");
        assert_eq!(placed.y, height);
    }

    #[test]
    fn a_skeleton_on_a_root_standing_across_its_rest_moves_and_turns_the_character() {
        let mut app = app_with_every_plugin();
        let root_id = Uuid::from_u128(22);
        let base = Quat::from_rotation_x(-FRAC_PI_2);
        let server = app.get_resource::<AssetServer>().unwrap().clone();
        let mut clip = AnimationClip::default();
        clip.add_channel(
            root_id,
            AnimationChannel::new(
                vec![0.0, 10.0],
                AnimationChanelOutput::Translation(vec![Vec3::ZERO, Vec3::new(0.0, 0.0, -10.0)]),
            ),
        );
        let seconds: Vec<f32> = (0..=10).map(|second| second as f32).collect();
        let rotations = seconds
            .iter()
            .map(|second| Quat::from_rotation_y(*second) * base)
            .collect();
        clip.add_channel(
            root_id,
            AnimationChannel::new(seconds, AnimationChanelOutput::Rotation(rotations)),
        );
        let clip = server.add(clip);
        let graph = server.add(AnimationGraph::from_node(AnimationClipNode::new(clip)));
        let skeleton = server.add(
            Skeleton::new(
                vec![Mat4::IDENTITY],
                vec![None],
                vec!["Root".into()],
                vec![JointPose::IDENTITY],
                vec![root_id],
            )
            .unwrap(),
        );
        app.update();

        let world = app.main_mut().world_mut();
        let capsule = spawn_capsule(world, Vec3::ZERO);
        let root = world.spawn(Transform::IDENTITY);
        let character =
            spawn_linked_character(world, capsule, root, synchronization(1.0), Vec3::ZERO);
        world.insert(
            (
                ChildOf::new(character),
                AnimationRootBone::default(),
                AnimationPlayer::new(1),
                AnimationHandleComponent::new(graph),
                SkeletonComponent::new(
                    skeleton,
                    vec![SceneEntityRef::Entity(root)],
                    Some(SceneEntityRef::Entity(root)),
                ),
            ),
            root,
        );

        app.update();
        app.update();
        let world = app.main().world();
        let start = position_of(world, character);
        let start_facing = rotation_of(world, character);

        let mut elapsed = 0.0;
        for _ in 0..4 {
            thread::sleep(Duration::from_millis(5));
            app.update();
            elapsed += app.get_resource::<Time>().unwrap().delta().as_secs_f32();
        }

        let world = app.main().world();
        let moved = position_of(world, character) - start;
        assert!(
            moved.distance(Vec3::new(0.0, 0.0, -elapsed)) < 0.1 * elapsed,
            "{moved:?} in {elapsed}"
        );
        let turned = rotation_of(world, character) * start_facing.inverse();
        assert!(
            turned.angle_between(Quat::from_rotation_y(elapsed)) < 0.1 * elapsed,
            "{turned:?} in {elapsed}"
        );
    }

    #[test]
    fn a_characters_placement_reaches_its_childrens_globals_in_the_same_frame() {
        let mut app = app_with_every_plugin();
        let world = app.main_mut().world_mut();
        let capsule = spawn_capsule(world, Vec3::ZERO);
        let character = spawn_character(world, capsule, clamp(0.5, 1.0), Vec3::new(3.0, 0.0, 0.0));
        let model = world.spawn((
            Transform::from_translation(Vec3::new(0.0, 1.0, 0.0)),
            ChildOf::new(character),
        ));

        app.update();

        let global = app
            .main()
            .world()
            .get_component_for_entity::<GlobalTransform>(model)
            .unwrap()
            .translation();
        assert_near(global, Vec3::new(0.5, 1.0, 0.0));
    }

    const WALL_X: f32 = 2.0;
    const CAPSULE_RADIUS: f32 = 0.3;

    struct Walled {
        world: World,
        capsule: Entity,
        character: Entity,
    }

    fn walled_world(
        wall_half_width: f32,
        desired: DesiredMotion,
        method: CharacterDisplacement,
        character_at: Vec3,
        root_motion: RootMotion,
    ) -> Walled {
        let mut world = displacement_world();
        world.register_component::<Collider>();
        world.insert_resource(PhysicsState::new());
        world.insert_resource(PhysicsPipeline::new());
        world.spawn((
            Collider::cuboid(50.0, 0.5, 50.0),
            Transform::from_translation(Vec3::new(0.0, -0.5, 0.0)),
        ));
        world.spawn((
            Collider::cuboid(wall_half_width, 3.0, 50.0),
            Transform::from_translation(Vec3::new(WALL_X, 3.0, 0.0)),
        ));
        let collider = Collider::capsule(0.5, CAPSULE_RADIUS);
        let offset = ColliderOffset::bottom_origin(&collider);
        let capsule = world.spawn((
            RigidBody {
                density: 1000.0,
                allowed_dofs: AllowedDofs::TRANSLATION | AllowedDofs::ROTATION_Y,
                motion_type: MotionType::Dynamic,
            },
            CharacterSimulation::default(),
            desired,
            collider,
            offset,
            Transform::IDENTITY,
        ));
        let character =
            spawn_moving_character(&mut world, capsule, method, character_at, root_motion);
        run_once(&mut world, register_colliders);
        Walled {
            world,
            capsule,
            character,
        }
    }

    fn run_fixed_steps(world: &mut World, steps: usize) {
        let mut schedule = Schedule::new();
        schedule.add_system((drive_body, step_simulation, apply_character_displacement).chain());
        let mut schedule = schedule.compile::<SingleThreadedExecutor>(world);
        for _ in 0..steps {
            schedule.run(world);
        }
    }

    #[test]
    fn a_wall_stops_a_walking_capsule_and_a_clamped_character_stays_within_reach_of_it() {
        let max_distance = 0.15;
        let walking_into_it = DesiredMotion {
            velocity: Vec3::new(2.0, 0.0, 0.0),
            rotation: Quat::IDENTITY,
        };
        let Walled {
            mut world,
            capsule,
            character,
        } = walled_world(
            0.5,
            walking_into_it,
            clamp(max_distance, FRAC_PI_2),
            Vec3::ZERO,
            forward_by(0.1),
        );

        run_fixed_steps(&mut world, 150);

        let held = position_of(&world, capsule);
        let face = WALL_X - 0.5 - CAPSULE_RADIUS;
        assert!((held.x - face).abs() < 0.05, "{held:?}");
        let placed = position_of(&world, character);
        assert!(placed.x > held.x, "{placed:?} vs {held:?}");
        assert!(
            placed.x - held.x <= max_distance + EPS,
            "{placed:?} vs {held:?}"
        );
        assert_eq!(placed.y, held.y);
    }

    #[test]
    fn seconds_of_pull_do_not_carry_a_capsule_through_a_wall_thinner_than_an_uncapped_step() {
        let wall_half_width = 0.05;
        let Walled {
            mut world,
            capsule,
            character,
        } = walled_world(
            wall_half_width,
            DesiredMotion::default(),
            synchronization(1.0),
            Vec3::new(12.0, 0.0, 0.0),
            forward_by(0.1),
        );

        run_fixed_steps(&mut world, 150);

        let held = position_of(&world, capsule);
        assert!(held.x < WALL_X - wall_half_width, "{held:?}");
        let uncapped_step = (position_of(&world, character).x - held.x) / 2.0;
        assert!(uncapped_step > 2.0 * wall_half_width, "{uncapped_step}");
    }

    #[test]
    fn a_capsule_keeps_up_with_a_character_sprinting_at_the_springs_speed() {
        let dt = Time::fixed_delta_time().as_secs_f32();
        let sprint = 6.8;
        let sprinting = DesiredMotion {
            velocity: Vec3::new(-sprint, 0.0, 0.0),
            ..DesiredMotion::default()
        };
        let Walled {
            mut world,
            capsule,
            character,
        } = walled_world(
            0.5,
            sprinting,
            synchronization(1.0),
            Vec3::ZERO,
            forward_by(-sprint * dt),
        );

        run_fixed_steps(&mut world, 150);

        let gap = position_of(&world, capsule).distance(position_of(&world, character));
        assert!(gap < 2.0 * sprint * dt, "{gap}");
    }
}
