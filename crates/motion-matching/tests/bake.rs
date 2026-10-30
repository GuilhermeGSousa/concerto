use concerto_animation::clip::{AnimationChanelOutput, AnimationChannel, AnimationClip};
use concerto_foundation::transform::Transform;
use concerto_mesh::{JointPose, Skeleton};
use concerto_motion_matching::bake::{
    bake, contacts, local_velocities, model_pose, model_velocities, plan_entries, reexpress_roots,
    sample_pose, simulation_bone,
};
use concerto_motion_matching::{
    BakeError, ContactSettings, FeatureChannel, FrameContext, MotionDatabaseDefinition,
    MotionEntry, ReflectFeatureChannel, SimulationBoneSettings, StoredChannel, TimeRange,
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
fn constant_velocity_is_recovered_including_at_the_ends() {
    let locals = moving_locals(5, Vec3::new(0.0, 0.0, -0.5), 0.1);
    let (linear, angular) = local_velocities(&locals, 10.0);
    for (linear, angular) in linear.iter().zip(&angular) {
        assert!(linear[0].distance(Vec3::new(0.0, 0.0, -5.0)) < 1e-4);
        assert!(angular[0].distance(Vec3::new(0.0, 1.0, 0.0)) < 1e-4);
        assert_eq!(linear[1], Vec3::ZERO);
        assert_eq!(angular[1], Vec3::ZERO);
    }
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

fn contact_settings(bones: &[&str], window: usize) -> ContactSettings {
    ContactSettings {
        bones: bones.iter().map(|name| name.to_string()).collect(),
        speed_threshold: 0.15,
        median_window: window,
    }
}

#[test]
fn contacts_are_true_at_rest_and_false_when_moving() {
    let skeleton = skeleton();
    let rest = vec![vec![Vec3::ZERO; 2]; 8];
    let moving = vec![vec![Vec3::new(1.0, 0.0, 0.0); 2]; 8];
    let settings = contact_settings(&["Child"], 3);
    assert!(contacts(&settings, &skeleton, &rest)
        .unwrap()
        .iter()
        .all(|frame| frame == &[true]));
    assert!(contacts(&settings, &skeleton, &moving)
        .unwrap()
        .iter()
        .all(|frame| frame == &[false]));
}

#[test]
fn contacts_median_filter_removes_a_one_frame_glitch() {
    let skeleton = skeleton();
    let mut velocities = vec![vec![Vec3::ZERO; 2]; 9];
    velocities[4][1] = Vec3::new(1.0, 0.0, 0.0);
    let settings = contact_settings(&["Child"], 5);
    let result = contacts(&settings, &skeleton, &velocities).unwrap();
    assert!(result.iter().all(|frame| frame == &[true]));
}

#[test]
fn contacts_reject_an_unknown_bone() {
    let skeleton = skeleton();
    let velocities = vec![vec![Vec3::ZERO; 2]; 4];
    let settings = contact_settings(&["Nope"], 3);
    assert!(matches!(
        contacts(&settings, &skeleton, &velocities),
        Err(BakeError::MissingJoint(name)) if name == "Nope"
    ));
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
        out[1] = frame.range_index as f32;
        out[2] = frame.range_simulation.len() as f32;
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
    registry
}

fn probe(tag: f32) -> StoredChannel {
    StoredChannel {
        type_name: "ProbeChannel".into(),
        value: to_value(&ProbeChannel { tag }).unwrap(),
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
        assert_eq!(database.features(frame), [7.0, frame as f32, 30.0]);
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
    assert_eq!(features[3], 4.0);
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
    let mut definition = baking_definition(
        vec![entry(None), entry(Some((0.5, 1.5)))],
        vec![probe(2.0), pair()],
    );
    definition.contacts = ContactSettings {
        bones: vec!["Spine2".into()],
        speed_threshold: 10.0,
        median_window: 5,
    };

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
    assert_eq!(database.contacts(10).len(), 1);
    assert!(database.contacts(10)[0]);
    assert_eq!(database.feature_dims(), 5);
    assert_eq!(database.features(31)[1], 1.0);
    assert_eq!(database.features(31)[2], 30.0);

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
