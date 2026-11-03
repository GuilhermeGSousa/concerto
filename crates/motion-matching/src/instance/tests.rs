use std::f32::consts::FRAC_PI_2;

use concerto_animation::{
    blackboard::AnimationBlackboard,
    clip::{AnimationChanelOutput, AnimationChannel, AnimationClip},
    node::AnimationNode,
    pose::{Pose, RootMotion},
};
use concerto_foundation::transform::Transform;
use concerto_mesh::{skeleton::Skeleton, JointPose};
use glam::{Quat, Vec3};
use uuid::Uuid;

use super::{MotionMatchingInstance, Sources};
use crate::{
    database::MotionDatabase,
    node::{MotionMatchingNode, MotionMatchingTrajectory, TRAJECTORY_KEY},
    query::FeatureQuery,
    search::search,
    simulation::TrajectorySample,
    test_support::{pace_database, pace_database_of, skeleton, turn_left, walk_forward, TURN_RATE},
};

struct Fixture {
    database: MotionDatabase,
    skeleton: Skeleton,
    walk: AnimationClip,
    turn: AnimationClip,
}

fn fixture(speed: f32) -> Fixture {
    Fixture {
        database: pace_database(speed, Default::default(), Default::default()),
        skeleton: skeleton(),
        walk: walk_forward(),
        turn: turn_left(),
    }
}

fn fixture_of(rest: Quat, walk: AnimationClip) -> Fixture {
    let mut skeleton = skeleton();
    skeleton.rest_pose[0].rotation = rest;
    let turn = turn_left();
    Fixture {
        database: pace_database_of(
            1.0,
            Default::default(),
            Default::default(),
            &skeleton,
            &[&walk, &turn],
        ),
        skeleton,
        walk,
        turn,
    }
}

