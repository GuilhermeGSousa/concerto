use concerto_animation::clip::{AnimationChanelOutput, AnimationChannel, AnimationClip};
use concerto_foundation::math::spring_damper::CharacterState;
use concerto_foundation::transform::Transform;
use concerto_mesh::{JointPose, Skeleton};
use concerto_motion_matching::bake::{
    bake, model_pose, model_velocities, plan_entries, reexpress_roots, sample_pose, simulation_bone,
};
use concerto_motion_matching::{
    BakeError, BodyMotion, ChannelSource, CharacterSimulation, FeatureChannel, FrameContext,
    MotionDatabaseDefinition, MotionEntry, ReflectFeatureChannel, SimulationBoneSettings,
    StoredChannel, TimeRange, TrajectorySample, TrajectorySettings,
};
use concerto_reflect::{to_value, Reflect, TypeRegistry};
use glam::{Mat4, Quat, Vec3};
use uuid::Uuid;

fn skeleton() -> Skeleton {
    Skeleton::new(
        vec![Mat4::IDENTITY; 2],
        vec![None, Some(0)],
        vec!["Root".into(), "Child".into()],
        vec![
            JointPose::IDENTITY,
            JointPose {
                translation: Vec3::new(0.0, 1.0, 0.0),
                ..JointPose::IDENTITY
            },
        ],
        vec![Uuid::from_u128(1), Uuid::from_u128(2)],
    )
    .unwrap()
}

fn walk_clip(duration: f32) -> AnimationClip {
    let mut clip = AnimationClip::default();
    clip.add_channel(
        Uuid::from_u128(1),
        AnimationChannel::new(
            vec![0.0, duration],
            AnimationChanelOutput::Translation(vec![Vec3::ZERO, Vec3::new(0.0, 0.0, -duration)]),
        ),
    );
    clip
}

fn definition(entries: Vec<MotionEntry>, rate: f32, speed: f32) -> MotionDatabaseDefinition {
    MotionDatabaseDefinition {
        entries,
        rate,
        speed,
        ..Default::default()
    }
}

fn entry(range: Option<(f32, f32)>) -> MotionEntry {
    MotionEntry {
        clip: Default::default(),
        range: range.map(|(start, end)| TimeRange { start, end }),
    }
}

#[test]
fn ranges_are_contiguous_for_whole_clips_and_sub_ranges() {
    let skeleton = skeleton();
    let first = walk_clip(1.0);
    let second = walk_clip(2.0);
    let definition = definition(vec![entry(None), entry(Some((0.5, 1.5)))], 60.0, 1.0);

    let plans = plan_entries(&definition, &skeleton, &[&first, &second]).unwrap();

    assert_eq!(plans.len(), 2);
    assert_eq!((plans[0].first_frame, plans[0].frames), (0, 60));
    assert_eq!((plans[1].first_frame, plans[1].frames), (60, 60));
    assert_eq!(plans[1].start, 0.5);
}

#[test]
fn speed_scales_the_frame_count() {
    let skeleton = skeleton();
    let clip = walk_clip(1.0);
    let definition = definition(vec![entry(None)], 60.0, 2.0);

    let plans = plan_entries(&definition, &skeleton, &[&clip]).unwrap();

    assert_eq!(plans[0].frames, 30);
    assert_eq!(plans[0].frame_time(3, 60.0, 2.0), 0.1);
}

#[test]
fn speed_one_reproduces_the_clip_pose() {
    let skeleton = skeleton();
    let clip = walk_clip(1.0);
    let definition = definition(vec![entry(None)], 60.0, 1.0);
    let plan = plan_entries(&definition, &skeleton, &[&clip])
        .unwrap()
        .remove(0);

    let time = plan.frame_time(30, 60.0, 1.0);
    let pose = sample_pose(&skeleton, &clip, time);

    assert!((pose[0].translation.z + 0.5).abs() < 1e-5);
    assert_eq!(pose[1], skeleton.rest_pose[1]);
}

#[test]
fn speed_two_samples_twice_as_far() {
    let skeleton = skeleton();
    let clip = walk_clip(1.0);
    let definition = definition(vec![entry(None)], 60.0, 2.0);
    let plan = plan_entries(&definition, &skeleton, &[&clip])
        .unwrap()
        .remove(0);

    let pose = sample_pose(&skeleton, &clip, plan.frame_time(15, 60.0, 2.0));

    assert!((pose[0].translation.z + 0.5).abs() < 1e-5);
}

#[test]
fn a_range_offsets_the_sampled_time() {
    let skeleton = skeleton();
    let clip = walk_clip(2.0);
    let definition = definition(vec![entry(Some((0.5, 1.5)))], 60.0, 1.0);
    let plan = plan_entries(&definition, &skeleton, &[&clip])
        .unwrap()
        .remove(0);

    let pose = sample_pose(&skeleton, &clip, plan.frame_time(0, 60.0, 1.0));

    assert!((pose[0].translation.z + 0.5).abs() < 1e-5);
}

