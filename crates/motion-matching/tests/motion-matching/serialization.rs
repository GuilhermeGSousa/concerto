use std::path::Path;

use concerto_animation::clip::{AnimationChanelOutput, AnimationChannel, AnimationClip};
use concerto_foundation::assets::content::{
    check_encoding, decode_content_asset, write_content_asset, ContentAssetHeader,
};
use concerto_foundation::assets::{handle::AssetHandle, Asset, AssetId};
use concerto_mesh::{JointPose, Skeleton};
use concerto_motion_matching::bake::bake;
use concerto_motion_matching::{
    FeatureChannel, FeatureChannelData, FrameContext, JointAxis, MirrorAxis, MotionDatabase,
    MotionEntry, ReflectFeatureChannel, SimulationBoneSettings, TimeRange, TrajectorySettings,
};
use concerto_reflect::Reflect;
use glam::{Mat4, Vec3};
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

fn baked(definition: MotionDatabase, clips: &[&AnimationClip]) -> MotionDatabase {
    let baked = bake(&definition, &skeleton(), clips).unwrap();
    definition.with_baked(baked)
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
fn database_encoding_matches_its_fixture_and_decodes_back() {
    let definition = encoded_definition();
    check_encoding(&definition, encoding_dir());

    let bytes = bincode::serialize(&definition).unwrap();
    let decoded: MotionDatabase = bincode::deserialize(&bytes).unwrap();

    assert_eq!(decoded, definition);
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
        embedded: Vec::new(),
    };
    write_content_asset(&header, &payload).unwrap()
}

#[test]
fn a_version_2_bake_is_rejected() {
    let bytes = content_asset(MotionDatabase::name(), 2, "MotionDatabase.v2.bin");

    assert!(decode_content_asset::<MotionDatabase>(&bytes).is_err());
}
