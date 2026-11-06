use concerto_foundation::assets::{handle::AssetHandle, AssetId};
use concerto_motion_matching::{
    weightless_hash, weights_of, BakeError, FeatureChannel, FeatureChannelData, FrameContext,
    MirrorAxis, MotionDatabase, MotionEntry, ReflectFeatureChannel, SimulationBoneSettings,
    TimeRange,
};
use concerto_reflect::{Reflect, TypeRegistry};

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

fn definition() -> MotionDatabase {
    MotionDatabase {
        entries: vec![
            MotionEntry {
                clip: AssetHandle::weak(AssetId::from_path("walk")),
                range: None,
                mirrored: false,
            },
            MotionEntry {
                clip: AssetHandle::weak(AssetId::from_path("turn")),
                range: Some(TimeRange {
                    start: 0.25,
                    end: 1.5,
                }),
                mirrored: false,
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

fn hash(definition: &MotionDatabase) -> u64 {
    weightless_hash(definition, SKELETON, &[WALK, TURN])
}

fn assert_changes_the_hash(change: impl FnOnce(&mut MotionDatabase)) {
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
fn a_changed_mirrored_flag_changes_the_hash() {
    assert_changes_the_hash(|definition| definition.entries[0].mirrored = true);
}

#[test]
fn the_mirror_axis_changes_the_hash_only_when_an_entry_is_mirrored() {
    let mut unmirrored = definition();
    unmirrored.mirror_axis = MirrorAxis::Z;
    assert_eq!(hash(&unmirrored), hash(&definition()));

    let mut mirrored = definition();
    mirrored.entries[1].mirrored = true;
    let mut turned = mirrored.clone();
    turned.mirror_axis = MirrorAxis::Z;
    assert_ne!(hash(&turned), hash(&mirrored));
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