#[test]
fn rate_and_speed_must_be_positive() {
    let skeleton = skeleton();
    let clip = walk_clip(1.0);
    for rate in [0.0, -1.0, f32::NAN] {
        let definition = definition(vec![entry(None)], rate, 1.0);
        let error = plan_entries(&definition, &skeleton, &[&clip])
            .err()
            .unwrap();
        assert!(matches!(error, BakeError::InvalidRate(_)));
    }
    for speed in [0.0, -1.0, f32::NAN] {
        let definition = definition(vec![entry(None)], 60.0, speed);
        let error = plan_entries(&definition, &skeleton, &[&clip])
            .err()
            .unwrap();
        assert!(matches!(error, BakeError::InvalidSpeed(_)));
    }
}

#[test]
fn the_clip_count_must_match_the_entries() {
    let skeleton = skeleton();
    let clip = walk_clip(1.0);
    let definition = definition(vec![entry(None), entry(None)], 60.0, 1.0);
    let error = plan_entries(&definition, &skeleton, &[&clip])
        .err()
        .unwrap();
    assert_eq!(
        error,
        BakeError::ClipCountMismatch {
            expected: 2,
            actual: 1
        }
    );
}

#[test]
fn a_clip_without_the_skeletons_bones_is_an_error() {
    let skeleton = skeleton();
    let mut clip = AnimationClip::default();
    clip.add_channel(
        Uuid::from_u128(99),
        AnimationChannel::new(
            vec![0.0, 1.0],
            AnimationChanelOutput::Translation(vec![Vec3::ZERO, Vec3::X]),
        ),
    );
    let definition = definition(vec![entry(None)], 60.0, 1.0);
    let error = plan_entries(&definition, &skeleton, &[&clip])
        .err()
        .unwrap();
    assert_eq!(error, BakeError::ClipHasNoBones { entry: 0 });
}

#[test]
fn invalid_ranges_are_errors() {
    let skeleton = skeleton();
    let clip = walk_clip(1.0);
    for range in [(0.5, 0.5), (0.8, 0.2), (-0.1, 0.5), (0.5, 1.5)] {
        let definition = definition(vec![entry(Some(range))], 60.0, 1.0);
        let error = plan_entries(&definition, &skeleton, &[&clip])
            .err()
            .unwrap();
        assert_eq!(error, BakeError::InvalidRange { entry: 0 }, "{range:?}");
    }
}

#[test]
fn a_range_under_two_frames_is_an_error() {
    let skeleton = skeleton();
    let clip = walk_clip(1.0);
    let definition = definition(vec![entry(Some((0.0, 0.02)))], 60.0, 1.0);
    let error = plan_entries(&definition, &skeleton, &[&clip])
        .err()
        .unwrap();
    assert_eq!(
        error,
        BakeError::RangeTooShort {
            entry: 0,
            frames: 1
        }
    );
}

fn body_skeleton() -> Skeleton {
    Skeleton::new(
        vec![Mat4::IDENTITY; 3],
        vec![None, Some(0), Some(1)],
        vec!["Root".into(), "Hips".into(), "Spine2".into()],
        vec![
            JointPose::IDENTITY,
            JointPose {
                translation: Vec3::new(0.0, 1.0, 0.0),
                rotation: Quat::from_rotation_y(std::f32::consts::PI),
                ..JointPose::IDENTITY
            },
            JointPose {
                translation: Vec3::new(0.0, 0.3, 0.0),
                ..JointPose::IDENTITY
            },
        ],
        vec![Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3)],
    )
    .unwrap()
}

fn walk_model_poses(skeleton: &Skeleton, seconds: f32, rate: f32) -> Vec<Vec<Transform>> {
    let clip = walk_clip(seconds);
    let frames = (seconds * rate) as usize;
    (0..frames)
        .map(|frame| model_pose(skeleton, &sample_pose(skeleton, &clip, frame as f32 / rate)))
        .collect()
}

#[test]
fn a_straight_walk_faces_its_travel_and_moves_at_walk_speed() {
    let skeleton = body_skeleton();
    let poses = walk_model_poses(&skeleton, 2.0, 60.0);

    let simulation =
        simulation_bone(&SimulationBoneSettings::default(), &skeleton, &poses).unwrap();

    assert_eq!(simulation.len(), poses.len());
    for (frame, transform) in simulation.iter().enumerate() {
        let facing = transform.rotation * Vec3::Z;
        assert!((facing - Vec3::NEG_Z).length() < 1e-3, "frame {frame}");
        assert!(transform.translation.y.abs() < 1e-6);
    }
    for pair in simulation.windows(2) {
        let speed = (pair[1].translation - pair[0].translation).length() * 60.0;
        assert!((speed - 1.0).abs() < 1e-2, "{speed}");
    }
}

