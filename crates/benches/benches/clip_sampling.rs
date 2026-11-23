use criterion::{criterion_group, criterion_main, Criterion};
use glam::Quat;
use uuid::Uuid;

use concerto_animation::clip::{AnimationChanelOutput, AnimationChannel, AnimationClip, KeyCursor};
use concerto_animation::pose::JointPose;

const BONES: usize = 60;
const KEYS: usize = 300;
const KEY_RATE: f32 = 30.0;
const FRAME: f32 = 1.0 / 60.0;

fn key_times() -> Vec<f32> {
    (0..KEYS).map(|key| key as f32 / KEY_RATE).collect()
}

fn vectors(bone: usize, base: f32) -> impl Iterator<Item = [f32; 3]> {
    (0..KEYS).map(move |key| {
        let swing = (key as f32 * 0.1 + bone as f32).sin();
        [base + swing, base, base - swing]
    })
}

fn rotations(bone: usize) -> AnimationChanelOutput {
    AnimationChanelOutput::Rotation(
        (0..KEYS)
            .map(|key| Quat::from_rotation_y((key as f32 * 0.05 + bone as f32).sin()))
            .collect(),
    )
}

fn clip(bones: &[Uuid]) -> AnimationClip {
    let mut clip = AnimationClip::default();
    for (bone, id) in bones.iter().enumerate() {
        let outputs = [
            AnimationChanelOutput::from_translation(vectors(bone, 0.0)),
            rotations(bone),
            AnimationChanelOutput::from_scale(vectors(bone, 1.0)),
        ];
        for output in outputs {
            clip.add_channel(*id, AnimationChannel::new(key_times(), output));
        }
    }
    clip
}

struct Playback {
    clip: AnimationClip,
    bones: Vec<Uuid>,
    pose: Vec<JointPose>,
    cursors: Vec<KeyCursor>,
    time: f32,
}

impl Playback {
    fn new() -> Self {
        let bones: Vec<Uuid> = (1..=BONES)
            .map(|bone| Uuid::from_u128(bone as u128))
            .collect();
        Self {
            clip: clip(&bones),
            pose: vec![JointPose::IDENTITY; BONES],
            cursors: vec![KeyCursor::default(); BONES * 3],
            bones,
            time: 0.0,
        }
    }

    fn advance(&mut self) -> f32 {
        self.time = (self.time + FRAME) % self.clip.duration();
        self.time
    }

    fn search(&mut self) {
        let time = self.advance();
        for (id, joint) in self.bones.iter().zip(&mut self.pose) {
            let Some(channels) = self.clip.get_channels(id) else {
                continue;
            };
            for channel in channels {
                channel.sample_transform(time, joint);
            }
        }
    }

    fn follow(&mut self) {
        let time = self.advance();
        let mut cursors = self.cursors.iter_mut();
        for (id, joint) in self.bones.iter().zip(&mut self.pose) {
            let Some(channels) = self.clip.get_channels(id) else {
                continue;
            };
            for (channel, cursor) in channels.iter().zip(&mut cursors) {
                channel.sample_transform_from(time, cursor, joint);
            }
        }
    }
}

fn frame_by_frame(c: &mut Criterion) {
    let mut group = c.benchmark_group("clip_sampling_frame_by_frame");
    let mut searched = Playback::new();
    group.bench_function("search", |b| {
        b.iter(|| searched.search());
    });
    let mut followed = Playback::new();
    group.bench_function("cursor", |b| {
        b.iter(|| followed.follow());
    });
    group.finish();
}

criterion_group!(benches, frame_by_frame);
criterion_main!(benches);
