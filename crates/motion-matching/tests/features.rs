use concerto_animation::clip::{AnimationChanelOutput, AnimationChannel, AnimationClip};
use concerto_mesh::{JointPose, Skeleton};
use concerto_motion_matching::bake::bake;
use concerto_motion_matching::{
    BakeError, FeatureChannel, FrameContext, MotionDatabase, MotionDatabaseDefinition, MotionEntry,
    ReflectFeatureChannel, SimulationBoneSettings, StoredChannel,
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