#[test]
fn the_old_root_composes_back_to_its_model_transform() {
    let skeleton = body_skeleton();
    let clip = walk_clip(2.0);
    let local = sample_pose(&skeleton, &clip, 0.75);
    let model = model_pose(&skeleton, &local);
    let simulation =
        Transform::from_translation_rotation(Vec3::new(0.2, 0.0, -0.7), Quat::from_rotation_y(0.4));

    let mut reexpressed = local.clone();
    reexpress_roots(&skeleton, &mut reexpressed, &model, simulation.clone());

    let root = Transform::from_translation_rotation_scale(
        reexpressed[0].translation,
        reexpressed[0].rotation,
        reexpressed[0].scale,
    );
    let composed = simulation * root;
    assert!((composed.translation - model[0].translation).length() < 1e-5);
    assert!(composed.rotation.angle_between(model[0].rotation) < 1e-5);
    assert_eq!(reexpressed[1], local[1]);
    assert_eq!(reexpressed[2], local[2]);
}

#[test]
fn a_missing_joint_is_an_error() {
    let skeleton = body_skeleton();
    let poses = walk_model_poses(&skeleton, 1.0, 60.0);

    let position = SimulationBoneSettings {
        position_joint: "Nope".into(),
        ..Default::default()
    };
    assert_eq!(
        simulation_bone(&position, &skeleton, &poses).err(),
        Some(BakeError::MissingJoint("Nope".into()))
    );

    let facing = SimulationBoneSettings {
        facing_joint: "Gone".into(),
        ..Default::default()
    };
    assert_eq!(
        simulation_bone(&facing, &skeleton, &poses).err(),
        Some(BakeError::MissingJoint("Gone".into()))
    );
}

#[test]
fn smoothing_windows_must_be_odd_and_greater_than_three() {
    let skeleton = body_skeleton();
    let poses = walk_model_poses(&skeleton, 1.0, 60.0);

    for window in [0, 3, 4, 32] {
        let settings = SimulationBoneSettings {
            position_window: window,
            ..Default::default()
        };
        assert_eq!(
            simulation_bone(&settings, &skeleton, &poses).err(),
            Some(BakeError::InvalidWindow {
                name: "position_window",
                window
            })
        );
        let settings = SimulationBoneSettings {
            facing_window: window,
            ..Default::default()
        };
        assert_eq!(
            simulation_bone(&settings, &skeleton, &poses).err(),
            Some(BakeError::InvalidWindow {
                name: "facing_window",
                window
            })
        );
    }
}

#[test]
fn a_window_longer_than_the_range_is_clamped() {
    let skeleton = body_skeleton();
    let poses = walk_model_poses(&skeleton, 0.1, 60.0);
    assert_eq!(poses.len(), 6);

    let simulation =
        simulation_bone(&SimulationBoneSettings::default(), &skeleton, &poses).unwrap();

    assert_eq!(simulation.len(), 6);
    assert!((simulation[3].rotation * Vec3::Z - Vec3::NEG_Z).length() < 1e-3);
}

#[test]
fn a_zero_length_facing_vector_holds_the_previous_yaw() {
    let skeleton = body_skeleton();
    let poses = walk_model_poses(&skeleton, 1.0, 60.0);
    let settings = SimulationBoneSettings {
        facing_axis: Vec3::Y,
        ..Default::default()
    };

    let simulation = simulation_bone(&settings, &skeleton, &poses).unwrap();

    for transform in &simulation {
        assert!(transform.rotation.angle_between(Quat::IDENTITY) < 1e-5);
    }
}

fn moving_locals(frames: usize, step: Vec3, turn: f32) -> Vec<Vec<JointPose>> {
    (0..frames)
        .map(|frame| {
            vec![
                JointPose {
                    translation: step * frame as f32,
                    rotation: Quat::from_rotation_y(turn * frame as f32),
                    ..JointPose::IDENTITY
                },
                JointPose::IDENTITY,
            ]
        })
        .collect()
}

#[test]
fn model_velocities_follow_the_parent_chain() {
    let skeleton = skeleton();
    let model: Vec<Vec<Transform>> = moving_locals(4, Vec3::new(1.0, 0.0, 0.0), 0.0)
        .iter()
        .map(|local| model_pose(&skeleton, local))
        .collect();
    let velocities = model_velocities(&model, 2.0);
    for frame in &velocities {
        assert!(frame[1].distance(Vec3::new(2.0, 0.0, 0.0)) < 1e-4);
    }
}

#[derive(Reflect, Default, Clone, Debug, PartialEq)]
#[reflect(FeatureChannel)]
struct ProbeChannel {
    tag: f32,
}

impl FeatureChannel for ProbeChannel {
    fn dims(&self) -> usize {
        3
    }

    fn extract(&self, frame: &FrameContext, out: &mut [f32]) {
        out[0] = self.tag;
        out[1] = frame.model_pose.len() as f32;
        out[2] = frame.trajectory.len() as f32;
    }
}

#[derive(Reflect, Default, Clone, Debug, PartialEq)]
#[reflect(FeatureChannel)]
struct PoseChannel {
    unused: f32,
}

impl FeatureChannel for PoseChannel {
    fn dims(&self) -> usize {
        10
    }

