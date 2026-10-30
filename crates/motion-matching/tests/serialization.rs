use concerto_animation::clip::{AnimationChanelOutput, AnimationChannel, AnimationClip};
use concerto_foundation::assets::{handle::AssetHandle, AssetId};
use concerto_mesh::{JointPose, Skeleton};
use concerto_motion_matching::bake::bake;
use concerto_motion_matching::{
    reference_channels, FeatureChannel, FrameContext, MotionDatabase, MotionDatabaseDefinition,
    MotionEntry, ReflectFeatureChannel, SimulationBoneSettings, StoredChannel, TimeRange,
    TrajectorySettings,
};
use concerto_reflect::{to_value, Reflect, ReflectedValue, TypeRegistry};
use glam::{Mat4, Vec3};
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

fn entry(range: Option<(f32, f32)>) -> MotionEntry {
    MotionEntry {
        clip: Default::default(),
        range: range.map(|(start, end)| TimeRange { start, end }),
    }
}

#[derive(Reflect, Default, Clone, Debug, PartialEq)]
#[reflect(FeatureChannel)]
struct HeightChannel {
    scale: f32,
}

impl FeatureChannel for HeightChannel {
    fn dims(&self) -> usize {
        2
    }

    fn extract(&self, frame: &FrameContext, out: &mut [f32]) {
        out[0] = frame.simulation.translation.z * self.scale;
        out[1] = frame.trajectory.len() as f32;
    }
}

fn database() -> MotionDatabase {
    let definition = MotionDatabaseDefinition {
        entries: vec![entry(None), entry(Some((0.5, 1.5)))],
        rate: 10.0,
        speed: 2.0,
        simulation: SimulationBoneSettings {
            position_joint: "Child".into(),
            facing_joint: "Root".into(),
            position_window: 5,
            facing_window: 5,
            ..Default::default()
        },
        channels: Vec::new(),
        ..Default::default()
    };
    let first = walk_clip(2.0);
    let second = walk_clip(2.0);
    bake(
        &definition,
        &skeleton(),
        &[&first, &second],
        &TypeRegistry::default(),
    )
    .unwrap()
}

#[test]
fn definition_defaults_match_the_spec() {
    let definition = MotionDatabaseDefinition::default();
    assert!(definition.entries.is_empty());
    assert_eq!(definition.channels, reference_channels());
    assert_eq!(definition.channels.len(), 5);
    assert_eq!(definition.rate, 60.0);
    assert_eq!(definition.speed, 1.1);
    assert_eq!(definition.simulation.position_joint, "Spine2");
    assert_eq!(definition.simulation.facing_joint, "Hips");
    assert_eq!(definition.simulation.facing_axis, Vec3::Z);
    assert_eq!(definition.simulation.position_window, 31);
    assert_eq!(definition.simulation.facing_window, 61);
    assert_eq!(definition.trajectory.sample_dt, 1.0 / 3.0);
    assert_eq!(definition.trajectory.sample_count, 3);
}

#[test]
fn definition_round_trips_through_bincode() {
    let definition = MotionDatabaseDefinition {
        entries: vec![
            MotionEntry {
                range: Some(TimeRange {
                    start: 0.5,
                    end: 2.0,
                }),
                ..Default::default()
            },
            MotionEntry::default(),
        ],
        rate: 30.0,
        speed: 1.0,
        simulation: SimulationBoneSettings {
            position_joint: "Chest".into(),
            facing_joint: "Pelvis".into(),
            facing_axis: Vec3::X,
            position_window: 15,
            facing_window: 21,
        },
        trajectory: TrajectorySettings {
            sample_dt: 0.5,
            sample_count: 5,
        },
        channels: vec![StoredChannel {
            type_name: "test::Channel".into(),
            value: ReflectedValue::Struct(vec![("scale".into(), ReflectedValue::Leaf(vec![1, 2]))]),
        }],
        ..Default::default()
    };
    let bytes = bincode::serialize(&definition).unwrap();
    let decoded: MotionDatabaseDefinition = bincode::deserialize(&bytes).unwrap();
    assert_eq!(decoded, definition);
}

