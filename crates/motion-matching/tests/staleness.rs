use concerto_animation::clip::{AnimationChanelOutput, AnimationChannel, AnimationClip};
use concerto_foundation::assets::{handle::AssetHandle, AssetId};
use concerto_mesh::{JointPose, Skeleton};
use concerto_motion_matching::bake::bake;
use concerto_motion_matching::{
    is_stale, weightless_hash, weights_changed_only, weights_of, BakeError, FeatureChannel,
    FeatureChannelData, FrameContext, MotionDatabase, MotionDatabaseDefinition, MotionEntry,
    ReflectFeatureChannel, SimulationBoneSettings, TimeRange,
};
use concerto_reflect::{Reflect, TypeRegistry};
use glam::{Mat4, Vec3};
use uuid::Uuid;

const SKELETON: &[u8] = &[1, 2, 3];
const WALK: &[u8] = &[4, 5, 6];
const TURN: &[u8] = &[7, 8];

#[derive(Reflect, Default, Clone, Debug, PartialEq)]
struct Gait {
    weight: f32,
}

#[derive(Reflect, Default, Clone, Debug, PartialEq)]
#[reflect(FeatureChannel)]
struct Stride {
    scale: f32,
    gait: Gait,
    weight: f32,
}

impl FeatureChannel for Stride {
    fn dims(&self) -> usize {
        1
    }

    fn extract(&self, frame: &FrameContext, out: &mut [f32]) {
        out[0] = frame.simulation.translation.z * self.scale;
    }

    fn weight(&self) -> f32 {
        self.weight
    }
}

fn stride(scale: f32, nested: f32, weight: f32) -> FeatureChannelData {
    FeatureChannelData::live(Stride {
        scale,
        gait: Gait { weight: nested },
        weight,
    })
}

fn unloaded(channel: &FeatureChannelData) -> FeatureChannelData {
    FeatureChannelData::Stored {
        type_name: channel.type_name().into(),
        value: channel.value().unwrap().into_owned(),
    }
}

fn registry() -> TypeRegistry {
    let mut registry = TypeRegistry::default();
    registry.register::<Stride>();
    registry
}

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

fn walk_clip() -> AnimationClip {
    let mut clip = AnimationClip::default();
    clip.add_channel(
        Uuid::from_u128(1),
        AnimationChannel::new(
            vec![0.0, 2.0],
            AnimationChanelOutput::Translation(vec![Vec3::ZERO, Vec3::new(0.0, 0.0, -2.0)]),
        ),
    );
    clip
}

fn definition() -> MotionDatabaseDefinition {
    MotionDatabaseDefinition {
        entries: vec![
            MotionEntry {
                clip: AssetHandle::weak(AssetId::from_path("walk")),
                range: None,
            },
            MotionEntry {
                clip: AssetHandle::weak(AssetId::from_path("turn")),
                range: Some(TimeRange {
                    start: 0.25,
                    end: 1.5,
                }),
            },
        ],
        rate: 10.0,
        speed: 1.0,
        simulation: SimulationBoneSettings {
            position_joint: "Child".into(),
            facing_joint: "Root".into(),
            position_window: 5,
            facing_window: 5,
            ..Default::default()
        },
        channels: vec![stride(2.0, 0.5, 1.0), stride(3.0, 0.5, 0.25)],
        ..Default::default()
    }
}

fn hash(definition: &MotionDatabaseDefinition) -> u64 {
    weightless_hash(definition, SKELETON, &[WALK, TURN])
}

fn baked(definition: &MotionDatabaseDefinition) -> MotionDatabase {
    let walk = walk_clip();
    bake(definition, &skeleton(), &[&walk, &walk])
        .unwrap()
        .with_source_hash(hash(definition))
}

fn assert_changes_the_hash(change: impl FnOnce(&mut MotionDatabaseDefinition)) {
    let mut changed = definition();
    change(&mut changed);
    assert_ne!(hash(&changed), hash(&definition()));
}

#[test]
fn the_same_input_gives_the_same_hash() {
    assert_eq!(hash(&definition()), hash(&definition()));
}

#[test]
fn a_changed_skeleton_byte_changes_the_hash() {
    assert_ne!(
        weightless_hash(&definition(), &[1, 2, 4], &[WALK, TURN]),
        hash(&definition())
    );
}

#[test]
fn a_changed_skeleton_asset_changes_the_hash_even_with_identical_bytes() {
    assert_changes_the_hash(|definition| {
        definition.skeleton = AssetHandle::weak(AssetId::from_path("another skeleton"));
    });
}

#[test]
fn a_changed_clip_byte_changes_the_hash() {
    assert_ne!(
        weightless_hash(&definition(), SKELETON, &[&[4, 5, 7], TURN]),
        hash(&definition())
    );
}

#[test]
fn a_byte_moved_from_one_clip_to_the_next_changes_the_hash() {
    assert_ne!(
        weightless_hash(&definition(), SKELETON, &[&[4, 5], &[6, 7, 8]]),
        hash(&definition())
    );
}

