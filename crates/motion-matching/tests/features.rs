use concerto_animation::clip::{AnimationChanelOutput, AnimationChannel, AnimationClip};
use concerto_foundation::transform::Transform;
use concerto_mesh::{JointPose, Skeleton};
use concerto_motion_matching::bake::{bake, trajectory_samples};
use concerto_motion_matching::{
    reference_channels, BakeError, BodyMotion, BonePositions, BoneVelocities, ChannelSource,
    FeatureChannel, FeatureQuery, FrameContext, MotionDatabase, MotionDatabaseDefinition,
    MotionEntry, ReflectFeatureChannel, SimulationBoneSettings, StoredChannel,
    TrajectoryDirections, TrajectoryPositions, TrajectorySample,
};
use concerto_reflect::{to_value, Reflect, TypeRegistry};
use glam::{Mat4, Quat, Vec3};
use uuid::Uuid;

#[derive(Reflect, Default, Clone, Debug, PartialEq)]
#[reflect(FeatureChannel)]
struct RampChannel {
    weight: f32,
}

impl FeatureChannel for RampChannel {
    fn dims(&self) -> usize {
        2
    }

    fn extract(&self, frame: &FrameContext, out: &mut [f32]) {
        out[0] = frame.simulation.translation.z;
        out[1] = 3.0 * frame.simulation.translation.z + 1.0;
    }

    fn weight(&self) -> f32 {
        self.weight
    }
}

#[derive(Reflect, Default, Clone, Debug, PartialEq)]
#[reflect(FeatureChannel)]
struct FlatChannel {
    weight: f32,
}

impl FeatureChannel for FlatChannel {
    fn dims(&self) -> usize {
        1
    }

    fn extract(&self, _frame: &FrameContext, out: &mut [f32]) {
        out[0] = 0.1;
    }

    fn weight(&self) -> f32 {
        self.weight
    }
}

fn registry() -> TypeRegistry {
    let mut registry = TypeRegistry::default();
    registry.register::<RampChannel>();
    registry.register::<FlatChannel>();
    registry
}

fn skeleton() -> Skeleton {
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

fn ramp(weight: f32) -> StoredChannel {
    StoredChannel {
        type_name: "RampChannel".into(),
        value: to_value(&RampChannel { weight }).unwrap(),
    }
}

fn flat(weight: f32) -> StoredChannel {
    StoredChannel {
        type_name: "FlatChannel".into(),
        value: to_value(&FlatChannel { weight }).unwrap(),
    }
}

fn baked(channels: Vec<StoredChannel>) -> MotionDatabase {
    let definition = MotionDatabaseDefinition {
        entries: vec![MotionEntry {
            clip: Default::default(),
            range: None,
        }],
        rate: 30.0,
        speed: 1.0,
        simulation: SimulationBoneSettings {
            position_window: 5,
            facing_window: 5,
            ..Default::default()
        },
        channels,
        ..Default::default()
    };
    bake(&definition, &skeleton(), &[&walk_clip(1.0)], &registry()).unwrap()
}

fn group_deviation(database: &MotionDatabase, channel: usize) -> f32 {
    let span = &database.channels()[channel];
    let count = database.frame_count() as f32;
    let mut total = 0.0;
    for dim in span.offset..span.offset + span.dims {
        let values: Vec<f32> = (0..database.frame_count())
            .map(|frame| database.normalised_features(frame)[dim])
            .collect();
        let mean = values.iter().sum::<f32>() / count;
        total += (values.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / count).sqrt();
    }
    total / span.dims as f32
}

#[test]
fn normalised_dimensions_have_mean_zero_and_group_deviation_equal_to_the_weight() {
    let database = baked(vec![ramp(2.0), flat(1.0)]);

    for dim in 0..2 {
        let mean = (0..database.frame_count())
            .map(|frame| database.normalised_features(frame)[dim])
            .sum::<f32>()
            / database.frame_count() as f32;
        assert!(mean.abs() < 1e-4, "dimension {dim} mean {mean}");
    }
    assert!((group_deviation(&database, 0) - 2.0).abs() < 1e-3);
}

#[test]
fn a_constant_group_normalises_to_exactly_zero() {
    let database = baked(vec![ramp(1.0), flat(1.0)]);

    for frame in 0..database.frame_count() {
        let normalised = database.normalised_features(frame);
        assert_eq!(normalised[2], 0.0);
        assert!(normalised.iter().all(|value| value.is_finite()));
    }
}

#[test]
fn set_weights_scales_one_group_by_the_weight_ratio() {
    let mut database = baked(vec![ramp(1.0), flat(1.0)]);
    let before = database.scales();
    let features = database.features(5).to_vec();
    let offset = database.offset().to_vec();

    database.set_weights(&[4.0, 1.0]).unwrap();

    let after = database.scales();
    assert!((after[0] - before[0] / 4.0).abs() < before[0] * 1e-5);
    assert!((after[1] - before[1] / 4.0).abs() < before[1] * 1e-5);
    assert_eq!(after[2], before[2]);
    assert_eq!(database.features(5), features);
    assert_eq!(database.offset(), offset);
    assert_eq!(database.channels()[0].weight, 4.0);
}

#[test]
fn set_weights_rejects_bad_input() {
    let mut database = baked(vec![ramp(1.0), flat(1.0)]);

    assert_eq!(
        database.set_weights(&[1.0]),
        Err(BakeError::WeightCountMismatch {
            expected: 2,
            actual: 1
        })
    );
    for weight in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        let error = database.set_weights(&[1.0, weight]).unwrap_err();
        assert!(matches!(error, BakeError::InvalidWeight(_)), "{weight}");
    }
    assert_eq!(database.channels()[0].weight, 1.0);
}