impl Fixture {
    fn sources(&self) -> Sources<'_> {
        Sources {
            database: &self.database,
            skeleton: &self.skeleton,
            player_skeleton: &self.skeleton,
            clips: vec![&self.walk, &self.turn],
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
    instance.delta_time = dt;
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

    instance.step(&node, instance.delta_time, &quiet(), fixture.sources());

    assert!(instance.initialised);
    assert!(!instance.failed);
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
    first.step(&node, first.delta_time, &quiet(), fixture.sources());
    assert!(first.initialised);

    let mut second = instance(&node, 0.1);
    assert!(!second.initialised);
    second.step(&node, second.delta_time, &quiet(), fixture.sources());

    assert!(second.initialised);
    assert_eq!(second.frame, first.frame);
}

#[test]
fn playback_at_speed_two_advances_two_clip_seconds_per_second() {
    let fixture = fixture(2.0);
    let node = MotionMatchingNode::default();
    let mut instance = instance(&node, 0.0);
    instance.step(&node, instance.delta_time, &quiet(), fixture.sources());
    let (_, start) = fixture.database.clip_time(instance.frame).unwrap();

    instance.delta_time = 0.5;
    instance.step(&node, instance.delta_time, &quiet(), fixture.sources());

    let (_, end) = fixture.database.clip_time(instance.frame).unwrap();
    assert!((end - start - 1.0).abs() < 1e-3, "{start} -> {end}");
}

#[test]
fn a_trajectory_shorter_than_needed_skips_the_search_and_keeps_playing() {
    let fixture = fixture(1.0);
    let node = MotionMatchingNode::default();
    let mut instance = instance(&node, 0.0);
    instance.step(&node, instance.delta_time, &quiet(), fixture.sources());

    instance.delta_time = 0.1;
    instance.step(
        &node,
        instance.delta_time,
        &trajectory(0.0, 1),
        fixture.sources(),
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
    instance.step(&node, instance.delta_time, &quiet(), fixture.sources());

    instance.delta_time = 0.1;
    instance.step(&node, instance.delta_time, &quiet(), fixture.sources());

    assert!(!instance.failed);
    assert_eq!(instance.frame, 3);
}

#[test]
fn a_standing_trajectory_moves_the_playhead_into_the_turn_clip() {
    let fixture = fixture(1.0);
    let node = MotionMatchingNode::default();
    let mut instance = instance(&node, 0.0);
    instance.step(&node, instance.delta_time, &quiet(), fixture.sources());

    instance.delta_time = 0.1;
    instance.search_timer = 0.0;
    instance.step(
        &node,
        instance.delta_time,
        &trajectory(0.0, 2),
        fixture.sources(),
    );

    assert_eq!(fixture.database.entry_of(instance.frame), Some(1));
    assert!((instance.search_timer - node.search_period).abs() < 1e-6);
}

#[test]
fn a_transition_lands_on_the_frame_the_normalised_query_finds() {
    let fixture = fixture(1.0);
    let node = MotionMatchingNode::default();
    let mut instance = instance(&node, 0.0);
    instance.step(&node, instance.delta_time, &quiet(), fixture.sources());

    instance.delta_time = 0.1;
    instance.search_timer = 0.0;
    let board = trajectory(0.25, 2);
    instance.step(&node, instance.delta_time, &board, fixture.sources());

    let samples = &board
        .get::<MotionMatchingTrajectory>(TRAJECTORY_KEY)
        .unwrap()
        .samples;
    let query = FeatureQuery::new(&fixture.database)
        .unwrap()
        .build(
            &fixture.database,
            &fixture.skeleton,
            3,
            &Transform::IDENTITY,
            samples,
        )
        .unwrap();
    let (expected, _) = search(
        &fixture.database,
        &query,
        Some(3),
        node.ignore_surrounding,
        node.ignore_range_end,
    )
    .unwrap();
    assert_ne!(expected, 3);
    assert_eq!(instance.frame, expected);
}

#[test]
fn a_search_skips_the_frames_the_node_ignores_around_the_current_one() {
    let fixture = fixture(1.0);
    let node = MotionMatchingNode {
        ignore_surrounding: 1000,
        ..Default::default()
    };

    let instance = searched(&fixture, &node);

    assert_eq!(instance.frame, 3);
}

#[test]
fn a_search_skips_the_range_ends_the_node_ignores() {
    let fixture = fixture(1.0);
    let node = MotionMatchingNode {
        ignore_range_end: 1000,
        ..Default::default()
    };

    let instance = searched(&fixture, &node);

    assert_eq!(instance.frame, 3);
}

fn searched(fixture: &Fixture, node: &MotionMatchingNode) -> MotionMatchingInstance {
    let mut instance = instance(node, 0.0);
    instance.step(node, instance.delta_time, &quiet(), fixture.sources());
    instance.delta_time = 0.1;
    instance.search_timer = 0.0;
    instance.step(
        node,
        instance.delta_time,
        &trajectory(0.25, 2),
        fixture.sources(),
    );
    instance
}

#[test]
fn an_instance_that_has_not_searched_has_no_last_query() {
    let fixture = fixture(1.0);
    let node = MotionMatchingNode::default();
    let mut instance = instance(&node, 0.1);

    instance.step(&node, instance.delta_time, &quiet(), fixture.sources());

    assert!(instance.initialised);
    assert!(instance.last_query.is_none());
}

#[test]
fn a_search_keeps_its_raw_query_with_what_it_was_built_from() {
    let fixture = fixture(1.0);
    let node = MotionMatchingNode::default();
    let mut instance = instance(&node, 0.0);
    instance.step(&node, instance.delta_time, &quiet(), fixture.sources());
    let published = MotionMatchingTrajectory {
        translation: Vec3::new(1.0, 0.0, 2.0),
        rotation: Quat::from_rotation_y(0.5),
        samples: vec![
            TrajectorySample {
                position: Vec3::new(1.0, 0.0, 3.0),
                velocity: Vec3::new(0.0, 0.0, -0.25),
                rotation: Quat::from_rotation_y(0.6),
            };
            2
        ],
    };
    let mut board = AnimationBlackboard::default();
    board.set(TRAJECTORY_KEY, published.clone());

    instance.delta_time = 0.1;
    instance.search_timer = 0.0;
    instance.step(&node, instance.delta_time, &board, fixture.sources());

    let last = instance.last_query.as_ref().unwrap();
    assert_eq!(last.features, [0.25]);
    assert_eq!(last.frame, 3);
    assert_ne!(instance.frame, last.frame);
    assert_eq!(last.simulation.translation, published.translation);
    assert_eq!(last.simulation.rotation, published.rotation);
    assert_eq!(last.trajectory, published.samples);
}

#[test]
fn a_search_keeps_the_cost_of_the_frame_it_found() {
    let fixture = fixture(1.0);
    let node = MotionMatchingNode::default();

    let instance = searched(&fixture, &node);

    let last = instance.last_query.as_ref().unwrap();
    let mut query = last.features.clone();
    fixture.database.normalise(&mut query);
    let (_, expected) = search(
        &fixture.database,
        &query,
        Some(last.frame),
        node.ignore_surrounding,
        node.ignore_range_end,
    )
    .unwrap();
    assert_eq!(last.best_cost, Some(expected));
}

#[test]
fn the_node_passes_each_exclusion_to_its_own_side_of_the_search() {
    let fixture = fixture(1.0);
    let node = MotionMatchingNode {
        ignore_surrounding: 1,
        ignore_range_end: 60,
        ..Default::default()
    };

    let instance = searched(&fixture, &node);

    let last = instance.last_query.as_ref().unwrap();
    let mut query = last.features.clone();
    fixture.database.normalise(&mut query);
    let cost_with = |surrounding, range_end| {
        search(
            &fixture.database,
            &query,
            Some(last.frame),
            surrounding,
            range_end,
        )
        .map(|(_, cost)| cost)
    };
    assert_ne!(cost_with(1, 60), cost_with(60, 1));
    assert_eq!(last.best_cost, cost_with(1, 60));
}

#[test]
fn a_non_finite_query_leaves_no_best_cost() {
    let fixture = fixture(1.0);
    let node = MotionMatchingNode::default();
    let mut instance = instance(&node, 0.0);
    instance.step(&node, instance.delta_time, &quiet(), fixture.sources());

    instance.delta_time = 0.1;
    instance.search_timer = 0.0;
    instance.step(
        &node,
        instance.delta_time,
        &trajectory(f32::INFINITY, 2),
        fixture.sources(),
    );

    let last = instance.last_query.as_ref().unwrap();
    assert_eq!(last.best_cost, None);
}

#[test]
fn a_trajectory_too_short_to_search_clears_the_last_query() {
    let fixture = fixture(1.0);
    let node = MotionMatchingNode::default();
    let mut instance = searched(&fixture, &node);
    assert!(instance.last_query.is_some());

    instance.delta_time = 0.1;
    instance.search_timer = 0.0;
    instance.step(
        &node,
        instance.delta_time,
        &trajectory(0.25, 1),
        fixture.sources(),
    );

    assert!(instance.last_query.is_none());
}

#[test]
fn a_tick_with_no_search_due_keeps_the_last_query() {
    let fixture = fixture(1.0);
    let node = MotionMatchingNode::default();
    let mut instance = searched(&fixture, &node);

    instance.delta_time = 0.03;
    instance.search_timer = 10.0;
    instance.step(
        &node,
        instance.delta_time,
        &trajectory(0.5, 2),
        fixture.sources(),
    );

    let last = instance.last_query.as_ref().unwrap();
    assert_eq!(last.features, [0.25]);
}

#[test]
fn a_reset_clears_the_last_query() {
    let fixture = fixture(1.0);
    let node = MotionMatchingNode::default();
    let mut instance = searched(&fixture, &node);
    assert!(instance.last_query.is_some());

    node.reset(&mut instance);

    assert!(instance.last_query.is_none());
}

#[test]
fn no_search_runs_before_the_timer_is_due() {
    let fixture = fixture(1.0);
    let node = MotionMatchingNode::default();
    let mut instance = instance(&node, 0.0);
    instance.step(&node, instance.delta_time, &quiet(), fixture.sources());

    instance.delta_time = 0.03;
    instance.search_timer = 10.0;
    instance.step(
        &node,
        instance.delta_time,
        &trajectory(0.0, 2),
        fixture.sources(),
    );

    assert_eq!(fixture.database.entry_of(instance.frame), Some(0));
}

#[test]
fn the_last_ignore_range_end_frames_force_a_search_before_the_timer_is_due() {
    let fixture = fixture(1.0);
    let node = MotionMatchingNode::default();
    let mut instance = instance(&node, 0.0);
    instance.step(&node, instance.delta_time, &quiet(), fixture.sources());
    instance.frame = 40;
    instance.search_timer = 10.0;

    instance.delta_time = 0.0;
    instance.step(
        &node,
        instance.delta_time,
        &trajectory(0.0, 2),
        fixture.sources(),
    );

    assert_eq!(fixture.database.entry_of(instance.frame), Some(1));
}

#[test]
fn the_last_frame_of_a_range_never_holds_against_its_own_cost() {
    let fixture = fixture(1.0);
    let node = MotionMatchingNode::default();
    let mut instance = instance(&node, 0.0);
    instance.step(&node, instance.delta_time, &quiet(), fixture.sources());
    let last = fixture.database.range_of(0).unwrap().end - 1;
    instance.frame = last;
    instance.search_timer = 10.0;
    let simulation = fixture.database.simulation(last);
    let mut board = AnimationBlackboard::default();
    board.set(
        TRAJECTORY_KEY,
        MotionMatchingTrajectory {
            translation: simulation.translation,
            rotation: simulation.rotation,
            samples: fixture.database.trajectory_samples(last),
        },
    );

    instance.delta_time = 0.0;
    instance.step(&node, instance.delta_time, &board, fixture.sources());

    assert_ne!(instance.frame, last);
}

#[test]
fn the_last_frame_of_a_range_moves_to_the_next_ranges_first_frame() {
    let fixture = fixture(1.0);
    let node = MotionMatchingNode::default();
    let mut instance = instance(&node, 0.0);
    instance.step(&node, instance.delta_time, &quiet(), fixture.sources());
    let last = fixture.database.range_of(0).unwrap().end - 1;
    instance.frame = last;
    instance.search_timer = 10.0;

    instance.delta_time = 0.0;
    instance.step(
        &node,
        instance.delta_time,
        &trajectory(0.0, 2),
        fixture.sources(),
    );

    let mut query = instance.last_query.as_ref().unwrap().features.clone();
    fixture.database.normalise(&mut query);
    let (best, _) = search(
        &fixture.database,
        &query,
        None,
        node.ignore_surrounding,
        node.ignore_range_end,
    )
    .unwrap();
    assert_eq!(best, last + 1);
    assert_eq!(instance.frame, last + 1);
}

#[test]
fn a_nan_query_leaves_no_best_cost() {
    let fixture = fixture(1.0);
    let node = MotionMatchingNode::default();
    let mut instance = instance(&node, 0.0);
    instance.step(&node, instance.delta_time, &quiet(), fixture.sources());

    instance.delta_time = 0.1;
    instance.search_timer = 0.0;
    instance.step(
        &node,
        instance.delta_time,
        &trajectory(f32::NAN, 2),
        fixture.sources(),
    );

    let last = instance.last_query.as_ref().unwrap();
    assert_eq!(last.best_cost, None);
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
    instance.step(&node, instance.delta_time, &quiet(), fixture.sources());
    instance.search_timer = 100.0;

    instance.delta_time = 0.1;
    instance.step(
        &node,
        instance.delta_time,
        &trajectory(0.0, 2),
        fixture.sources(),
    );
    assert!(instance.forced_search_timer <= 0.0);

    instance.delta_time = 0.1;
    instance.step(
        &node,
        instance.delta_time,
        &trajectory(5.0, 2),
        fixture.sources(),
    );
    assert!(instance.forced_search_timer <= 0.0);

    instance.delta_time = 0.1;
    instance.step(
        &node,
        instance.delta_time,
        &trajectory(5.0, 2),
        fixture.sources(),
    );
    assert_eq!(instance.forced_search_timer, 100.0);
}

#[test]
fn the_output_pose_on_a_transition_frame_equals_the_pose_before_it() {
    let fixture = fixture(1.0);
    let node = MotionMatchingNode::default();
    let mut instance = instance(&node, 0.0);
    instance.step(&node, instance.delta_time, &quiet(), fixture.sources());
    instance.delta_time = 0.1;
    instance.step(&node, instance.delta_time, &quiet(), fixture.sources());
    let before = joints(&instance.blended);
    let frame_before = instance.frame;

    instance.delta_time = 0.0;
    instance.search_timer = 0.0;
    instance.step(
        &node,
        instance.delta_time,
        &trajectory(0.0, 2),
        fixture.sources(),
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

    instance.step(&node, instance.delta_time, &quiet(), sources);

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

    instance.step(&node, instance.delta_time, &quiet(), sources);

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

    instance.step(&node, instance.delta_time, &quiet(), fixture.sources());

    assert!(!instance.initialised);
}

#[test]
fn root_motion_is_the_simulation_bones_velocity_at_the_frame() {
    let fixture = fixture(1.0);
    let node = MotionMatchingNode::default();
    let mut instance = instance(&node, 0.0);
    instance.delta_time = 0.5;

    instance.step(&node, instance.delta_time, &quiet(), fixture.sources());

    let motion = instance.root_motion;
    assert!(
        (motion.velocity.length() - 1.0).abs() < 0.3,
        "{:?}",
        motion.velocity
    );
}

#[test]
fn root_motion_reads_forward_for_a_walk_heading_away_from_model_forward() {
    let root = Uuid::from_u128(1);
    let rest = Quat::from_rotation_y(0.6);
    for heading in [0.0, FRAC_PI_2, -2.4] {
        let heading = Quat::from_rotation_y(heading);
        let mut walk = AnimationClip::default();
        walk.add_channel(
            root,
            AnimationChannel::new(
                vec![0.0, 2.0],
                AnimationChanelOutput::Translation(vec![
                    Vec3::ZERO,
                    heading * Vec3::new(0.0, 0.0, -2.0),
                ]),
            ),
        );
        walk.add_channel(
            root,
            AnimationChannel::new(
                vec![0.0, 2.0],
                AnimationChanelOutput::Rotation(vec![heading * rest; 2]),
            ),
        );
        let fixture = fixture_of(rest, walk);
        let node = MotionMatchingNode::default();
        let mut instance = instance(&node, 0.1);

        instance.step(&node, instance.delta_time, &quiet(), fixture.sources());

        let motion = instance.output.root_motion();
        assert!(
            motion.translation.distance(Vec3::new(0.0, 0.0, -0.1)) < 1e-3,
            "{:?}",
            motion.translation
        );
        assert!(motion.rotation.angle_between(Quat::IDENTITY) < 1e-3);
    }
}

#[test]
fn root_motion_turns_out_the_rest_of_the_skeleton_it_plays_on() {
    let root = Uuid::from_u128(1);
    let baked_rest = Quat::from_rotation_y(0.6);
    let played_rest = Quat::from_rotation_x(FRAC_PI_2);
    let mut walk = AnimationClip::default();
    walk.add_channel(
        root,
        AnimationChannel::new(
            vec![0.0, 2.0],
            AnimationChanelOutput::Translation(vec![Vec3::ZERO, Vec3::new(0.0, 0.0, -2.0)]),
        ),
    );
    walk.add_channel(
        root,
        AnimationChannel::new(
            vec![0.0, 2.0],
            AnimationChanelOutput::Rotation(vec![baked_rest; 2]),
        ),
    );
    let fixture = fixture_of(baked_rest, walk);
    let mut rest_pose = vec![concerto_mesh::JointPose::IDENTITY; 3];
    rest_pose[1].rotation = played_rest;
    let played = Skeleton::new(
        vec![glam::Mat4::IDENTITY; 3],
        vec![None, Some(0), Some(1)],
        vec!["Spine2".into(), "Root".into(), "Hips".into()],
        rest_pose,
        vec![Uuid::from_u128(3), root, Uuid::from_u128(2)],
    )
    .unwrap();
    let node = MotionMatchingNode::default();
    let mut instance = instance(&node, 0.1);
    let sources = Sources {
        player_skeleton: &played,
        ..fixture.sources()
    };

    instance.step(&node, instance.delta_time, &quiet(), sources);

    let expected = played_rest * baked_rest.inverse() * Vec3::new(0.0, 0.0, -0.1);
    let motion = instance.output.root_motion();
    assert!(
        motion.translation.distance(expected) < 1e-3,
        "{:?}",
        motion.translation
    );
}

#[test]
fn root_motion_carries_a_leaning_root_to_where_it_faces_next() {
    let root = Uuid::from_u128(1);
    let rest = Quat::from_rotation_y(0.6);
    let facing = |time: f32| Quat::from_rotation_y(TURN_RATE * time) * Quat::from_rotation_x(0.4);
    let times: Vec<f32> = (0..5).map(|key| key as f32 * 0.5).collect();
    let rotations = times.iter().map(|time| facing(*time) * rest).collect();
    let mut spin = AnimationClip::default();
    spin.add_channel(
        root,
        AnimationChannel::new(times, AnimationChanelOutput::Rotation(rotations)),
    );
    let fixture = fixture_of(rest, spin);
    let node = MotionMatchingNode::default();
    let dt = 0.1;
    let mut instance = instance(&node, dt);

    instance.step(&node, instance.delta_time, &quiet(), fixture.sources());

    let (_, time) = fixture.database.clip_time(instance.frame).unwrap();
    let arrived = facing(time) * instance.output.root_motion().rotation;
    assert!(arrived.angle_between(facing(time + dt)) < 1e-3);
}

#[test]
fn a_pose_held_at_the_end_of_its_range_gives_no_root_motion() {
    let fixture = fixture(1.0);
    let node = MotionMatchingNode::default();
    let mut instance = instance(&node, 0.0);
    instance.step(&node, instance.delta_time, &quiet(), fixture.sources());
    let last = fixture.database.range_of(0).unwrap().end - 1;
    instance.frame = last - 1;

    instance.delta_time = 0.1;
    instance.step(&node, instance.delta_time, &quiet(), fixture.sources());
    assert_eq!(instance.frame, last);
    assert!(instance.output.root_motion().translation.z < 0.0);

    for dt in [0.1, 0.001] {
        instance.delta_time = dt;
        instance.step(&node, instance.delta_time, &quiet(), fixture.sources());
        assert_eq!(instance.frame, last);
        assert_eq!(instance.output.root_motion(), RootMotion::IDENTITY);
    }
}

#[test]
fn a_jump_away_from_the_end_of_a_range_moves_the_root_again() {
    let fixture = fixture(1.0);
    let node = MotionMatchingNode::default();
    let mut instance = instance(&node, 0.0);
    instance.step(&node, instance.delta_time, &quiet(), fixture.sources());
    let last = fixture.database.range_of(0).unwrap().end - 1;
    instance.frame = last;

    instance.delta_time = 0.1;
    instance.step(
        &node,
        instance.delta_time,
        &trajectory(1.0, 2),
        fixture.sources(),
    );

    assert_ne!(instance.frame, last);
    assert_ne!(instance.output.root_motion(), RootMotion::IDENTITY);
}

#[test]
fn steps_shorter_than_a_database_frame_add_up_to_the_distance_played() {
    let fixture = fixture(1.0);
    let node = MotionMatchingNode::default();
    let mut instance = instance(&node, 0.0);
    instance.step(&node, instance.delta_time, &quiet(), fixture.sources());
    instance.frame = 20;
    let steps = 40;
    let dt = 0.5 / steps as f32;

    let mut travelled = Vec3::ZERO;
    for _ in 0..steps {
        instance.delta_time = dt;
        instance.step(&node, instance.delta_time, &quiet(), fixture.sources());
        travelled += instance.output.root_motion().translation;
    }

    assert!(
        travelled.distance(Vec3::new(0.0, 0.0, -0.5)) < 1e-3,
        "{travelled:?}"
    );
}

fn moved(x: f32) -> JointPose {
    JointPose {
        translation: Vec3::new(x, 0.0, 0.0),
        ..JointPose::IDENTITY
    }
}

#[test]
fn an_uninitialised_instance_leaves_the_output_untouched() {
    let mut instance = MotionMatchingInstance::new(0.1);
    instance.output = Pose::from_joints(vec![moved(5.0)]);
    let mut output = Pose::from_joints(vec![moved(1.0)]);

    assert!(!instance.write_output(&mut output));
    assert_eq!(output.joints()[0].translation.x, 1.0);
}

#[test]
fn an_initialised_instance_copies_its_pose_to_the_output() {
    let mut instance = MotionMatchingInstance::new(0.1);
    instance.output = Pose::from_joints(vec![moved(5.0)]);
    instance.initialised = true;
    let mut output = Pose::from_joints(vec![moved(1.0)]);

    assert!(instance.write_output(&mut output));
    assert_eq!(output.joints()[0].translation.x, 5.0);
}
