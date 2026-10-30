use concerto_motion_matching::{
    ChannelSpan, ContactSettings, MotionDatabase, MotionDatabaseDefinition, MotionEntry,
    SimulationBoneSettings, StoredChannel, TimeRange,
};
use concerto_reflect::ReflectedValue;
use glam::{Quat, Vec3};

fn database() -> MotionDatabase {
    let bone_count = 3;
    let frame_count = 4;
    let cells = bone_count * frame_count;
    MotionDatabase {
        names: vec!["simulation".into(), "hips".into(), "foot".into()],
        parents: vec![None, Some(0), Some(1)],
        rate: 60.0,
        bone_count,
        frame_count,
        positions: (0..cells).map(|i| Vec3::splat(i as f32)).collect(),
        rotations: vec![Quat::IDENTITY; cells],
        velocities: (0..cells).map(|i| Vec3::splat(-(i as f32))).collect(),
        angular_velocities: vec![Vec3::ZERO; cells],
        contact_bones: vec![2],
        contacts: vec![true, false, false, true],
        range_starts: vec![0, 3],
        range_stops: vec![3, 4],
        features: (0..frame_count * 2).map(|i| i as f32).collect(),
        feature_dims: 2,
        channels: vec![ChannelSpan {
            type_name: "test".into(),
            offset: 0,
            dims: 2,
        }],
    }
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
    let pose = database.pose(2);
    assert_eq!(
        pose.positions,
        &[Vec3::splat(6.0), Vec3::splat(7.0), Vec3::splat(8.0)]
    );
    assert_eq!(pose.velocities[1], Vec3::splat(-7.0));
    assert_eq!(pose.rotations.len(), 3);
    assert_eq!(pose.angular_velocities.len(), 3);
    assert_eq!(database.contacts(1), &[false]);
    assert_eq!(database.contacts(3), &[true]);
    assert_eq!(database.features(2), &[4.0, 5.0]);
}

#[test]
fn range_of_finds_the_entry_containing_a_frame() {
    let database = database();
    assert_eq!(database.range_of(0), Some(0));
    assert_eq!(database.range_of(2), Some(0));
    assert_eq!(database.range_of(3), Some(1));
    assert_eq!(database.range_of(4), None);
}