    fn extract(&self, frame: &FrameContext, out: &mut [f32]) {
        out[0..3].copy_from_slice(&frame.local_pose[0].translation.to_array());
        out[3..7].copy_from_slice(&frame.local_pose[0].rotation.to_array());
        out[7..10].copy_from_slice(&frame.local_pose[1].translation.to_array());
    }
}

#[derive(Reflect, Default, Clone, Debug, PartialEq)]
#[reflect(FeatureChannel)]
struct PairChannel {
    gain: f32,
}

impl FeatureChannel for PairChannel {
    fn dims(&self) -> usize {
        2
    }

    fn extract(&self, frame: &FrameContext, out: &mut [f32]) {
        out[0] = frame.simulation.translation.z;
        out[1] = frame.model_velocities.len() as f32;
    }
}

fn probe_registry() -> TypeRegistry {
    let mut registry = TypeRegistry::default();
    registry.register::<ProbeChannel>();
    registry.register::<PairChannel>();
    registry.register::<PoseChannel>();
    registry
}

fn probe(tag: f32) -> StoredChannel {
    StoredChannel {
        type_name: "ProbeChannel".into(),
        value: to_value(&ProbeChannel { tag }).unwrap(),
    }
}

fn pose_channel() -> StoredChannel {
    StoredChannel {
        type_name: "PoseChannel".into(),
        value: to_value(&PoseChannel { unused: 0.0 }).unwrap(),
    }
}

fn pair() -> StoredChannel {
    StoredChannel {
        type_name: "PairChannel".into(),
        value: to_value(&PairChannel { gain: 1.0 }).unwrap(),
    }
}

fn baking_definition(
    entries: Vec<MotionEntry>,
    channels: Vec<StoredChannel>,
) -> MotionDatabaseDefinition {
    MotionDatabaseDefinition {
        entries,
        rate: 30.0,
        speed: 1.0,
        simulation: SimulationBoneSettings {
            position_window: 5,
            facing_window: 5,
            ..Default::default()
        },
        channels,
        ..Default::default()
    }
}

#[test]
fn a_channel_fills_its_span_for_every_frame() {
    let skeleton = body_skeleton();
    let clip = walk_clip(1.0);
    let definition = baking_definition(vec![entry(None)], vec![probe(7.0)]);

    let database = bake(&definition, &skeleton, &[&clip], &probe_registry()).unwrap();

    assert_eq!(database.feature_dims(), 3);
    assert_eq!(database.frame_count(), 30);
    for frame in 0..database.frame_count() {
        assert_eq!(database.features(frame), [7.0, 3.0, 3.0]);
    }
}

#[test]
fn channels_are_laid_out_in_list_order() {
    let skeleton = body_skeleton();
    let clip = walk_clip(1.0);
    let definition = baking_definition(vec![entry(None)], vec![pair(), probe(1.0)]);

    let database = bake(&definition, &skeleton, &[&clip], &probe_registry()).unwrap();

    assert_eq!(database.feature_dims(), 5);
    assert_eq!(database.channels().len(), 2);
    assert_eq!(database.channels()[0].type_name, "PairChannel");
    assert_eq!(
        (database.channels()[0].offset, database.channels()[0].dims),
        (0, 2)
    );
    assert_eq!(database.channels()[1].type_name, "ProbeChannel");
    assert_eq!(
        (database.channels()[1].offset, database.channels()[1].dims),
        (2, 3)
    );
    let features = database.features(4);
    assert_eq!(features[1], 3.0);
    assert_eq!(features[2], 1.0);
    assert_eq!(features[3], 3.0);
    assert_eq!(features[4], 3.0);
}

#[test]
fn the_trajectory_settings_are_copied_from_the_definition() {
    let skeleton = body_skeleton();
    let clip = walk_clip(1.0);
    let mut definition = baking_definition(vec![entry(None)], Vec::new());
    definition.trajectory = TrajectorySettings {
        sample_dt: 0.25,
        sample_count: 4,
    };

    let database = bake(&definition, &skeleton, &[&clip], &probe_registry()).unwrap();

    assert_eq!(database.trajectory(), &definition.trajectory);
}

#[test]
fn a_sample_dt_that_is_not_positive_fails_the_bake() {
    let skeleton = body_skeleton();
    for sample_dt in [0.0, -0.1, f32::NAN] {
        let mut definition = baking_definition(Vec::new(), Vec::new());
        definition.trajectory.sample_dt = sample_dt;
        let error = bake(&definition, &skeleton, &[], &probe_registry())
            .err()
            .unwrap();
        assert!(matches!(error, BakeError::InvalidSampleDt(_)));
    }
}

#[test]
fn no_channels_gives_zero_dims() {
    let skeleton = body_skeleton();
    let clip = walk_clip(1.0);
    let definition = baking_definition(vec![entry(None)], Vec::new());

    let database = bake(&definition, &skeleton, &[&clip], &probe_registry()).unwrap();

    assert_eq!(database.feature_dims(), 0);
    assert!(database.features(0).is_empty());
    assert!(database.channels().is_empty());
    assert_eq!(database.features(3), &[] as &[f32]);
}

