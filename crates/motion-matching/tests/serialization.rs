use concerto_animation::clip::{AnimationChanelOutput, AnimationChannel, AnimationClip};
use concerto_mesh::{JointPose, Skeleton};
use concerto_motion_matching::bake::bake;
use concerto_motion_matching::{
    ContactSettings, MotionDatabase, MotionDatabaseDefinition, MotionEntry, SimulationBoneSettings,
    StoredChannel, TimeRange,
};
use concerto_reflect::{ReflectedValue, TypeRegistry};
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
        contacts: ContactSettings {
            bones: vec!["Child".into()],
            speed_threshold: 10.0,
            median_window: 3,
        },
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
    assert!(definition.channels.is_empty());
    assert_eq!(definition.rate, 60.0);
    assert_eq!(definition.speed, 1.1);
    assert_eq!(definition.simulation.position_joint, "Spine2");
    assert_eq!(definition.simulation.facing_joint, "Hips");
    assert_eq!(definition.simulation.facing_axis, Vec3::Z);
    assert_eq!(definition.simulation.position_window, 31);
    assert_eq!(definition.simulation.facing_window, 61);
    assert!(definition.contacts.bones.is_empty());
    assert_eq!(definition.contacts.speed_threshold, 0.15);
    assert_eq!(definition.contacts.median_window, 6);
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
        contacts: ContactSettings {
            bones: vec!["LeftFoot".into(), "RightFoot".into()],
            speed_threshold: 0.2,
            median_window: 4,
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
    assert_eq!(database.contact_bones(), &[1]);
    assert_eq!(database.contacts(0).len(), 1);
    assert_eq!(database.contacts(14).len(), 1);
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