#[test]
fn features_still_returns_raw_values() {
    let database = baked(vec![flat(1.0)]);

    assert_eq!(database.features(0), [0.1]);
    assert_eq!(database.offset(), [0.1]);
}

fn yawed_simulation() -> Transform {
    Transform::from_translation_rotation(
        Vec3::new(1.0, 0.0, 2.0),
        Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
    )
}

fn extract_with(channel: &dyn FeatureChannel, trajectory: &[TrajectorySample]) -> Vec<f32> {
    let skeleton = skeleton();
    let models = vec![
        Transform::default(),
        Transform::from_translation(Vec3::new(4.0, 5.0, 7.0)),
        Transform::default(),
    ];
    let velocities = vec![Vec3::ZERO, Vec3::new(0.0, 0.0, 2.0), Vec3::ZERO];
    let locals = vec![JointPose::IDENTITY; 3];
    let frame = FrameContext {
        skeleton: &skeleton,
        local_pose: &locals,
        model_pose: &models,
        model_velocities: &velocities,
        simulation: yawed_simulation(),
        body: BodyMotion::default(),
        trajectory,
    };
    let mut out = vec![0.0; channel.dims()];
    channel.extract(&frame, &mut out);
    out
}

fn assert_close(actual: &[f32], expected: &[f32]) {
    assert_eq!(actual.len(), expected.len());
    for (a, e) in actual.iter().zip(expected) {
        assert!((a - e).abs() < 1e-5, "{actual:?} != {expected:?}");
    }
}

#[test]
fn bone_positions_are_expressed_in_the_simulation_frame() {
    let channel = BonePositions {
        bones: vec!["Hips".into()],
        weight: 0.75,
    };

    assert_eq!(channel.dims(), 3);
    assert_eq!(channel.source(), ChannelSource::Pose);
    assert_close(&extract_with(&channel, &[]), &[-5.0, 5.0, 3.0]);
}

