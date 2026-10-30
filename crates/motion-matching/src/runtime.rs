use std::collections::HashMap;

use concerto_animation::{
    blackboard::AnimationBlackboard,
    clip::AnimationClip,
    graph::AnimationGraph,
    inertialization::{Inertializer, PoseVelocity},
    player::AnimationPlayer,
    pose::Pose,
};
use concerto_ecs::{reflect::AppTypeRegistry, Query, Res};
use concerto_foundation::{
    assets::asset_store::AssetStore, math::spring_damper::angular_velocity, transform::Transform,
};
use concerto_mesh::skeleton::{Skeleton, SkeletonComponent};
use concerto_reflect::TypeRegistry;
use log::error;

use crate::{
    channel::BodyMotion,
    database::{BakeError, MotionDatabase},
    instance::MotionMatchingInstance,
    node::{MotionMatchingNode, MotionMatchingTrajectory, TRAJECTORY_KEY},
    query::FeatureQuery,
    search::{cost_of, excluded_tail, search},
    simulation::TrajectorySample,
};

pub(crate) struct Sources<'a> {
    pub database: &'a MotionDatabase,
    pub skeleton: &'a Skeleton,
    pub player_skeleton: &'a Skeleton,
    pub clips: Vec<&'a AnimationClip>,
    pub registry: &'a TypeRegistry,
}

fn copy_pose(pose: &Pose) -> Pose {
    Pose::from_joints(pose.joints().to_vec())
}

fn copy_velocity(velocity: &PoseVelocity) -> PoseVelocity {
    PoseVelocity {
        linear: velocity.linear.clone(),
        angular: velocity.angular.clone(),
    }
}

fn sample(sources: &Sources, frame: usize) -> Result<(Pose, PoseVelocity), BakeError> {
    let database = sources.database;
    let range = database.range_of(frame).ok_or(BakeError::FrameOutOfRange {
        frame,
        frame_count: database.frame_count(),
    })?;
    let (first, second) = if frame + 1 < range.end {
        (frame, frame + 1)
    } else {
        (frame - 1, frame)
    };
    let first_pose = Pose::from_joints(database.pose(first, sources.skeleton, &sources.clips)?);
    let second_pose = Pose::from_joints(database.pose(second, sources.skeleton, &sources.clips)?);
    let velocity = PoseVelocity::between(&first_pose, &second_pose, 1.0 / database.rate());
    let pose = if first == frame {
        first_pose
    } else {
        second_pose
    };
    Ok((pose, velocity))
}

fn body_motion_at(database: &MotionDatabase, frame: usize) -> BodyMotion {
    let Some(range) = database.range_of(frame) else {
        return BodyMotion::default();
    };
    let previous = if frame > range.start {
        frame - 1
    } else {
        frame
    };
    let next = if frame + 1 < range.end {
        frame + 1
    } else {
        frame
    };
    if previous == next {
        return BodyMotion::default();
    }
    let dt = (next - previous) as f32 / database.rate();
    let previous = database.simulation(previous);
    let next = database.simulation(next);
    BodyMotion {
        velocity: (next.translation - previous.translation) / dt,
        angular_velocity: angular_velocity(next.rotation, previous.rotation, dt),
    }
}

fn scatter(instance: &mut MotionMatchingInstance, sources: &Sources) {
    for (bone, id) in sources.skeleton.bone_ids.iter().enumerate() {
        let Some(index) = instance.bone_indices.get(id).copied() else {
            continue;
        };
        if let (Some(joint), Some(source)) = (
            instance.output.get_joint_pose_mut(index),
            instance.blended.joints().get(bone),
        ) {
            *joint = source.clone();
        }
    }
}