#[test]
fn a_changed_entry_changes_the_hash() {
    assert_changes_the_hash(|definition| {
        definition.entries[0].clip = AssetHandle::weak(AssetId::from_path("run"));
    });
    assert_changes_the_hash(|definition| definition.entries[0].range = Some(TimeRange::default()));
    assert_changes_the_hash(|definition| {
        definition.entries[1].range = Some(TimeRange {
            start: 0.25,
            end: 1.0,
        });
    });
    assert_changes_the_hash(|definition| {
        definition.entries.pop();
    });
}

#[test]
fn a_changed_setting_changes_the_hash() {
    assert_changes_the_hash(|definition| definition.rate = 30.0);
    assert_changes_the_hash(|definition| definition.speed = 1.1);
    assert_changes_the_hash(|definition| definition.simulation.position_joint = "Root".into());
    assert_changes_the_hash(|definition| definition.simulation.position_window = 7);
    assert_changes_the_hash(|definition| definition.trajectory.sample_dt = 0.5);
    assert_changes_the_hash(|definition| definition.trajectory.sample_count = 2);
}

#[test]
fn a_changed_channel_changes_the_hash() {
    assert_changes_the_hash(|definition| definition.channels[0] = stride(9.0, 0.5, 1.0));
    assert_changes_the_hash(|definition| {
        definition.channels[0] = FeatureChannelData::Stored {
            type_name: "Pace".into(),
            value: definition.channels[0].value().unwrap().into_owned(),
        }
    });
    assert_changes_the_hash(|definition| definition.channels.swap(0, 1));
    assert_changes_the_hash(|definition| {
        definition.channels.pop();
    });
}

#[test]
fn a_changed_weight_keeps_the_hash() {
    let mut changed = definition();
    changed.channels[0] = stride(2.0, 0.5, 7.0);
    changed.channels[1] = stride(3.0, 0.5, 8.0);

    assert_eq!(hash(&changed), hash(&definition()));
}

#[test]
fn a_changed_nested_weight_changes_the_hash() {
    assert_changes_the_hash(|definition| definition.channels[0] = stride(2.0, 0.75, 1.0));
}

#[test]
fn weights_of_reads_the_live_channels_in_order() {
    assert_eq!(weights_of(&definition()).unwrap(), vec![1.0, 0.25]);
}

#[test]
fn weights_of_names_a_channel_that_was_not_loaded() {
    let mut definition = definition();
    definition.channels[0] = unloaded(&definition.channels[0]);

    assert_eq!(
        weights_of(&definition),
        Err(BakeError::UnloadedChannel("Stride".into()))
    );
}

#[test]
fn a_loaded_definition_hashes_as_it_did_stored() {
    let live = definition();
    let mut stored = definition();
    stored.channels = live.channels.iter().map(unloaded).collect();
    assert_eq!(hash(&stored), hash(&live));

    stored.load_channels(&registry()).unwrap();
    assert_eq!(weights_of(&stored), weights_of(&live));
}

#[test]
fn a_database_baked_from_the_same_input_is_not_stale() {
    let definition = definition();

    assert!(!is_stale(
        &baked(&definition),
        &definition,
        hash(&definition)
    ));
}

#[test]
fn a_changed_weight_is_stale_though_the_hash_is_equal() {
    let database = baked(&definition());
    let mut changed = definition();
    changed.channels[0] = stride(2.0, 0.5, 7.0);

    assert_eq!(hash(&changed), database.source_hash());
    assert!(is_stale(&database, &changed, hash(&changed)));
}

#[test]
fn a_changed_clip_is_stale() {
    let definition = definition();
    let database = baked(&definition);
    let new_hash = weightless_hash(&definition, SKELETON, &[&[4, 5, 7], TURN]);

    assert!(is_stale(&database, &definition, new_hash));
}

#[test]
fn a_changed_definition_is_stale() {
    let database = baked(&definition());
    let mut changed = definition();
    changed.rate = 30.0;

    assert!(is_stale(&database, &changed, hash(&changed)));
}

#[test]
fn a_channel_that_was_not_loaded_is_stale() {
    let database = baked(&definition());
    let mut definition = definition();
    definition.channels[0] = unloaded(&definition.channels[0]);

    assert!(is_stale(&database, &definition, hash(&definition)));
}

#[test]
fn new_weights_applied_to_the_database_are_not_stale() {
    let mut database = baked(&definition());
    let mut changed = definition();
    changed.channels[0] = stride(2.0, 0.5, 7.0);

    database.reweigh(&changed).unwrap();

    assert!(!is_stale(&database, &changed, hash(&changed)));
}

#[test]
fn only_the_weights_changed_when_the_hash_is_equal_and_a_weight_differs() {
    let database = baked(&definition());
    let mut weights = definition();
    weights.channels[0] = stride(2.0, 0.5, 7.0);
    let mut rate = weights.clone();
    rate.rate = 30.0;

    assert!(weights_changed_only(&database, &weights, hash(&weights)));
    assert!(!weights_changed_only(
        &database,
        &definition(),
        hash(&definition())
    ));
    assert!(!weights_changed_only(&database, &rate, hash(&rate)));
}
