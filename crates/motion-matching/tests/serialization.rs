use std::path::Path;

use concerto_animation::clip::{AnimationChanelOutput, AnimationChannel, AnimationClip};
use concerto_foundation::assets::content::{
    check_encoding, decode_content_asset, write_content_asset, ContentAssetHeader,
};
use concerto_foundation::assets::{handle::AssetHandle, Asset, AssetId};
use concerto_mesh::{JointPose, Skeleton};
use concerto_motion_matching::bake::bake;
use concerto_motion_matching::{
    reference_channels, FeatureChannel, FeatureChannelData, FrameContext, JointAxis, MirrorAxis,
    MotionDatabase, MotionEntry, ReflectFeatureChannel, SimulationBoneSettings, TimeRange,
    TrajectorySettings,
};
use concerto_reflect::{Reflect, ReflectedValue, TypeRegistry};
use glam::{Mat4, Quat, Vec3};
use uuid::Uuid;

fn encoding_dir() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/encoding"))
}

fn skeleton() -> Skeleton {
    Skeleton::new(
        vec![Mat4::IDENTITY; 4],
        vec![None, Some(0), Some(1), Some(1)],
        vec![
            "Root".into(),
            "Child".into(),
            "LeftFoot".into(),
            "RightFoot".into(),
        ],
        vec![
            JointPose::IDENTITY,
            JointPose {
                translation: Vec3::new(0.0, 1.0, 0.0),
                ..JointPose::IDENTITY
            },
            JointPose {
                translation: Vec3::new(0.0, -0.5, 0.2),
                ..JointPose::IDENTITY
            },
            JointPose {
                translation: Vec3::new(0.0, -0.5, -0.2),
                ..JointPose::IDENTITY
            },
        ],
        vec![
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            Uuid::from_u128(3),
            Uuid::from_u128(4),
        ],
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
        mirrored: false,
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
    let definition = MotionDatabase {
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
    baked(definition, &[&first, &second])
}

fn baked(definition: MotionDatabase, clips: &[&AnimationClip]) -> MotionDatabase {
    let baked = bake(&definition, &skeleton(), clips).unwrap();
    definition.with_baked(baked)
}

#[test]
fn definition_defaults_match_the_spec() {
    let definition = MotionDatabase::default();
    assert!(definition.entries.is_empty());
    assert_eq!(definition.channels, reference_channels());
    assert_eq!(definition.channels.len(), 5);
    assert_eq!(definition.rate, 60.0);
    assert_eq!(definition.speed, 1.1);
    assert_eq!(definition.simulation.position_joint, "Spine2".into());
    assert_eq!(definition.simulation.facing_joint, "Hips".into());
    assert_eq!(definition.simulation.facing_axis, JointAxis::Y);
    assert_eq!(definition.simulation.position_window, 31);
    assert_eq!(definition.simulation.facing_window, 61);
    assert_eq!(definition.trajectory.sample_dt, 1.0 / 3.0);
    assert_eq!(definition.trajectory.sample_count, 3);
}

#[test]
fn definition_round_trips_through_bincode() {
    let definition = MotionDatabase {
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
            facing_axis: JointAxis::X,
            position_window: 15,
            facing_window: 21,
        },
        trajectory: TrajectorySettings {
            sample_dt: 0.5,
            sample_count: 5,
        },
        channels: vec![FeatureChannelData::Stored {
            type_name: "test::Channel".into(),
            value: ReflectedValue::Struct(vec![("scale".into(), ReflectedValue::Leaf(vec![1, 2]))]),
        }],
        ..Default::default()
    };
    let bytes = bincode::serialize(&definition).unwrap();
    let decoded: MotionDatabase = bincode::deserialize(&bytes).unwrap();
    assert_eq!(decoded, definition);
}

#[test]
fn a_database_round_trips_through_bincode_without_its_baked_part() {
    let database = database();
    assert!(database.is_baked());

    let bytes = bincode::serialize(&database).unwrap();
    let decoded: MotionDatabase = bincode::deserialize(&bytes).unwrap();

    assert!(!decoded.is_baked());
    assert_eq!(
        decoded,
        MotionDatabase {
            baked: None,
            ..database
        }
    );
}

#[test]
fn frames_of_entry_counts_each_range() {
    let database = database();
    assert_eq!(database.frames_of_entry(0), Some(10));
    assert_eq!(database.frames_of_entry(1), Some(5));
    assert_eq!(database.frames_of_entry(2), None);
}

#[test]
fn size_in_bytes_counts_the_feature_simulation_and_range_arrays() {
    let definition = MotionDatabase {
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
        channels: vec![FeatureChannelData::live(HeightChannel { scale: 2.0 })],
        ..Default::default()
    };
    let mut registry = TypeRegistry::default();
    registry.register::<HeightChannel>();
    let first = walk_clip(2.0);
    let second = walk_clip(2.0);
    let database = baked(definition, &[&first, &second]);

    let features = 15 * 2 * size_of::<f32>();
    let simulation = 15 * size_of::<(Vec3, Quat)>();
    let ranges = 2 * (2 * size_of::<usize>() + size_of::<f32>());
    assert_eq!(
        database.size_in_bytes(),
        Some(features + simulation + ranges)
    );
}

#[test]
fn deviations_hold_one_value_per_channel() {
    let definition = MotionDatabase {
        entries: vec![entry(None)],
        rate: 10.0,
        speed: 1.0,
        simulation: SimulationBoneSettings {
            position_joint: "Child".into(),
            facing_joint: "Root".into(),
            position_window: 5,
            facing_window: 5,
            ..Default::default()
        },
        channels: vec![FeatureChannelData::live(HeightChannel { scale: 2.0 })],
        ..Default::default()
    };
    let mut registry = TypeRegistry::default();
    registry.register::<HeightChannel>();
    let clip = walk_clip(2.0);
    let database = baked(definition, &[&clip]);

    assert_eq!(database.deviations().unwrap().len(), 1);
    assert!(database.deviations().unwrap()[0] > 0.0);
}

#[test]
fn accessors_index_frame_major() {
    let database = database();
    assert_eq!(database.frame_count(), Some(15));
    assert_eq!(database.bone_count(), Some(4));
    assert_eq!(database.clips().count(), 2);
    assert_eq!(database.feature_dims(), Some(0));
    assert_eq!(database.channel_range(0), None);
    assert!(database.features(3).unwrap().is_empty());
    assert!(
        database.simulation_bone(5).unwrap().translation.z
            < database.simulation_bone(4).unwrap().translation.z
    );
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
    let definition = MotionDatabase {
        skeleton: AssetHandle::weak(skeleton_id),
        entries: clip_ids
            .iter()
            .map(|id| MotionEntry {
                clip: AssetHandle::weak(*id),
                range: None,
                mirrored: false,
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
        channels: vec![FeatureChannelData::live(HeightChannel { scale: 2.0 })],
        ..Default::default()
    };
    let mut registry = TypeRegistry::default();
    registry.register::<HeightChannel>();
    let first = walk_clip(2.0);
    let second = walk_clip(2.0);
    let database = baked(definition, &[&first, &second]);
    assert_eq!(database.feature_dims(), Some(2));
    assert_eq!(database.channel_range(0), Some(0..2));
    assert!(database.features(4).unwrap()[0] != 0.0);

    let bytes = bincode::serialize(&database).unwrap();
    let decoded: MotionDatabase = bincode::deserialize(&bytes).unwrap();

    assert_eq!(decoded.skeleton.id(), skeleton_id);
    let decoded_clips: Vec<AssetId> = decoded.clips().map(|clip| clip.id()).collect();
    assert_eq!(decoded_clips, clip_ids);
    assert_eq!(decoded.channels, database.channels);
}

fn encoded_definition() -> MotionDatabase {
    MotionDatabase {
        skeleton: AssetHandle::weak(AssetId::from_path("content/hero/skeleton.gasset")),
        entries: vec![
            MotionEntry {
                clip: AssetHandle::weak(AssetId::from_path("content/hero/walk.gasset")),
                range: None,
                mirrored: false,
            },
            MotionEntry {
                clip: AssetHandle::weak(AssetId::from_path("content/hero/run.gasset")),
                range: Some(TimeRange {
                    start: 0.5,
                    end: 1.5,
                }),
                mirrored: true,
            },
        ],
        rate: 10.0,
        speed: 2.0,
        simulation: SimulationBoneSettings {
            position_joint: "Child".into(),
            facing_joint: "Root".into(),
            facing_axis: JointAxis::Z,
            position_window: 5,
            facing_window: 7,
        },
        trajectory: TrajectorySettings {
            sample_dt: 0.25,
            sample_count: 2,
        },
        channels: vec![FeatureChannelData::live(HeightChannel { scale: 2.0 })],
        mirror_axis: MirrorAxis::Z,
        baked: None,
    }
}

#[test]
fn database_encoding_matches_its_fixture() {
    check_encoding(&encoded_definition(), encoding_dir());
}

#[test]
fn a_baked_database_encodes_as_its_definition() {
    let first = walk_clip(2.0);
    let second = walk_clip(2.0);
    let database = baked(encoded_definition(), &[&first, &second]);
    assert!(database.is_baked());

    check_encoding(&database, encoding_dir());
}

fn content_asset(kind: &str, version: u32, fixture: &str) -> Vec<u8> {
    let payload = std::fs::read(encoding_dir().join(fixture)).unwrap();
    let header = ContentAssetHeader {
        asset_id: AssetId::new(),
        references: Vec::new(),
        kind: kind.into(),
        kind_version: version,
        provenance: None,
        derived: None,
    };
    write_content_asset(&header, &payload).unwrap()
}

#[test]
fn a_version_2_definition_loads_as_a_motion_database() {
    let bytes = content_asset(
        "MotionDatabaseDefinition",
        2,
        "MotionDatabaseDefinition.v2.bin",
    );

    let (header, database) = decode_content_asset::<MotionDatabase>(&bytes).unwrap();

    assert_eq!(header.kind, "MotionDatabaseDefinition");
    assert_eq!(database, encoded_definition());
    assert!(!database.is_baked());
}

#[test]
fn a_version_2_bake_is_rejected_with_the_delete_message() {
    let bytes = content_asset(MotionDatabase::name(), 2, "MotionDatabase.v2.bin");

    let error = decode_content_asset::<MotionDatabase>(&bytes)
        .err()
        .unwrap();

    assert!(
        format!("{error:#}").contains(
            "a baked motion database is now derived from its definition; delete this file"
        ),
        "{error:#}"
    );
}