fn initialise(instance: &mut MotionMatchingInstance, sources: &Sources) -> Result<(), String> {
    let database = sources.database;
    let bone_count = database.bone_count();
    if sources.skeleton.bone_count() != bone_count {
        return Err(BakeError::BoneCountMismatch {
            expected: bone_count,
            actual: sources.skeleton.bone_count(),
        }
        .to_string());
    }
    let mut bone_indices = HashMap::new();
    for id in sources.skeleton.bone_ids.iter() {
        if let Some(index) = sources
            .player_skeleton
            .bone_ids
            .iter()
            .position(|player_id| player_id == id)
        {
            bone_indices.insert(*id, index);
        }
    }
    if bone_indices.is_empty() {
        return Err("no bone of the database skeleton is in the player's skeleton".into());
    }
    let query = FeatureQuery::new(database, sources.registry).map_err(|error| error.to_string())?;
    let frame = database
        .ranges()
        .next()
        .map(|range| range.start)
        .ok_or_else(|| "the database has no frames".to_string())?;
    let (pose, velocity) = sample(sources, frame).map_err(|error| error.to_string())?;

    instance.output = Pose::from_joints(sources.player_skeleton.rest_pose.to_vec());
    instance.inertializer = Some(Inertializer::new(bone_count));
    instance.blended = copy_pose(&pose);
    instance.previous_blended = copy_pose(&pose);
    instance.pose = pose;
    instance.velocity = velocity;
    instance.frame = frame;
    instance.sub_frame_time = 0.0;
    instance.search_timer = 0.0;
    instance.forced_search_timer = 0.0;
    instance.last_delta = 0.0;
    instance.velocity_change = 0.0;
    instance.rotation_change = 0.0;
    instance.query = Some(query);
    instance.bone_indices = bone_indices;
    instance.root_motion = body_motion_at(database, frame);
    scatter(instance, sources);
    instance.initialised = true;
    Ok(())
}

fn forced_search(
    node: &MotionMatchingNode,
    instance: &mut MotionMatchingInstance,
    first: &TrajectorySample,
    dt: f32,
) -> bool {
    let (velocity_change, rotation_change) = if dt > 0.0 {
        (
            (first.velocity - instance.desired_velocity).length() / dt,
            angular_velocity(first.rotation, instance.desired_rotation, dt).length(),
        )
    } else {
        (instance.velocity_change, instance.rotation_change)
    };
    let fired = instance.forced_search_timer <= 0.0
        && ((instance.velocity_change >= node.velocity_change_threshold
            && velocity_change < node.velocity_change_threshold)
            || (instance.rotation_change >= node.rotation_change_threshold
                && rotation_change < node.rotation_change_threshold));
    if fired {
        instance.forced_search_timer = node.search_period;
    }
    instance.velocity_change = velocity_change;
    instance.rotation_change = rotation_change;
    instance.desired_velocity = first.velocity;
    instance.desired_rotation = first.rotation;
    fired
}

