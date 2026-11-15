use concerto_animation::clip::{AnimationChanelOutput, AnimationChannel, AnimationClip};
use concerto_foundation::transform::Transform;
use concerto_mesh::{JointPose, Skeleton};
use concerto_motion_matching::bake::{bake, trajectory_samples};
use concerto_motion_matching::{
    BakeError, BodyMotion, BonePositions, BoneVelocities, FeatureChannel, FeatureChannelData,
    FeatureQuery, FrameContext, JointAxis, MotionDatabase, MotionEntry, ReflectFeatureChannel,
    SimulationBoneSettings, TrajectoryDirections, TrajectoryPositions, TrajectorySample,
};
use concerto_reflect::Reflect;
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

fn ramp(weight: f32) -> FeatureChannelData {
    FeatureChannelData::live(RampChannel { weight })
}

fn flat(weight: f32) -> FeatureChannelData {
    FeatureChannelData::live(FlatChannel { weight })
}

fn definition_of(channels: Vec<FeatureChannelData>) -> MotionDatabase {
    MotionDatabase {
        entries: vec![MotionEntry {
            clip: Default::default(),
            range: None,
            mirrored: false,
        }],
        rate: 30.0,
        speed: 1.0,
        simulation: SimulationBoneSettings {
            facing_axis: JointAxis::Z,
            position_window: 5,
            facing_window: 5,
            ..Default::default()
        },
        channels,
        ..Default::default()
    }
}

fn with_bake(definition: MotionDatabase) -> Result<MotionDatabase, BakeError> {
    let baked = bake(&definition, &skeleton(), &[&walk_clip(1.0)])?;
    Ok(definition.with_baked(baked))
}

fn baked(channels: Vec<FeatureChannelData>) -> MotionDatabase {
    with_bake(definition_of(channels)).unwrap()
}

fn group_deviation(database: &MotionDatabase, channel: usize) -> f32 {
    let span = database.channel_range(channel).unwrap();
    let count = database.frame_count().unwrap() as f32;
    let mut total = 0.0;
    for dim in span.clone() {
        let values: Vec<f32> = (0..database.frame_count().unwrap())
            .map(|frame| database.normalised_features(frame).unwrap()[dim])
            .collect();
        let mean = values.iter().sum::<f32>() / count;
        total += (values.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / count).sqrt();
    }
    total / span.len() as f32
}

#[test]
fn normalised_dimensions_have_mean_zero_and_group_deviation_equal_to_the_weight() {
    let database = baked(vec![ramp(2.0), flat(1.0)]);

    for dim in 0..2 {
        let mean = (0..database.frame_count().unwrap())
            .map(|frame| database.normalised_features(frame).unwrap()[dim])
            .sum::<f32>()
            / database.frame_count().unwrap() as f32;
        assert!(mean.abs() < 1e-4, "dimension {dim} mean {mean}");
    }
    assert!((group_deviation(&database, 0) - 2.0).abs() < 1e-3);
}

#[test]
fn a_constant_group_normalises_to_exactly_zero() {
    let database = baked(vec![ramp(1.0), flat(1.0)]);

    for frame in 0..database.frame_count().unwrap() {
        let normalised = database.normalised_features(frame).unwrap();
        assert_eq!(normalised[2], 0.0);
        assert!(normalised.iter().all(|value| value.is_finite()));
    }
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

struct Extracted {
    channel: Box<dyn FeatureChannel>,
    trajectory: Vec<TrajectorySample>,
    features: Vec<f32>,
}

#[test]
fn each_reference_channel_extracts_its_features_in_the_simulation_frame() {
    let rows = [
        Extracted {
            channel: Box::new(BonePositions {
                bones: vec!["Hips".into()],
                weight: 0.75,
            }),
            trajectory: Vec::new(),
            features: vec![-5.0, 5.0, 3.0],
        },
        Extracted {
            channel: Box::new(BoneVelocities {
                bones: vec!["Hips".into(), "Spine2".into()],
                weight: 1.0,
            }),
            trajectory: Vec::new(),
            features: vec![-2.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        },
        Extracted {
            channel: Box::new(TrajectoryPositions {
                samples: vec![1],
                weight: 1.0,
            }),
            trajectory: vec![
                TrajectorySample::default(),
                TrajectorySample {
                    position: Vec3::new(1.0, 3.0, 6.0),
                    ..Default::default()
                },
            ],
            features: vec![-4.0, 0.0],
        },
        Extracted {
            channel: Box::new(TrajectoryDirections {
                samples: vec![0, 1],
                weight: 1.5,
            }),
            trajectory: vec![
                TrajectorySample {
                    rotation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
                    ..Default::default()
                },
                TrajectorySample {
                    rotation: Quat::from_rotation_y(std::f32::consts::PI),
                    ..Default::default()
                },
            ],
            features: vec![0.0, 1.0, 1.0, 0.0],
        },
    ];

    for row in rows {
        assert_close(
            &extract_with(row.channel.as_ref(), &row.trajectory),
            &row.features,
        );
    }
}

fn query_definition(
    [ramp_weight, positions_weight, flat_weight, directions_weight]: [f32; 4],
) -> MotionDatabase {
    let positions = FeatureChannelData::live(TrajectoryPositions {
        samples: vec![0, 2],
        weight: positions_weight,
    });
    let directions = FeatureChannelData::live(TrajectoryDirections {
        samples: vec![1],
        weight: directions_weight,
    });
    MotionDatabase {
        entries: vec![MotionEntry {
            clip: Default::default(),
            range: None,
            mirrored: false,
        }],
        rate: 30.0,
        speed: 1.0,
        simulation: SimulationBoneSettings {
            facing_axis: JointAxis::Z,
            position_window: 5,
            facing_window: 5,
            ..Default::default()
        },
        channels: vec![ramp(ramp_weight), positions, flat(flat_weight), directions],
        ..Default::default()
    }
}

fn query_database() -> MotionDatabase {
    with_bake(query_definition([2.0, 1.0, 1.0, 1.5])).unwrap()
}

fn frame_trajectory(database: &MotionDatabase, frame: usize) -> Vec<TrajectorySample> {
    let simulation: Vec<Transform> = (0..database.frame_count().unwrap())
        .map(|frame| database.simulation_bone(frame).unwrap())
        .collect();
    let motion = vec![BodyMotion::default(); database.frame_count().unwrap()];
    trajectory_samples(
        &database.trajectory,
        &simulation,
        &motion,
        database.rate,
        frame,
    )
}

#[test]
fn a_query_for_a_database_frame_equals_its_normalised_features() {
    let database = query_database();
    let query = FeatureQuery::new(&database).unwrap();

    for frame in [0, 7, database.frame_count().unwrap() - 1] {
        let built = query
            .build(
                &database,
                &skeleton(),
                frame,
                &database.simulation_bone(frame).unwrap(),
                &frame_trajectory(&database, frame),
            )
            .unwrap();

        assert_close(&built, &database.normalised_features(frame).unwrap());
    }
}