#[test]
fn database_round_trips_through_bincode() {
    let database = database();
    let bytes = bincode::serialize(&database).unwrap();
    let decoded: MotionDatabase = bincode::deserialize(&bytes).unwrap();
    assert_eq!(decoded, database);
}

#[test]
fn accessors_index_frame_major() {
    let database = database();
    assert_eq!(database.frame_count(), 15);
    assert_eq!(database.bone_count(), 2);
    assert_eq!(database.rate(), 10.0);
    assert_eq!(database.speed(), 2.0);
    assert_eq!(database.clips().len(), 2);
    assert_eq!(database.feature_dims(), 0);
    assert!(database.channels().is_empty());
    assert!(database.features(3).is_empty());
    assert!(database.simulation(5).translation.z < database.simulation(4).translation.z);
}

#[test]
fn range_of_finds_the_entry_containing_a_frame() {
    let database = database();
    assert_eq!(database.entry_of(0), Some(0));
    assert_eq!(database.entry_of(9), Some(0));
    assert_eq!(database.entry_of(10), Some(1));
    assert_eq!(database.entry_of(15), None);
    assert_eq!(database.range_of(4), Some(0..10));
    assert_eq!(database.range_of(12), Some(10..15));
    assert_eq!(database.range_of(15), None);
}

#[test]
fn clip_time_follows_speed_and_rate() {
    let database = database();
    assert_eq!(database.clip_time(0), Some((0, 0.0)));
    let (entry, time) = database.clip_time(5).unwrap();
    assert_eq!(entry, 0);
    assert!((time - 1.0).abs() < 1e-6);
    let (entry, time) = database.clip_time(12).unwrap();
    assert_eq!(entry, 1);
    assert!((time - 0.9).abs() < 1e-5);
    assert_eq!(database.clip_time(15), None);
}

#[test]
fn a_database_with_channels_and_handles_round_trips_through_bincode() {
    let skeleton_id = AssetId::new();
    let clip_ids = [AssetId::new(), AssetId::new()];
    let definition = MotionDatabaseDefinition {
        skeleton: AssetHandle::weak(skeleton_id),
        entries: clip_ids
            .iter()
            .map(|id| MotionEntry {
                clip: AssetHandle::weak(*id),
                range: None,
            })
            .collect(),
        rate: 10.0,
        speed: 2.0,
        simulation: SimulationBoneSettings {
            position_joint: "Child".into(),
            facing_joint: "Root".into(),
            position_window: 5,
            facing_window: 5,
            ..Default::default()
        },
        channels: vec![StoredChannel {
            type_name: "HeightChannel".into(),
            value: to_value(&HeightChannel { scale: 2.0 }).unwrap(),
        }],
        ..Default::default()
    };
    let mut registry = TypeRegistry::default();
    registry.register::<HeightChannel>();
    let first = walk_clip(2.0);
    let second = walk_clip(2.0);
    let database = bake(&definition, &skeleton(), &[&first, &second], &registry).unwrap();
    assert_eq!(database.feature_dims(), 2);
    assert_eq!(database.channels().len(), 1);
    assert!(database.features(4)[0] != 0.0);

    let bytes = bincode::serialize(&database).unwrap();
    let decoded: MotionDatabase = bincode::deserialize(&bytes).unwrap();

    assert_eq!(decoded, database);
    assert_eq!(decoded.skeleton().id(), skeleton_id);
    let decoded_clips: Vec<AssetId> = decoded.clips().iter().map(|clip| clip.id()).collect();
    assert_eq!(decoded_clips, clip_ids);
    assert_eq!(decoded.channels(), database.channels());
    assert_eq!(decoded.features(4), database.features(4));
}