#[test]
fn an_unregistered_channel_fails_the_bake() {
    let skeleton = body_skeleton();
    let clip = walk_clip(1.0);
    let definition = baking_definition(vec![entry(None)], vec![probe(1.0)]);

    let error = bake(&definition, &skeleton, &[&clip], &TypeRegistry::default())
        .err()
        .unwrap();

    assert_eq!(error, BakeError::UnregisteredChannel("ProbeChannel".into()));
}

#[test]
fn an_end_to_end_bake_lays_out_bones_ranges_and_channels() {
    let skeleton = body_skeleton();
    let first = walk_clip(1.0);
    let second = walk_clip(2.0);
    let definition = baking_definition(
        vec![entry(None), entry(Some((0.5, 1.5)))],
        vec![probe(2.0), pair()],
    );

    let database = bake(
        &definition,
        &skeleton,
        &[&first, &second],
        &probe_registry(),
    )
    .unwrap();

    assert_eq!(database.bone_count(), 3);
    assert_eq!(database.frame_count(), 60);
    assert_eq!(database.rate(), 30.0);
    assert_eq!(database.clips().len(), 2);
    assert_eq!(database.entry_of(29), Some(0));
    assert_eq!(database.entry_of(30), Some(1));
    assert_eq!(database.range_of(30), Some(30..60));
    assert_eq!(database.clip_time(30), Some((1, 0.5)));
    assert_eq!(database.feature_dims(), 5);
    assert_eq!(database.features(31)[1], 3.0);
    assert_eq!(database.features(31)[2], 3.0);

    let simulation = database.simulation(10);
    assert!(simulation.translation.y.abs() < 1e-6);
    let facing = simulation.rotation * Vec3::Z;
    assert!((facing - Vec3::NEG_Z).length() < 1e-3);
}

#[test]
fn simulation_settings_are_checked_even_without_entries() {
    let skeleton = body_skeleton();
    let registry = probe_registry();

    let mut missing = definition(Vec::new(), 60.0, 1.0);
    missing.simulation.position_joint = "Nope".into();
    assert_eq!(
        bake(&missing, &skeleton, &[], &registry).err(),
        Some(BakeError::MissingJoint("Nope".into()))
    );

    let mut even = definition(Vec::new(), 60.0, 1.0);
    even.simulation.facing_window = 4;
    assert_eq!(
        bake(&even, &skeleton, &[], &registry).err(),
        Some(BakeError::InvalidWindow {
            name: "facing_window",
            window: 4
        })
    );
}

fn turning_clip(duration: f32) -> AnimationClip {
    let mut clip = walk_clip(duration);
    clip.add_channel(
        Uuid::from_u128(2),
        AnimationChannel::new(
            vec![0.0, duration],
            AnimationChanelOutput::Rotation(vec![Quat::IDENTITY, Quat::from_rotation_y(1.0)]),
        ),
    );
    clip
}

#[test]
fn a_frames_pose_equals_the_pose_the_bake_gave_the_channels() {
    let skeleton = body_skeleton();
    let first = walk_clip(1.0);
    let second = turning_clip(2.0);
    let mut definition = baking_definition(
        vec![entry(None), entry(Some((0.5, 1.5)))],
        vec![pose_channel()],
    );
    definition.speed = 1.5;
    let clips = [&first, &second];

    let database = bake(&definition, &skeleton, &clips, &probe_registry()).unwrap();

    assert!(database.frame_count() >= 40);
    for frame in 0..database.frame_count() {
        let pose = database.pose(frame, &skeleton, &clips).unwrap();
        let features = database.features(frame);
        assert_eq!(pose.len(), 3);
        assert_eq!(pose[0].translation.to_array(), features[0..3]);
        assert_eq!(pose[0].rotation.to_array(), features[3..7]);
        assert_eq!(pose[1].translation.to_array(), features[7..10]);
    }
}

#[test]
fn a_pose_is_expressed_relative_to_the_simulation_bone() {
    let skeleton = body_skeleton();
    let clip = walk_clip(2.0);
    let definition = baking_definition(vec![entry(None)], Vec::new());
    let database = bake(&definition, &skeleton, &[&clip], &probe_registry()).unwrap();

    let pose = database.pose(20, &skeleton, &[&clip]).unwrap();

    let (_, time) = database.clip_time(20).unwrap();
    let model = model_pose(&skeleton, &sample_pose(&skeleton, &clip, time));
    let root = Transform::from_translation_rotation_scale(
        pose[0].translation,
        pose[0].rotation,
        pose[0].scale,
    );
    let composed = database.simulation(20) * root;
    assert!((composed.translation - model[0].translation).length() < 1e-5);
    assert!(composed.rotation.angle_between(model[0].rotation) < 1e-5);
    assert_eq!(pose[1], skeleton.rest_pose[1]);
}

