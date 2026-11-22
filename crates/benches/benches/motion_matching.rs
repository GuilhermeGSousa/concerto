use criterion::{criterion_group, criterion_main, Criterion};
use glam::{Mat4, Vec3};
use uuid::Uuid;

use concerto::animation::clip::{AnimationChanelOutput, AnimationChannel, AnimationClip};
use concerto::mesh::{JointPose, Skeleton};
use concerto::motion_matching::{
    bake, search, FeatureChannel, FeatureChannelData, FrameContext, MotionDatabase, MotionEntry,
    ReflectFeatureChannel,
};
use concerto::reflect::Reflect;

const FRAMES: usize = 20_000;
const GROUPS: usize = 3;
const GROUP_DIMS: usize = 9;
const TARGET: usize = 12_345;

#[derive(Reflect, Default, Clone)]
#[reflect(FeatureChannel)]
struct Waves {
    phase: f32,
    weight: f32,
}

impl FeatureChannel for Waves {
    fn dims(&self) -> usize {
        GROUP_DIMS
    }

    fn extract(&self, frame: &FrameContext, out: &mut [f32]) {
        let travelled = frame.simulation.translation.z;
        for (index, value) in out.iter_mut().enumerate() {
            *value = (travelled * (0.37 + index as f32 * 1.3) + self.phase).sin();
        }
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
        vec![JointPose::IDENTITY; 3],
        vec![Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3)],
    )
    .unwrap()
}

fn walk(seconds: f32) -> AnimationClip {
    let mut clip = AnimationClip::default();
    clip.add_channel(
        Uuid::from_u128(1),
        AnimationChannel::new(
            vec![0.0, seconds],
            AnimationChanelOutput::Translation(vec![Vec3::ZERO, Vec3::new(0.0, 0.0, -seconds)]),
        ),
    );
    clip
}

fn channels(weight: f32) -> Vec<FeatureChannelData> {
    (0..GROUPS)
        .map(|group| {
            FeatureChannelData::live(Waves {
                phase: group as f32,
                weight,
            })
        })
        .collect()
}

fn database() -> MotionDatabase {
    let definition = MotionDatabase {
        entries: vec![MotionEntry {
            clip: Default::default(),
            range: None,
            mirrored: false,
        }],
        channels: channels(1.0),
        ..Default::default()
    };
    let seconds = FRAMES as f32 * definition.speed / definition.rate;
    let baked = bake(&definition, &skeleton(), &[&walk(seconds)]).unwrap();
    definition.with_baked(baked)
}

fn bench_search(c: &mut Criterion) {
    let loaded = database();
    let frames = loaded.frame_count().unwrap();
    assert!(frames >= FRAMES - 1 && loaded.feature_dims() == Some(GROUPS * GROUP_DIMS));
    let reweighted = MotionDatabase {
        channels: channels(2.0),
        ..loaded.clone()
    };

    let mut group = c.benchmark_group("motion_matching_search");
    group.bench_function("weights_as_loaded", |b| {
        let query = loaded.normalised_features(TARGET).unwrap();
        b.iter(|| search(&loaded, &query, None, 0, 0));
    });
    group.bench_function("weights_changed_in_place", |b| {
        let query = reweighted.normalised_features(TARGET).unwrap();
        b.iter(|| search(&reweighted, &query, None, 0, 0));
    });
    group.finish();
}

criterion_group!(benches, bench_search);
criterion_main!(benches);