#[test]
fn bone_velocities_are_rotated_but_not_translated() {
    let channel = BoneVelocities {
        bones: vec!["Hips".into(), "Spine2".into()],
        weight: 1.0,
    };

    assert_eq!(channel.dims(), 6);
    assert_eq!(channel.source(), ChannelSource::Pose);
    assert_close(
        &extract_with(&channel, &[]),
        &[-2.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    );
}

#[test]
fn trajectory_positions_keep_the_ground_plane_in_the_simulation_frame() {
    let channel = TrajectoryPositions {
        samples: vec![1],
        weight: 1.0,
    };
    let trajectory = [
        TrajectorySample::default(),
        TrajectorySample {
            position: Vec3::new(1.0, 3.0, 6.0),
            ..Default::default()
        },
    ];

    assert_eq!(channel.dims(), 2);
    assert_eq!(channel.source(), ChannelSource::Trajectory);
    assert_close(&extract_with(&channel, &trajectory), &[-4.0, 0.0]);
}

#[test]
fn trajectory_directions_are_relative_to_the_simulation_facing() {
    let channel = TrajectoryDirections {
        samples: vec![0, 1],
        weight: 1.5,
    };
    let trajectory = [
        TrajectorySample {
            rotation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
            ..Default::default()
        },
        TrajectorySample {
            rotation: Quat::from_rotation_y(std::f32::consts::PI),
            ..Default::default()
        },
    ];

    assert_eq!(channel.dims(), 4);
    assert_eq!(channel.source(), ChannelSource::Trajectory);
    assert_close(&extract_with(&channel, &trajectory), &[0.0, 1.0, 1.0, 0.0]);
}

#[test]
fn reference_channels_are_27_dimensions_with_the_reference_weights() {
    let mut registry = TypeRegistry::default();
    registry.register::<BonePositions>();
    registry.register::<BoneVelocities>();
    registry.register::<TrajectoryPositions>();
    registry.register::<TrajectoryDirections>();

    let stored = reference_channels();
    let built = concerto_motion_matching::build_channels(&stored, &registry).unwrap();

    let dims: Vec<usize> = built.iter().map(|channel| channel.dims()).collect();
    let weights: Vec<f32> = built.iter().map(|channel| channel.weight()).collect();
    assert_eq!(dims, [6, 6, 3, 6, 6]);
    assert_eq!(dims.iter().sum::<usize>(), 27);
    assert_eq!(weights, [0.75, 1.0, 1.0, 1.0, 1.5]);
    assert_eq!(MotionDatabaseDefinition::default().channels, stored);
}

fn bake_with(channel: StoredChannel) -> Result<MotionDatabase, BakeError> {
    let mut registry = TypeRegistry::default();
    registry.register::<BoneVelocities>();
    registry.register::<TrajectoryPositions>();
    let definition = MotionDatabaseDefinition {
        entries: vec![MotionEntry {
            clip: Default::default(),
            range: None,
        }],
        rate: 30.0,
        speed: 1.0,
        simulation: SimulationBoneSettings {
            position_window: 5,
            facing_window: 5,
            ..Default::default()
        },
        channels: vec![channel],
        ..Default::default()
    };
    bake(&definition, &skeleton(), &[&walk_clip(1.0)], &registry)
}

#[test]
fn a_missing_bone_fails_the_bake_with_a_channel_value_error() {
    let missing = to_value(&BoneVelocities {
        bones: vec!["Nope".into()],
        weight: 1.0,
    })
    .unwrap();

    let error = bake_with(StoredChannel {
        type_name: "BoneVelocities".into(),
        value: missing,
    })
    .unwrap_err();

    assert!(matches!(error, BakeError::ChannelValue { .. }), "{error:?}");
}

#[test]
fn a_sample_index_past_the_trajectory_fails_the_bake_with_a_channel_value_error() {
    for samples in [vec![3], vec![0, 7]] {
        let value = to_value(&TrajectoryPositions {
            samples,
            weight: 1.0,
        })
        .unwrap();

        let error = bake_with(StoredChannel {
            type_name: "TrajectoryPositions".into(),
            value,
        })
        .unwrap_err();

        assert!(matches!(error, BakeError::ChannelValue { .. }), "{error:?}");
    }
}

#[test]
fn valid_concrete_channels_bake() {
    let value = to_value(&TrajectoryPositions {
        samples: vec![0, 2],
        weight: 1.0,
    })
    .unwrap();

    let database = bake_with(StoredChannel {
        type_name: "TrajectoryPositions".into(),
        value,
    })
    .unwrap();

    assert_eq!(database.feature_dims(), 4);
    assert_eq!(database.channels()[0].source, ChannelSource::Trajectory);
}

fn query_registry() -> TypeRegistry {
    let mut registry = registry();
    registry.register::<TrajectoryPositions>();
    registry.register::<TrajectoryDirections>();
    registry
}

fn query_database() -> MotionDatabase {
    let positions = StoredChannel {
        type_name: "TrajectoryPositions".into(),
        value: to_value(&TrajectoryPositions {
            samples: vec![0, 2],
            weight: 1.0,
        })
        .unwrap(),
    };
    let directions = StoredChannel {
        type_name: "TrajectoryDirections".into(),
        value: to_value(&TrajectoryDirections {
            samples: vec![1],
            weight: 1.5,
        })
        .unwrap(),
    };
    let definition = MotionDatabaseDefinition {
        entries: vec![MotionEntry {
            clip: Default::default(),
            range: None,
        }],
        rate: 30.0,
        speed: 1.0,
        simulation: SimulationBoneSettings {
            position_window: 5,
            facing_window: 5,
            ..Default::default()
        },
        channels: vec![ramp(2.0), positions, flat(1.0), directions],
        ..Default::default()
    };
    bake(
        &definition,
        &skeleton(),
        &[&walk_clip(1.0)],
        &query_registry(),
    )
    .unwrap()
}

fn frame_trajectory(database: &MotionDatabase, frame: usize) -> Vec<TrajectorySample> {
    let simulation: Vec<Transform> = (0..database.frame_count())
        .map(|frame| database.simulation(frame))
        .collect();
    let motion = vec![BodyMotion::default(); database.frame_count()];
    trajectory_samples(
        database.trajectory(),
        &simulation,
        &motion,
        database.rate(),
        frame,
    )
}

#[test]
fn a_query_for_a_database_frame_equals_its_normalised_features() {
    let database = query_database();
    let query = FeatureQuery::new(&database, &query_registry()).unwrap();

    for frame in [0, 7, database.frame_count() - 1] {
        let built = query
            .build(
                &database,
                &skeleton(),
                frame,
                &database.simulation(frame),
                &frame_trajectory(&database, frame),
            )
            .unwrap();

        assert_close(&built, &database.normalised_features(frame));
    }
}

#[test]
fn a_query_follows_set_weights() {
    let mut database = query_database();
    let query = FeatureQuery::new(&database, &query_registry()).unwrap();
    database.set_weights(&[1.0, 3.0, 1.0, 0.5]).unwrap();

    let built = query
        .build(
            &database,
            &skeleton(),
            4,
            &database.simulation(4),
            &frame_trajectory(&database, 4),
        )
        .unwrap();

    assert_close(&built, &database.normalised_features(4));
}

#[test]
fn a_query_rejects_a_frame_past_the_database() {
    let database = query_database();
    let query = FeatureQuery::new(&database, &query_registry()).unwrap();
    let frame = database.frame_count();

    let error = query
        .build(
            &database,
            &skeleton(),
            frame,
            &database.simulation(0),
            &frame_trajectory(&database, 0),
        )
        .unwrap_err();

    assert_eq!(
        error,
        BakeError::FrameOutOfRange {
            frame,
            frame_count: frame
        }
    );
}

#[test]
fn a_query_rejects_a_short_trajectory() {
    let database = query_database();
    let query = FeatureQuery::new(&database, &query_registry()).unwrap();
    let needed = database.trajectory().sample_count;
    let mut trajectory = frame_trajectory(&database, 0);
    trajectory.pop();

    let error = query
        .build(
            &database,
            &skeleton(),
            0,
            &database.simulation(0),
            &trajectory,
        )
        .unwrap_err();

    assert_eq!(
        error,
        BakeError::TrajectoryTooShort {
            needed,
            actual: needed - 1
        }
    );
}

#[test]
fn a_query_rejects_a_wrong_bone_count() {
    let database = query_database();
    let query = FeatureQuery::new(&database, &query_registry()).unwrap();
    let small = Skeleton::new(
        vec![Mat4::IDENTITY],
        vec![None],
        vec!["Root".into()],
        vec![JointPose::IDENTITY],
        vec![Uuid::from_u128(1)],
    )
    .unwrap();

    let error = query
        .build(
            &database,
            &small,
            0,
            &database.simulation(0),
            &frame_trajectory(&database, 0),
        )
        .unwrap_err();

    assert_eq!(
        error,
        BakeError::BoneCountMismatch {
            expected: 3,
            actual: 1
        }
    );
}

#[test]
fn a_query_rejects_a_database_with_another_channel_count() {
    let database = query_database();
    let query = FeatureQuery::new(&database, &query_registry()).unwrap();
    let other = baked(vec![ramp(1.0), flat(1.0)]);

    let error = query
        .build(
            &other,
            &skeleton(),
            0,
            &other.simulation(0),
            &frame_trajectory(&other, 0),
        )
        .unwrap_err();

    assert_eq!(
        error,
        BakeError::ChannelCountMismatch {
            expected: 4,
            actual: 2
        }
    );
}

#[test]
fn a_query_needs_every_channel_type_registered() {
    let database = query_database();

    let error = FeatureQuery::new(&database, &TypeRegistry::default())
        .err()
        .unwrap();

    assert!(
        matches!(error, BakeError::UnregisteredChannel(_)),
        "{error:?}"
    );
}