#[test]
fn clip_time_follows_speed_and_rate() {
    let skeleton = body_skeleton();
    let first = walk_clip(1.0);
    let second = walk_clip(2.0);
    let mut definition = baking_definition(vec![entry(None), entry(Some((0.5, 1.5)))], Vec::new());
    definition.speed = 2.0;

    let database = bake(
        &definition,
        &skeleton,
        &[&first, &second],
        &probe_registry(),
    )
    .unwrap();

    assert_eq!(database.frame_count(), 15 + 15);
    assert_eq!(database.clip_time(0), Some((0, 0.0)));
    assert_eq!(database.clip_time(3), Some((0, 0.2)));
    assert_eq!(database.clip_time(15), Some((1, 0.5)));
    let (entry, time) = database.clip_time(18).unwrap();
    assert_eq!(entry, 1);
    assert!((time - 0.7).abs() < 1e-5);
    assert_eq!(database.clip_time(30), None);
}

#[test]
fn a_skeleton_with_another_bone_count_is_an_error() {
    let database_skeleton = body_skeleton();
    let clip = walk_clip(1.0);
    let definition = baking_definition(vec![entry(None)], Vec::new());
    let database = bake(&definition, &database_skeleton, &[&clip], &probe_registry()).unwrap();

    assert_eq!(
        database.pose(0, &skeleton(), &[&clip]).err(),
        Some(BakeError::BoneCountMismatch {
            expected: 3,
            actual: 2
        })
    );
}

#[test]
fn pose_rejects_a_wrong_clip_count_and_an_unknown_frame() {
    let skeleton = body_skeleton();
    let clip = walk_clip(1.0);
    let definition = baking_definition(vec![entry(None)], Vec::new());
    let database = bake(&definition, &skeleton, &[&clip], &probe_registry()).unwrap();

    assert_eq!(
        database.pose(0, &skeleton, &[&clip, &clip]).err(),
        Some(BakeError::ClipCountMismatch {
            expected: 1,
            actual: 2
        })
    );
    assert_eq!(
        database.pose(30, &skeleton, &[&clip]).err(),
        Some(BakeError::FrameOutOfRange {
            frame: 30,
            frame_count: 30
        })
    );
}

const SAMPLES: usize = 3;
const BODY_DIMS: usize = 6 + 7 + SAMPLES * 10;

#[derive(Reflect, Default, Clone, Debug, PartialEq)]
#[reflect(FeatureChannel)]
struct BodyChannel {
    unused: f32,
}

impl FeatureChannel for BodyChannel {
    fn dims(&self) -> usize {
        BODY_DIMS
    }

    fn extract(&self, frame: &FrameContext, out: &mut [f32]) {
        out[0..3].copy_from_slice(&frame.body.velocity.to_array());
        out[3..6].copy_from_slice(&frame.body.angular_velocity.to_array());
        out[6..9].copy_from_slice(&frame.simulation.translation.to_array());
        out[9..13].copy_from_slice(&frame.simulation.rotation.to_array());
        for (index, sample) in frame.trajectory.iter().enumerate() {
            let at = 13 + index * 10;
            out[at..at + 3].copy_from_slice(&sample.position.to_array());
            out[at + 3..at + 6].copy_from_slice(&sample.velocity.to_array());
            out[at + 6..at + 10].copy_from_slice(&sample.rotation.to_array());
        }
    }
}

const LOCAL_DIMS: usize = 6 + SAMPLES * 10;

#[derive(Reflect, Default, Clone, Debug, PartialEq)]
#[reflect(FeatureChannel)]
struct LocalChannel {
    unused: f32,
}

impl FeatureChannel for LocalChannel {
    fn dims(&self) -> usize {
        LOCAL_DIMS
    }

    fn extract(&self, frame: &FrameContext, out: &mut [f32]) {
        let inverse = frame.simulation.rotation.inverse();
        let origin = frame.simulation.translation;
        out[0..3].copy_from_slice(&(inverse * frame.body.velocity).to_array());
        out[3..6].copy_from_slice(&(inverse * frame.body.angular_velocity).to_array());
        for (index, sample) in frame.trajectory.iter().enumerate() {
            let at = 6 + index * 10;
            out[at..at + 3].copy_from_slice(&(inverse * (sample.position - origin)).to_array());
            out[at + 3..at + 6].copy_from_slice(&(inverse * sample.velocity).to_array());
            out[at + 6..at + 10].copy_from_slice(&(inverse * sample.rotation).to_array());
        }
    }
}

fn body_registry() -> TypeRegistry {
    let mut registry = probe_registry();
    registry.register::<BodyChannel>();
    registry.register::<LocalChannel>();
    registry
}

fn body_channel() -> StoredChannel {
    StoredChannel {
        type_name: "BodyChannel".into(),
        value: to_value(&BodyChannel { unused: 0.0 }).unwrap(),
    }
}

fn local_channel() -> StoredChannel {
    StoredChannel {
        type_name: "LocalChannel".into(),
        value: to_value(&LocalChannel { unused: 0.0 }).unwrap(),
    }
}