fn find_better_frame(
    node: &MotionMatchingNode,
    instance: &MotionMatchingInstance,
    trajectory: &MotionMatchingTrajectory,
    sources: &Sources,
) -> Result<Option<usize>, String> {
    let database = sources.database;
    let Some(query) = instance.query.as_ref() else {
        return Ok(None);
    };
    let simulation =
        Transform::from_translation_rotation(trajectory.translation, trajectory.rotation);
    let features = match query.build(
        database,
        sources.skeleton,
        instance.frame,
        &simulation,
        &trajectory.samples,
    ) {
        Ok(features) => features,
        Err(BakeError::TrajectoryTooShort { .. }) => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let Some((best, best_cost)) =
        search(database, &features, instance.frame, excluded_tail(database))
    else {
        return Ok(None);
    };
    let Some(current_cost) = cost_of(database, &features, instance.frame) else {
        return Ok(None);
    };
    let differs = best.abs_diff(instance.frame) > node.same_frame_tolerance;
    Ok((best_cost < current_cost && differs).then_some(best))
}

fn run(
    node: &MotionMatchingNode,
    instance: &mut MotionMatchingInstance,
    blackboard: &AnimationBlackboard,
    sources: &Sources,
) -> Result<(), String> {
    if !instance.initialised {
        initialise(instance, sources)?;
    }
    let database = sources.database;
    let dt = instance.delta_time;
    let rate = database.rate();
    let range = database
        .range_of(instance.frame)
        .ok_or_else(|| "the instance's frame left the database".to_string())?;

    instance.search_timer -= dt;
    instance.forced_search_timer -= dt;
    instance.sub_frame_time += dt;
    let whole = (instance.sub_frame_time * rate).floor().max(0.0) as usize;
    instance.sub_frame_time -= whole as f32 / rate;
    let previous_frame = instance.frame;
    instance.frame = (instance.frame + whole).min(range.end - 1);
    let moved = instance.frame != previous_frame;

    let trajectory = blackboard.get::<MotionMatchingTrajectory>(TRAJECTORY_KEY);
    let forced = trajectory
        .and_then(|trajectory| trajectory.samples.first())
        .is_some_and(|first| forced_search(node, instance, first, dt));
    let timer_due = instance.search_timer <= 0.0;
    if timer_due {
        instance.search_timer = node.search_period;
    }
    let in_tail = instance.frame + excluded_tail(database) >= range.end;

    let mut better = None;
    if timer_due || forced || in_tail {
        if let Some(trajectory) = trajectory {
            better = find_better_frame(node, instance, trajectory, sources)?;
        }
    }

    if let Some(best) = better {
        let (pose, velocity) = sample(sources, best).map_err(|error| error.to_string())?;
        let source_velocity = if instance.last_delta > 0.0 {
            PoseVelocity::between(
                &instance.previous_blended,
                &instance.blended,
                instance.last_delta,
            )
        } else {
            copy_velocity(&instance.velocity)
        };
        if let Some(inertializer) = instance.inertializer.as_mut() {
            inertializer.transition(&instance.blended, &source_velocity, &pose, &velocity);
        }
        instance.frame = best;
        instance.pose = pose;
        instance.velocity = velocity;
    } else if moved {
        let (pose, velocity) =
            sample(sources, instance.frame).map_err(|error| error.to_string())?;
        instance.pose = pose;
        instance.velocity = velocity;
    }

    instance.previous_blended.copy_from(&instance.blended);
    if let Some(inertializer) = instance.inertializer.as_mut() {
        inertializer.update(
            &instance.pose,
            &instance.velocity,
            node.blend_halflife,
            dt,
            &mut instance.blended,
        );
    }
    instance.last_delta = dt;
    instance.root_motion = body_motion_at(database, instance.frame);
    scatter(instance, sources);
    Ok(())
}

pub(crate) fn step(
    node: &MotionMatchingNode,
    instance: &mut MotionMatchingInstance,
    blackboard: &AnimationBlackboard,
    sources: &Sources,
) {
    instance.active = false;
    if instance.failed {
        return;
    }
    if let Err(reason) = run(node, instance, blackboard, sources) {
        error!("motion matching stopped: {reason}");
        instance.failed = true;
        instance.initialised = false;
    }
}

/// Advances every active motion matching instance, searches the database and writes the instance's output pose for the node to copy.
pub fn run_motion_matching(
    players: Query<(&mut AnimationPlayer, &SkeletonComponent)>,
    graphs: Res<AssetStore<AnimationGraph>>,
    databases: Res<AssetStore<MotionDatabase>>,
    clips: Res<AssetStore<AnimationClip>>,
    skeletons: Res<AssetStore<Skeleton>>,
    registry: Res<AppTypeRegistry>,
) {
    let registry = registry.read();
    for (mut player, component) in players.iter() {
        if player.is_paused() {
            continue;
        }
        let Some(player_skeleton) = skeletons.get(component.skeleton()) else {
            continue;
        };
        player.for_each_instance_mut::<MotionMatchingNode>(
            &graphs,
            |node, instance, blackboard| {
                if !instance.active {
                    return;
                }
                let Some(database) = databases.get(&node.database) else {
                    return;
                };
                let Some(skeleton) = skeletons.get(database.skeleton()) else {
                    return;
                };
                let mut resolved = Vec::with_capacity(database.clips().len());
                for handle in database.clips() {
                    match clips.get(handle) {
                        Some(clip) => resolved.push(clip),
                        None => return,
                    }
                }
                let sources = Sources {
                    database,
                    skeleton,
                    player_skeleton,
                    clips: resolved,
                    registry: &registry,
                };
                step(node, instance, blackboard, &sources);
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use concerto_animation::{
        blackboard::AnimationBlackboard, clip::AnimationClip, plugin::register_animation_types,
        pose::Pose,
    };
    use concerto_app::{plugins::AssetManagerPlugin, schedule_groups::LateUpdate, App};
    use concerto_ecs::{
        component::scene::SceneEntityRef,
        system::{executor::single_thread::SingleThreadedExecutor, schedule::Schedules},
    };
    use concerto_foundation::{
        assets::{asset_store::AssetStore, handle::AssetHandle, AssetId},
        time::Time,
        transform::Transform,
    };
    use concerto_mesh::skeleton::{Skeleton, SkeletonComponent};
    use concerto_reflect::TypeRegistry;
    use glam::{Quat, Vec3};
    use uuid::Uuid;

    use super::{step, Sources};
    use crate::{
        database::MotionDatabase,
        instance::MotionMatchingInstance,
        node::{MotionMatchingNode, MotionMatchingTrajectory, TRAJECTORY_KEY},
        plugin::register_runtime,
        simulation::TrajectorySample,
        test_support::{pace_database, registry, skeleton, turn_left, walk_forward, Pace},
    };

    struct Fixture {
        database: MotionDatabase,
        skeleton: Skeleton,
        walk: AnimationClip,
        turn: AnimationClip,
        registry: TypeRegistry,
    }

    fn fixture(speed: f32) -> Fixture {
        Fixture {
            database: pace_database(speed, Default::default(), Default::default()),
            skeleton: skeleton(),
            walk: walk_forward(),
            turn: turn_left(),
            registry: registry(),
        }
    }

    impl Fixture {
        fn sources(&self) -> Sources<'_> {
            Sources {
                database: &self.database,
                skeleton: &self.skeleton,
                player_skeleton: &self.skeleton,
                clips: vec![&self.walk, &self.turn],
                registry: &self.registry,
            }
        }
    }

    fn trajectory(pace: f32, samples: usize) -> AnimationBlackboard {
        let mut board = AnimationBlackboard::default();
        board.set(
            TRAJECTORY_KEY,
            MotionMatchingTrajectory {
                translation: Vec3::ZERO,
                rotation: Quat::IDENTITY,
                samples: vec![
                    TrajectorySample {
                        velocity: Vec3::new(0.0, 0.0, -pace),
                        ..Default::default()
                    };
                    samples
                ],
            },
        );
        board
    }

    fn instance(node: &MotionMatchingNode, dt: f32) -> MotionMatchingInstance {
        let mut instance = MotionMatchingInstance::new(node.search_period);
        instance.activate(dt);
        instance
    }

    fn quiet() -> AnimationBlackboard {
        AnimationBlackboard::default()
    }

    fn joints(pose: &Pose) -> Vec<(Vec3, Quat)> {
        pose.joints()
            .iter()
            .map(|joint| (joint.translation, joint.rotation))
            .collect()
    }

    #[test]
    fn the_first_step_initialises_the_instance_and_outputs_the_first_frame() {
        let fixture = fixture(1.0);
        let node = MotionMatchingNode::default();
        let mut instance = instance(&node, 0.0);
        assert!(!instance.initialised);

        step(&node, &mut instance, &quiet(), &fixture.sources());

        assert!(instance.initialised);
        assert!(!instance.failed);
        assert!(!instance.active);
        assert_eq!(instance.frame, 0);
        let expected = fixture
            .database
            .pose(0, &fixture.skeleton, &[&fixture.walk, &fixture.turn])
            .unwrap();
        assert_eq!(instance.output.len(), 3);
        for (joint, expected) in instance.output.joints().iter().zip(&expected) {
            assert!(joint.translation.distance(expected.translation) < 1e-5);
        }
    }

    #[test]
    fn a_new_instance_after_a_graph_change_starts_uninitialised_and_initialises_again() {
        let fixture = fixture(1.0);
        let node = MotionMatchingNode::default();
        let mut first = instance(&node, 0.1);
        step(&node, &mut first, &quiet(), &fixture.sources());
        assert!(first.initialised);

        let mut second = instance(&node, 0.1);
        assert!(!second.initialised);
        step(&node, &mut second, &quiet(), &fixture.sources());

        assert!(second.initialised);
        assert_eq!(second.frame, first.frame);
    }

    #[test]
    fn playback_at_speed_two_advances_two_clip_seconds_per_second() {
        let fixture = fixture(2.0);
        let node = MotionMatchingNode::default();
        let mut instance = instance(&node, 0.0);
        step(&node, &mut instance, &quiet(), &fixture.sources());
        let (_, start) = fixture.database.clip_time(instance.frame).unwrap();

        instance.activate(0.5);
        step(&node, &mut instance, &quiet(), &fixture.sources());

        let (_, end) = fixture.database.clip_time(instance.frame).unwrap();
        assert!((end - start - 1.0).abs() < 1e-3, "{start} -> {end}");
    }

    #[test]
    fn a_trajectory_shorter_than_needed_skips_the_search_and_keeps_playing() {
        let fixture = fixture(1.0);
        let node = MotionMatchingNode::default();
        let mut instance = instance(&node, 0.0);
        step(&node, &mut instance, &quiet(), &fixture.sources());

        instance.activate(0.1);
        step(
            &node,
            &mut instance,
            &trajectory(0.0, 1),
            &fixture.sources(),
        );

        assert!(!instance.failed);
        assert!(instance.initialised);
        assert_eq!(instance.frame, 3);
        assert_eq!(fixture.database.entry_of(instance.frame), Some(0));
    }

    #[test]
    fn a_missing_trajectory_keeps_playing_without_a_search() {
        let fixture = fixture(1.0);
        let node = MotionMatchingNode::default();
        let mut instance = instance(&node, 0.0);
        step(&node, &mut instance, &quiet(), &fixture.sources());

        instance.activate(0.1);
        step(&node, &mut instance, &quiet(), &fixture.sources());

        assert!(!instance.failed);
        assert_eq!(instance.frame, 3);
    }

    #[test]
    fn a_standing_trajectory_moves_the_playhead_into_the_turn_clip() {
        let fixture = fixture(1.0);
        let node = MotionMatchingNode::default();
        let mut instance = instance(&node, 0.0);
        step(&node, &mut instance, &quiet(), &fixture.sources());

        instance.activate(0.1);
        instance.search_timer = 0.0;
        step(
            &node,
            &mut instance,
            &trajectory(0.0, 2),
            &fixture.sources(),
        );

        assert_eq!(fixture.database.entry_of(instance.frame), Some(1));
        assert!((instance.search_timer - node.search_period).abs() < 1e-6);
    }

    #[test]
    fn no_search_runs_before_the_timer_is_due() {
        let fixture = fixture(1.0);
        let node = MotionMatchingNode::default();
        let mut instance = instance(&node, 0.0);
        step(&node, &mut instance, &quiet(), &fixture.sources());

        instance.activate(0.03);
        instance.search_timer = 10.0;
        step(
            &node,
            &mut instance,
            &trajectory(0.0, 2),
            &fixture.sources(),
        );

        assert_eq!(fixture.database.entry_of(instance.frame), Some(0));
    }

    #[test]
    fn the_excluded_tail_forces_a_search_before_the_timer_is_due() {
        let fixture = fixture(1.0);
        let node = MotionMatchingNode::default();
        let mut instance = instance(&node, 0.0);
        step(&node, &mut instance, &quiet(), &fixture.sources());
        instance.frame = 56;
        instance.search_timer = 10.0;

        instance.activate(0.0);
        step(
            &node,
            &mut instance,
            &trajectory(0.0, 2),
            &fixture.sources(),
        );

        assert_eq!(fixture.database.entry_of(instance.frame), Some(1));
    }

    #[test]
    fn a_velocity_change_that_settles_forces_one_search() {
        let fixture = fixture(1.0);
        let node = MotionMatchingNode {
            search_period: 100.0,
            velocity_change_threshold: 1.0,
            ..Default::default()
        };
        let mut instance = instance(&node, 0.0);
        step(&node, &mut instance, &quiet(), &fixture.sources());
        instance.search_timer = 100.0;

        instance.activate(0.1);
        step(
            &node,
            &mut instance,
            &trajectory(0.0, 2),
            &fixture.sources(),
        );
        assert!(instance.forced_search_timer <= 0.0);

        instance.activate(0.1);
        step(
            &node,
            &mut instance,
            &trajectory(5.0, 2),
            &fixture.sources(),
        );
        assert!(instance.forced_search_timer <= 0.0);

        instance.activate(0.1);
        step(
            &node,
            &mut instance,
            &trajectory(5.0, 2),
            &fixture.sources(),
        );
        assert_eq!(instance.forced_search_timer, 100.0);
    }

    #[test]
    fn the_output_pose_on_a_transition_frame_equals_the_pose_before_it() {
        let fixture = fixture(1.0);
        let node = MotionMatchingNode::default();
        let mut instance = instance(&node, 0.0);
        step(&node, &mut instance, &quiet(), &fixture.sources());
        instance.activate(0.1);
        step(&node, &mut instance, &quiet(), &fixture.sources());
        let before = joints(&instance.blended);
        let frame_before = instance.frame;

        instance.activate(0.0);
        instance.search_timer = 0.0;
        step(
            &node,
            &mut instance,
            &trajectory(0.0, 2),
            &fixture.sources(),
        );

        assert_ne!(fixture.database.entry_of(instance.frame), Some(0));
        assert_ne!(instance.frame, frame_before);
        for ((translation, rotation), (before_translation, before_rotation)) in
            joints(&instance.blended).into_iter().zip(before)
        {
            assert!(translation.distance(before_translation) < 1e-4);
            assert!(rotation.dot(before_rotation).abs() > 0.9999);
        }
    }

    #[test]
    fn the_player_skeleton_is_matched_to_the_database_by_bone_uuid() {
        let fixture = fixture(1.0);
        let reordered = Skeleton::new(
            vec![glam::Mat4::IDENTITY; 3],
            vec![None, Some(0), Some(1)],
            vec!["Spine2".into(), "Root".into(), "Hips".into()],
            vec![
                concerto_mesh::JointPose::IDENTITY,
                concerto_mesh::JointPose::IDENTITY,
                concerto_mesh::JointPose::IDENTITY,
            ],
            vec![Uuid::from_u128(3), Uuid::from_u128(1), Uuid::from_u128(2)],
        )
        .unwrap();
        let node = MotionMatchingNode::default();
        let mut instance = instance(&node, 0.0);
        let sources = Sources {
            player_skeleton: &reordered,
            ..fixture.sources()
        };

        step(&node, &mut instance, &quiet(), &sources);

        assert_eq!(instance.bone_indices[&Uuid::from_u128(1)], 1);
        assert_eq!(instance.bone_indices[&Uuid::from_u128(2)], 2);
        assert_eq!(instance.bone_indices[&Uuid::from_u128(3)], 0);
        let expected = fixture
            .database
            .pose(0, &fixture.skeleton, &[&fixture.walk, &fixture.turn])
            .unwrap();
        assert!(
            instance.output.joints()[2]
                .translation
                .distance(expected[1].translation)
                < 1e-5
        );
    }

    #[test]
    fn a_player_skeleton_with_none_of_the_database_bones_stops_the_instance() {
        let fixture = fixture(1.0);
        let stranger = Skeleton::new(
            vec![glam::Mat4::IDENTITY],
            vec![None],
            vec!["Other".into()],
            vec![concerto_mesh::JointPose::IDENTITY],
            vec![Uuid::from_u128(99)],
        )
        .unwrap();
        let node = MotionMatchingNode::default();
        let mut instance = instance(&node, 0.0);
        let sources = Sources {
            player_skeleton: &stranger,
            ..fixture.sources()
        };

        step(&node, &mut instance, &quiet(), &sources);

        assert!(instance.failed);
        assert!(!instance.initialised);
        let mut output = Pose::identity(1);
        assert!(!instance.write_output(&mut output));
    }

    #[test]
    fn a_failed_instance_stays_stopped() {
        let fixture = fixture(1.0);
        let node = MotionMatchingNode::default();
        let mut instance = instance(&node, 0.0);
        instance.failed = true;

        step(&node, &mut instance, &quiet(), &fixture.sources());

        assert!(!instance.initialised);
    }

    #[test]
    fn root_motion_is_the_simulation_bones_velocity_at_the_frame() {
        let fixture = fixture(1.0);
        let node = MotionMatchingNode::default();
        let mut instance = instance(&node, 0.0);
        instance.activate(0.5);

        step(&node, &mut instance, &quiet(), &fixture.sources());

        let motion = instance.root_motion();
        assert!(
            (motion.velocity.length() - 1.0).abs() < 0.3,
            "{:?}",
            motion.velocity
        );
    }

    #[test]
    fn run_motion_matching_then_animate_targets_poses_the_bones_from_the_database() {
        let mut app = App::new();
        app.register_plugin(AssetManagerPlugin);
        app.insert_resource(Time::new());
        app.register_plugin(concerto_animation::plugin::AnimationPlugin);
        app.register_asset::<MotionDatabase>();
        app.register_type::<Pace>();
        app.register_type::<MotionMatchingNode>();
        register_runtime(&mut app);

        let world = app.main_mut().world_mut();
        let skeleton_handle = AssetHandle::<Skeleton>::weak(AssetId::new());
        let walk_handle = AssetHandle::<AnimationClip>::weak(AssetId::new());
        let turn_handle = AssetHandle::<AnimationClip>::weak(AssetId::new());
        let database = pace_database(
            1.0,
            skeleton_handle.clone(),
            [walk_handle.clone(), turn_handle.clone()],
        );
        let database_handle = AssetHandle::<MotionDatabase>::weak(AssetId::new());
        world
            .get_resource_mut::<AssetStore<Skeleton>>()
            .unwrap()
            .insert(skeleton_handle.id(), skeleton());
        {
            let clips = world
                .get_resource_mut::<AssetStore<AnimationClip>>()
                .unwrap();
            clips.insert(walk_handle.id(), walk_forward());
            clips.insert(turn_handle.id(), turn_left());
        }
        world
            .get_resource_mut::<AssetStore<MotionDatabase>>()
            .unwrap()
            .insert(database_handle.id(), database.clone());

        let mut type_registry = TypeRegistry::default();
        register_animation_types(&mut type_registry);
        type_registry.register::<MotionMatchingNode>();
        type_registry.register::<AssetHandle<MotionDatabase>>();
        let mut graph = concerto_animation::graph::AnimationGraph::from_node(MotionMatchingNode {
            database: database_handle,
            ..Default::default()
        });
        graph.make_live(&type_registry, None).unwrap();
        let graph_handle =
            AssetHandle::<concerto_animation::graph::AnimationGraph>::weak(AssetId::new());
        world
            .get_resource_mut::<AssetStore<concerto_animation::graph::AnimationGraph>>()
            .unwrap()
            .insert(graph_handle.id(), graph);

        let player = concerto_animation::player::AnimationPlayer::new(3);
        let entities: Vec<_> = (0..3).map(|_| world.spawn(Transform::IDENTITY)).collect();
        let hips = entities[1];
        let bones = entities
            .iter()
            .map(|entity| SceneEntityRef::Entity(*entity))
            .collect();
        world.spawn((
            player,
            concerto_animation::player::AnimationHandleComponent::new(graph_handle),
            SkeletonComponent {
                skeleton: skeleton_handle,
                bones,
                root: None,
            },
        ));

        let schedules = world.remove_resource::<Schedules>().unwrap();
        let compiled = schedules.compile::<SingleThreadedExecutor>(world);
        world.insert_resource(compiled);
        world.run_schedule(LateUpdate);
        world.run_schedule(LateUpdate);

        let expected = database
            .pose(0, &skeleton(), &[&walk_forward(), &turn_left()])
            .unwrap();
        let hips_transform = world.get_component_for_entity::<Transform>(hips).unwrap();
        assert!(expected[1].translation.distance(Vec3::ZERO) > 0.5);
        assert!(
            hips_transform.translation.distance(expected[1].translation) < 1e-4,
            "{:?} vs {:?}",
            hips_transform.translation,
            expected[1].translation
        );
    }
}
