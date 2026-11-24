use std::f32::consts::FRAC_PI_2;

use concerto_animation::{
    blackboard::AnimationBlackboard,
    clip::{AnimationChanelOutput, AnimationChannel, AnimationClip},
    event::AnimationEvent,
    pose::{FiredEvent, RootMotion},
};
use concerto_foundation::assets::{handle::AssetHandle, AssetId};
use concerto_mesh::{skeleton::Skeleton, JointPose};
use concerto_name::name;
use glam::{Quat, Vec3};
use uuid::Uuid;

use super::{MotionMatchingState, Sources};
use crate::{
    database::MotionDatabase,
    node::{trajectory_key, MotionMatchingNode, MotionMatchingTrajectory},
    simulation::TrajectorySample,
    test_support::{
        baked, pace_database, pace_database_of, pace_definition, skeleton, turn_left, walk_forward,
        TURN_RATE,
    },
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
        trajectory_key(),
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

fn state(node: &MotionMatchingNode, dt: f32) -> MotionMatchingState {
    let mut state = MotionMatchingState::new(node.search_period);
    state.delta_time = dt;
    state
}

fn quiet() -> AnimationBlackboard {
    AnimationBlackboard::default()
}

#[test]
fn the_first_step_initialises_the_state_and_outputs_the_first_frame() {
    let fixture = fixture(1.0);
    let node = MotionMatchingNode::default();
    let mut state = state(&node, 0.0);
    assert!(!state.initialised);

    state.step(&node, state.delta_time, 1.0, &quiet(), fixture.sources());

    assert!(state.initialised);
    assert!(!state.failed);
    assert_eq!(state.frame, 0);
    let expected = fixture
        .database
        .pose(0, &fixture.skeleton, &[&fixture.walk, &fixture.turn])
        .unwrap();
    assert_eq!(state.output.len(), 3);
    for (joint, expected) in state.output.joints().iter().zip(&expected) {
        assert!(joint.translation.distance(expected.translation) < 1e-5);
    }
}

#[test]
fn playback_at_speed_two_advances_two_clip_seconds_per_second() {
    let fixture = fixture(2.0);
    let node = MotionMatchingNode::default();
    let mut state = state(&node, 0.0);
    state.step(&node, state.delta_time, 1.0, &quiet(), fixture.sources());
    let (_, start) = fixture.database.clip_time(state.frame).unwrap();

    state.delta_time = 0.5;
    state.step(&node, state.delta_time, 1.0, &quiet(), fixture.sources());

    let (_, end) = fixture.database.clip_time(state.frame).unwrap();
    assert!((end - start - 1.0).abs() < 1e-3, "{start} -> {end}");
}

#[test]
fn a_standing_trajectory_moves_the_playhead_into_the_turn_clip() {
    let fixture = fixture(1.0);
    let node = MotionMatchingNode::default();
    let mut state = state(&node, 0.0);
    state.step(&node, state.delta_time, 1.0, &quiet(), fixture.sources());

    state.delta_time = 0.1;
    state.search_timer = 0.0;
    state.step(
        &node,
        state.delta_time,
        1.0,
        &trajectory(0.0, 2),
        fixture.sources(),
    );

    assert_eq!(fixture.database.entry_of(state.frame), Some(1));
    assert!((state.search_timer - node.search_period).abs() < 1e-6);
}

#[test]
fn a_search_skips_the_frames_the_node_ignores_around_the_current_one_and_at_range_ends() {
    let fixture = fixture(1.0);
    let ignoring = [
        MotionMatchingNode::default().with_ignore_surrounding(1000),
        MotionMatchingNode::default().with_ignore_range_end(1000),
    ];

    for node in ignoring {
        let state = searched(&fixture, &node);

        assert_eq!(state.frame, 3);
    }
}

fn searched(fixture: &Fixture, node: &MotionMatchingNode) -> MotionMatchingState {
    let mut state = state(node, 0.0);
    state.step(node, state.delta_time, 1.0, &quiet(), fixture.sources());
    state.delta_time = 0.1;
    state.search_timer = 0.0;
    state.step(
        node,
        state.delta_time,
        1.0,
        &trajectory(0.25, 2),
        fixture.sources(),
    );
    state
}

#[test]
fn a_search_keeps_its_raw_query_with_what_it_was_built_from() {
    let fixture = fixture(1.0);
    let node = MotionMatchingNode::default();
    let mut state = state(&node, 0.0);
    state.step(&node, state.delta_time, 1.0, &quiet(), fixture.sources());
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
    board.set(trajectory_key(), published.clone());

    state.delta_time = 0.1;
    state.search_timer = 0.0;
    state.step(&node, state.delta_time, 1.0, &board, fixture.sources());

    let last = state.last_query.as_ref().unwrap();
    assert_eq!(last.features, [0.25]);
    assert_eq!(last.frame, 3);
    assert_ne!(state.frame, last.frame);
    assert_eq!(last.simulation.translation, published.translation);
    assert_eq!(last.simulation.rotation, published.rotation);
    assert_eq!(last.trajectory, published.samples);
}

#[test]
fn no_search_runs_before_the_timer_is_due() {
    let fixture = fixture(1.0);
    let node = MotionMatchingNode::default();
    let mut state = state(&node, 0.0);
    state.step(&node, state.delta_time, 1.0, &quiet(), fixture.sources());

    state.delta_time = 0.03;
    state.search_timer = 10.0;
    state.step(
        &node,
        state.delta_time,
        1.0,
        &trajectory(0.0, 2),
        fixture.sources(),
    );

    assert_eq!(fixture.database.entry_of(state.frame), Some(0));
}

#[test]
fn the_last_ignore_range_end_frames_force_a_search_before_the_timer_is_due() {
    let fixture = fixture(1.0);
    let node = MotionMatchingNode::default();
    let mut state = state(&node, 0.0);
    state.step(&node, state.delta_time, 1.0, &quiet(), fixture.sources());
    state.frame = 40;
    state.search_timer = 10.0;

    state.delta_time = 0.0;
    state.step(
        &node,
        state.delta_time,
        1.0,
        &trajectory(0.0, 2),
        fixture.sources(),
    );

    assert_eq!(fixture.database.entry_of(state.frame), Some(1));
}

#[test]
fn a_velocity_change_that_settles_forces_one_search() {
    let fixture = fixture(1.0);
    let node = MotionMatchingNode::default()
        .with_search_period(100.0)
        .with_velocity_change_threshold(1.0);
    let mut state = state(&node, 0.0);
    state.step(&node, state.delta_time, 1.0, &quiet(), fixture.sources());
    state.search_timer = 100.0;

    state.delta_time = 0.1;
    state.step(
        &node,
        state.delta_time,
        1.0,
        &trajectory(0.0, 2),
        fixture.sources(),
    );
    assert!(state.forced_search_timer <= 0.0);

    state.delta_time = 0.1;
    state.step(
        &node,
        state.delta_time,
        1.0,
        &trajectory(5.0, 2),
        fixture.sources(),
    );
    assert!(state.forced_search_timer <= 0.0);

    state.delta_time = 0.1;
    state.step(
        &node,
        state.delta_time,
        1.0,
        &trajectory(5.0, 2),
        fixture.sources(),
    );
    assert_eq!(state.forced_search_timer, 100.0);
}

#[test]
fn the_output_pose_on_a_transition_frame_equals_the_pose_before_it() {
    let fixture = fixture(1.0);
    let node = MotionMatchingNode::default();
    let mut state = state(&node, 0.0);
    state.step(&node, state.delta_time, 1.0, &quiet(), fixture.sources());
    state.delta_time = 0.1;
    state.step(&node, state.delta_time, 1.0, &quiet(), fixture.sources());
    let before = state.blended.joints().to_vec();
    let frame_before = state.frame;

    state.delta_time = 0.0;
    state.search_timer = 0.0;
    state.step(
        &node,
        state.delta_time,
        1.0,
        &trajectory(0.0, 2),
        fixture.sources(),
    );

    assert_ne!(fixture.database.entry_of(state.frame), Some(0));
    assert_ne!(state.frame, frame_before);
    for (joint, before) in state.blended.joints().iter().zip(before) {
        assert!(joint.translation.distance(before.translation) < 1e-4);
        assert!(joint.rotation.dot(before.rotation).abs() > 0.9999);
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
    let mut state = state(&node, 0.0);
    let sources = Sources {
        player_skeleton: &reordered,
        ..fixture.sources()
    };

    state.step(&node, state.delta_time, 1.0, &quiet(), sources);

    assert_eq!(state.bone_indices[&Uuid::from_u128(1)], 1);
    assert_eq!(state.bone_indices[&Uuid::from_u128(2)], 2);
    assert_eq!(state.bone_indices[&Uuid::from_u128(3)], 0);
    let expected = fixture
        .database
        .pose(0, &fixture.skeleton, &[&fixture.walk, &fixture.turn])
        .unwrap();
    assert!(
        state.output.joints()[2]
            .translation
            .distance(expected[1].translation)
            < 1e-5
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
        let mut state = state(&node, 0.1);

        state.step(&node, state.delta_time, 1.0, &quiet(), fixture.sources());

        let motion = state.delta;
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
    let mut state = state(&node, 0.1);
    let sources = Sources {
        player_skeleton: &played,
        ..fixture.sources()
    };

    state.step(&node, state.delta_time, 1.0, &quiet(), sources);

    let expected = played_rest * baked_rest.inverse() * Vec3::new(0.0, 0.0, -0.1);
    let motion = state.delta;
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
    let mut state = state(&node, dt);

    state.step(&node, state.delta_time, 1.0, &quiet(), fixture.sources());

    let (_, time) = fixture.database.clip_time(state.frame).unwrap();
    let arrived = facing(time) * state.delta.rotation;
    assert!(arrived.angle_between(facing(time + dt)) < 1e-3);
}

fn started(fixture: &Fixture, node: &MotionMatchingNode) -> MotionMatchingState {
    let mut state = state(node, 0.0);
    state.step(node, state.delta_time, 1.0, &quiet(), fixture.sources());
    state
}

#[test]
fn at_rate_two_the_playhead_advances_twice_the_frames_of_rate_one() {
    let fixture = fixture(1.0);
    let node = MotionMatchingNode::default();
    let dt = 2.65 / fixture.database.rate;
    let advanced = |rate: f32| {
        let mut state = started(&fixture, &node);
        let start = state.frame;
        for _ in 0..2 {
            state.step(&node, dt, rate, &quiet(), fixture.sources());
        }
        state.frame - start
    };

    let once = advanced(1.0);

    assert_eq!(once, 5);
    assert_eq!(advanced(2.0), 2 * once);
}

#[test]
fn at_rate_zero_the_frame_holds_with_no_root_motion_and_no_search() {
    let fixture = fixture(1.0);
    let node = MotionMatchingNode::default();
    let mut state = started(&fixture, &node);
    let start = state.frame;
    state.search_timer = 0.0;

    state.step(&node, 0.1, 0.0, &trajectory(0.25, 2), fixture.sources());

    assert_eq!(state.frame, start);
    assert_eq!(state.delta, RootMotion::IDENTITY);
    assert!(state.last_query.is_none());
}

const STEP: f32 = 0.05;

fn stepping(mut walk: AnimationClip) -> AnimationClip {
    walk.add_event(
        STEP,
        AnimationEvent {
            name: name!("step"),
        },
    );
    walk
}

fn accelerating() -> AnimationClip {
    let mut clip = AnimationClip::default();
    clip.add_channel(
        Uuid::from_u128(1),
        AnimationChannel::new(
            vec![0.0, 1.0, 2.0],
            AnimationChanelOutput::Translation(vec![
                Vec3::ZERO,
                Vec3::new(0.0, 0.0, -0.5),
                Vec3::new(0.0, 0.0, -2.0),
            ]),
        ),
    );
    clip
}

fn evented(walk: AnimationClip, mirrored: bool) -> Fixture {
    let skeleton = skeleton();
    let turn = turn_left();
    let mut definition = pace_definition(
        1.0,
        Default::default(),
        [
            AssetHandle::weak(AssetId::new()),
            AssetHandle::weak(AssetId::new()),
        ],
    );
    for entry in &mut definition.entries {
        entry.mirrored = mirrored;
    }
    Fixture {
        database: baked(definition, &skeleton, &[&walk, &turn]),
        skeleton,
        walk,
        turn,
    }
}

fn walked(fixture: &Fixture) -> MotionMatchingState {
    let node = MotionMatchingNode::default();
    let mut state = started(fixture, &node);
    state.delta_time = 0.1;
    state.step(&node, state.delta_time, 1.0, &quiet(), fixture.sources());
    state
}

fn step_on(fixture: &Fixture, mirrored: bool) -> FiredEvent {
    FiredEvent {
        clip: fixture.database.entries[0].clip.id(),
        key: 0,
        time: STEP,
        weight: 1.0,
        mirrored,
    }
}

#[test]
fn a_step_past_a_clip_event_fires_it_at_full_weight_marked_mirrored_on_a_mirrored_entry() {
    for mirrored in [false, true] {
        let fixture = evented(stepping(walk_forward()), mirrored);

        let state = walked(&fixture);

        assert_eq!(state.frame, 3);
        assert_eq!(state.fired, [step_on(&fixture, mirrored)]);
    }
}

#[test]
fn a_search_jump_forward_within_an_entry_fires_nothing() {
    let fixture = evented(stepping(accelerating()), false);
    let node = MotionMatchingNode::default();
    let mut state = started(&fixture, &node);

    state.delta_time = 0.1;
    state.search_timer = 0.0;
    state.step(
        &node,
        state.delta_time,
        1.0,
        &trajectory(1.5, 2),
        fixture.sources(),
    );

    assert_eq!(fixture.database.entry_of(state.frame), Some(0));
    assert!(state.frame > 3, "{}", state.frame);
    assert!(state.fired.is_empty(), "{:?}", state.fired);
}

fn turned_after_walking(fixture: &Fixture, ticks_per_frame: usize) -> Vec<JointPose> {
    let node = MotionMatchingNode::default().with_search_period(100.0);
    let mut state = started(fixture, &node);
    state.frame = 20;
    let dt = 1.0001 / (fixture.database.rate * ticks_per_frame as f32);
    let ticks = 8 * ticks_per_frame;
    for _ in 0..ticks {
        state.step(&node, dt, 1.0, &quiet(), fixture.sources());
    }
    state.search_timer = 0.0;
    state.step(&node, 0.0, 1.0, &trajectory(0.0, 2), fixture.sources());
    assert_eq!(fixture.database.entry_of(state.frame), Some(1));
    for _ in 0..ticks {
        state.step(&node, dt, 1.0, &quiet(), fixture.sources());
    }
    state.blended.joints().to_vec()
}

fn swinging(mut walk: AnimationClip) -> AnimationClip {
    walk.add_channel(
        Uuid::from_u128(3),
        AnimationChannel::new(
            vec![0.0, 2.0],
            AnimationChanelOutput::Rotation(vec![Quat::IDENTITY, Quat::from_rotation_x(3.0)]),
        ),
    );
    walk
}

#[test]
fn play_comes_out_the_same_at_any_tick_rate() {
    let fixture = fixture(1.0);
    let node = MotionMatchingNode::default();
    let mut state = state(&node, 0.0);
    state.step(&node, state.delta_time, 1.0, &quiet(), fixture.sources());
    state.frame = 20;
    let steps = 40;
    let dt = 0.5 / steps as f32;

    let mut travelled = Vec3::ZERO;
    for _ in 0..steps {
        state.delta_time = dt;
        state.step(&node, state.delta_time, 1.0, &quiet(), fixture.sources());
        travelled += state.delta.translation;
    }

    assert!(
        travelled.distance(Vec3::new(0.0, 0.0, -0.5)) < 1e-3,
        "{travelled:?}"
    );

    let swinging = fixture_of(skeleton().rest_pose[0].rotation, swinging(walk_forward()));

    let coarse = turned_after_walking(&swinging, 1);
    let fine = turned_after_walking(&swinging, 16);

    for (coarse, fine) in coarse.iter().zip(&fine) {
        assert!(
            coarse.translation.distance(fine.translation) < 1e-3,
            "{coarse:?} at one tick a frame, {fine:?} at sixteen"
        );
        assert!(
            coarse.rotation.angle_between(fine.rotation) < 1e-3,
            "{coarse:?} at one tick a frame, {fine:?} at sixteen"
        );
    }
}