fn vec3(features: &[f32], at: usize) -> Vec3 {
    Vec3::from_slice(&features[at..at + 3])
}

fn quat(features: &[f32], at: usize) -> Quat {
    Quat::from_slice(&features[at..at + 4])
}

fn trajectory_definition(
    entries: Vec<MotionEntry>,
    speed: f32,
    channels: Vec<StoredChannel>,
) -> MotionDatabaseDefinition {
    let mut definition = baking_definition(entries, channels);
    definition.speed = speed;
    definition.trajectory = TrajectorySettings {
        sample_dt: 0.2,
        sample_count: SAMPLES,
    };
    definition
}

#[test]
fn a_straight_walk_has_trajectory_samples_ahead_along_the_travel() {
    let skeleton = body_skeleton();
    let clip = walk_clip(3.0);
    for speed in [1.0, 2.0] {
        let definition = trajectory_definition(vec![entry(None)], speed, vec![body_channel()]);
        let database = bake(&definition, &skeleton, &[&clip], &body_registry()).unwrap();

        let features = database.features(5);
        assert!((vec3(features, 0) - Vec3::new(0.0, 0.0, -speed)).length() < 1e-3);
        assert!(vec3(features, 3).length() < 1e-3);
        let origin = vec3(features, 6);
        for k in 0..SAMPLES {
            let at = 13 + k * 10;
            let ahead = speed * 0.2 * (k + 1) as f32;
            let position = vec3(features, at) - origin;
            assert!(
                (position - Vec3::new(0.0, 0.0, -ahead)).length() < 1e-3,
                "{k}"
            );
            assert!((vec3(features, at + 3) - Vec3::new(0.0, 0.0, -speed)).length() < 1e-3);
            let facing = quat(features, at + 6) * Vec3::Z;
            assert!((facing - Vec3::NEG_Z).length() < 1e-3);
        }
    }
}

#[test]
fn a_turning_walk_has_the_turn_rate_as_angular_velocity() {
    let skeleton = body_skeleton();
    let clip = turning_clip(2.0);
    let definition = trajectory_definition(vec![entry(None)], 1.0, vec![body_channel()]);
    let database = bake(&definition, &skeleton, &[&clip], &body_registry()).unwrap();

    for frame in [0, 7, 30, database.frame_count() - 1] {
        let angular = vec3(database.features(frame), 3);
        assert!(angular.x.abs() < 1e-3 && angular.z.abs() < 1e-3, "{frame}");
        assert!((angular.y.abs() - 0.5).abs() < 1e-2, "{frame}: {angular:?}");
    }
}

#[test]
fn a_sample_past_the_range_end_equals_the_last_simulation_frame() {
    let skeleton = body_skeleton();
    let clip = walk_clip(3.0);
    let definition = trajectory_definition(
        vec![entry(Some((0.0, 1.0))), entry(Some((1.0, 3.0)))],
        1.0,
        vec![body_channel()],
    );
    let database = bake(&definition, &skeleton, &[&clip, &clip], &body_registry()).unwrap();

    let range = database.range_of(0).unwrap();
    let last = database.simulation(range.end - 1);
    let features = database.features(range.end - 8);
    let end = 13 + (SAMPLES - 1) * 10;
    assert_eq!(vec3(features, end), last.translation);
    assert_eq!(quat(features, end + 6), last.rotation);
    assert_eq!(vec3(features, 13 + 10), last.translation);
    assert!(vec3(features, 13).distance(last.translation) > 0.0);
    let in_range = database.features(range.start);
    assert!(vec3(in_range, 13).distance(database.simulation(range.start).translation) > 0.1);
}

#[test]
fn offline_features_equal_online_features_for_a_matching_simulation() {
    let skeleton = body_skeleton();
    let clip = turning_clip(3.0);
    let definition = trajectory_definition(
        vec![entry(None)],
        1.0,
        vec![body_channel(), local_channel()],
    );
    let database = bake(&definition, &skeleton, &[&clip], &body_registry()).unwrap();
    let local_at = BODY_DIMS;

    let yaw = Quat::from_rotation_y(1.2);
    let offset = Vec3::new(4.0, 0.5, -7.0);
    let place = |point: Vec3| yaw * point + offset;

    for frame in [0, 20, 50] {
        let features = database.features(frame);
        let mut simulation = CharacterSimulation {
            state: CharacterState {
                position: place(vec3(features, 6)),
                velocity: yaw * vec3(features, 0),
                ..Default::default()
            },
            rotation: yaw * quat(features, 9),
            angular_velocity: yaw * vec3(features, 3),
            ..Default::default()
        };
        simulation.trajectory = (0..SAMPLES)
            .map(|k| {
                let at = 13 + k * 10;
                TrajectorySample {
                    position: place(vec3(features, at)),
                    velocity: yaw * vec3(features, at + 3),
                    rotation: yaw * quat(features, at + 6),
                }
            })
            .collect();
        let context = FrameContext {
            skeleton: &skeleton,
            local_pose: &[],
            model_pose: &[],
            model_velocities: &[],
            simulation: Transform::from_translation_rotation(
                simulation.state.position,
                simulation.rotation,
            ),
            body: BodyMotion {
                velocity: simulation.state.velocity,
                angular_velocity: simulation.angular_velocity,
            },
            trajectory: &simulation.trajectory,
        };
        let mut online = [0.0; LOCAL_DIMS];
        LocalChannel::default().extract(&context, &mut online);

        let offline = &features[local_at..local_at + LOCAL_DIMS];
        assert!(offline.iter().any(|value| value.abs() > 0.1));
        for (index, (a, b)) in offline.iter().zip(&online).enumerate() {
            assert!(
                (a - b).abs() < 1e-5,
                "frame {frame} value {index}: {a} vs {b}"
            );
        }
    }
}

