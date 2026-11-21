use concerto_foundation::assets::{handle::AssetHandle, AssetId};
use concerto_motion_matching::{
    weightless_hash, FeatureChannel, FeatureChannelData, FrameContext, MirrorAxis, MotionDatabase,
    MotionEntry, ReflectFeatureChannel, SimulationBoneSettings, TimeRange,
};
use concerto_reflect::Reflect;

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

struct Inputs {
    definition: MotionDatabase,
    skeleton: Vec<u8>,
    clips: Vec<Vec<u8>>,
}

impl Inputs {
    fn new() -> Self {
        Self {
            definition: definition(),
            skeleton: SKELETON.to_vec(),
            clips: vec![WALK.to_vec(), TURN.to_vec()],
        }
    }

    fn hash(&self) -> u64 {
        let clips: Vec<&[u8]> = self.clips.iter().map(Vec::as_slice).collect();
        weightless_hash(&self.definition, &self.skeleton, &clips)
    }
}

struct Change {
    name: &'static str,
    base: fn(&mut Inputs),
    change: fn(&mut Inputs),
    changes_the_hash: bool,
}

fn change(name: &'static str, change: fn(&mut Inputs)) -> Change {
    Change {
        name,
        base: |_| {},
        change,
        changes_the_hash: true,
    }
}

#[test]
fn the_hash_follows_every_input_of_the_bake_but_the_top_level_channel_weights() {
    let rows = [
        Change {
            changes_the_hash: false,
            ..change("nothing", |_| {})
        },
        change("a skeleton byte", |inputs| inputs.skeleton[2] = 4),
        change("a clip byte", |inputs| inputs.clips[0][2] = 7),
        change("a byte moved from one clip to the next", |inputs| {
            let byte = inputs.clips[0].pop().unwrap();
            inputs.clips[1].insert(0, byte);
        }),
        change("an entry's clip", |inputs| {
            inputs.definition.entries[0].clip = AssetHandle::weak(AssetId::from_path("run"));
        }),
        change("an entry's range given", |inputs| {
            inputs.definition.entries[0].range = Some(TimeRange::default());
        }),
        change("an entry mirrored", |inputs| {
            inputs.definition.entries[0].mirrored = true;
        }),
        Change {
            changes_the_hash: false,
            ..change("the mirror axis with no entry mirrored", |inputs| {
                inputs.definition.mirror_axis = MirrorAxis::Z;
            })
        },
        Change {
            base: |inputs| inputs.definition.entries[1].mirrored = true,
            ..change("the mirror axis with an entry mirrored", |inputs| {
                inputs.definition.mirror_axis = MirrorAxis::Z;
            })
        },
        change("the rate", |inputs| inputs.definition.rate = 30.0),
        change("the speed", |inputs| inputs.definition.speed = 1.1),
        change("the simulation's position joint", |inputs| {
            inputs.definition.simulation.position_joint = "Root".into();
        }),
        change("the trajectory's sample dt", |inputs| {
            inputs.definition.trajectory.sample_dt = 0.5;
        }),
        change("a channel's setting", |inputs| {
            inputs.definition.channels[0] = stride(9.0, 0.5, 1.0);
        }),
        change("a nested weight", |inputs| {
            inputs.definition.channels[0] = stride(2.0, 0.75, 1.0);
        }),
        Change {
            changes_the_hash: false,
            ..change("every top-level weight", |inputs| {
                inputs.definition.channels[0] = stride(2.0, 0.5, 7.0);
                inputs.definition.channels[1] = stride(3.0, 0.5, 8.0);
            })
        },
    ];

    for row in rows {
        let mut base = Inputs::new();
        (row.base)(&mut base);
        let mut changed = Inputs::new();
        (row.base)(&mut changed);
        (row.change)(&mut changed);

        assert_eq!(
            changed.hash() != base.hash(),
            row.changes_the_hash,
            "{}",
            row.name
        );
    }
}