#[derive(Reflect, Clone, Debug, PartialEq)]
#[reflect(FeatureChannel)]
struct WeightedChannel {
    weight: f32,
    value: f32,
}

impl Default for WeightedChannel {
    fn default() -> Self {
        Self {
            weight: 1.0,
            value: 0.0,
        }
    }
}

impl FeatureChannel for WeightedChannel {
    fn dims(&self) -> usize {
        1
    }

    fn weight(&self) -> f32 {
        self.weight
    }

    fn source(&self) -> ChannelSource {
        ChannelSource::Trajectory
    }

    fn extract(&self, _frame: &FrameContext, out: &mut [f32]) {
        out[0] = self.value;
    }
}

#[derive(Reflect, Default, Clone, Debug, PartialEq)]
#[reflect(FeatureChannel)]
struct RejectingChannel {
    reason: String,
}

impl FeatureChannel for RejectingChannel {
    fn dims(&self) -> usize {
        1
    }

    fn validate(
        &self,
        _skeleton: &Skeleton,
        _trajectory: &TrajectorySettings,
    ) -> Result<(), String> {
        Err(self.reason.clone())
    }

    fn extract(&self, _frame: &FrameContext, out: &mut [f32]) {
        out[0] = 0.0;
    }
}

fn weighted_registry() -> TypeRegistry {
    let mut registry = probe_registry();
    registry.register::<WeightedChannel>();
    registry.register::<RejectingChannel>();
    registry
}

fn weighted(weight: f32, value: f32) -> StoredChannel {
    StoredChannel {
        type_name: "WeightedChannel".into(),
        value: to_value(&WeightedChannel { weight, value }).unwrap(),
    }
}

#[test]
fn a_default_channel_weighs_one_and_reads_the_pose() {
    let channel = ProbeChannel::default();
    assert_eq!(channel.weight(), 1.0);
    assert_eq!(channel.source(), ChannelSource::Pose);
    assert_eq!(
        channel.validate(&body_skeleton(), &TrajectorySettings::default()),
        Ok(())
    );
}

#[test]
fn spans_carry_the_weight_source_and_value_of_their_channel() {
    let skeleton = body_skeleton();
    let clip = walk_clip(1.0);
    let definition = baking_definition(vec![entry(None)], vec![probe(7.0), weighted(2.5, 4.0)]);

    let database = bake(&definition, &skeleton, &[&clip], &weighted_registry()).unwrap();

    let spans = database.channels();
    assert_eq!(spans[0].weight, 1.0);
    assert_eq!(spans[0].source, ChannelSource::Pose);
    assert_eq!(spans[0].value, definition.channels[0].value);
    assert_eq!(spans[1].weight, 2.5);
    assert_eq!(spans[1].source, ChannelSource::Trajectory);
    assert_eq!(spans[1].value, definition.channels[1].value);
}

#[test]
fn a_non_positive_or_non_finite_weight_is_rejected_before_extracting() {
    let skeleton = body_skeleton();
    let clip = walk_clip(1.0);
    for weight in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        let definition = baking_definition(vec![entry(None)], vec![weighted(weight, 0.0)]);

        let error = bake(&definition, &skeleton, &[&clip], &weighted_registry())
            .err()
            .unwrap();

        match error {
            BakeError::InvalidWeight(rejected) => {
                assert!(rejected == weight || (rejected.is_nan() && weight.is_nan()))
            }
            other => panic!("expected InvalidWeight, got {other:?}"),
        }
    }
}

#[test]
fn a_channel_that_fails_validation_is_a_channel_value_error() {
    let skeleton = body_skeleton();
    let clip = walk_clip(1.0);
    let rejecting = StoredChannel {
        type_name: "RejectingChannel".into(),
        value: to_value(&RejectingChannel {
            reason: "no such bone".into(),
        })
        .unwrap(),
    };
    let definition = baking_definition(vec![entry(None)], vec![rejecting]);

    let error = bake(&definition, &skeleton, &[&clip], &weighted_registry())
        .err()
        .unwrap();

    assert_eq!(
        error,
        BakeError::ChannelValue {
            type_name: "RejectingChannel".into(),
            reason: "no such bone".into(),
        }
    );
}
