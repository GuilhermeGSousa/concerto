use concerto_animation::clip::{AnimationChanelOutput, AnimationChannel, KeyCursor};
use concerto_animation::pose::JointPose;
use glam::{Quat, Vec3};
use proptest::prelude::*;

fn untouched() -> JointPose {
    JointPose {
        translation: Vec3::splat(7.0),
        rotation: Quat::from_rotation_y(0.3),
        scale: Vec3::splat(3.0),
    }
}

#[derive(Debug)]
struct Keys {
    kind: usize,
    steps: Vec<u8>,
    values: Vec<[f32; 4]>,
}

impl Keys {
    fn channel(&self) -> AnimationChannel {
        let mut time = 0.0;
        let times = self
            .steps
            .iter()
            .map(|step| {
                time += f32::from(*step) * 0.25;
                time
            })
            .collect();
        let vectors = self.values.iter().map(|[x, y, z, _]| [*x, *y, *z]);
        let outputs = match self.kind {
            0 => AnimationChanelOutput::from_translation(vectors),
            1 => AnimationChanelOutput::Rotation(
                self.values
                    .iter()
                    .map(|[x, y, z, w]| Quat::from_xyzw(*x, *y, *z, *w + 3.0).normalize())
                    .collect(),
            ),
            _ => AnimationChanelOutput::from_scale(vectors),
        };
        AnimationChannel::new(times, outputs)
    }
}

fn any_keys() -> impl Strategy<Value = Keys> {
    (0usize..3, 0usize..12).prop_flat_map(|(kind, keys)| {
        (
            prop::collection::vec(0u8..4, keys),
            prop::collection::vec(prop::array::uniform4(-2.0f32..2.0), keys),
        )
            .prop_map(move |(steps, values)| Keys {
                kind,
                steps,
                values,
            })
    })
}

fn any_time() -> impl Strategy<Value = f32> {
    prop_oneof![
        (0u8..40).prop_map(|quarter| f32::from(quarter) * 0.25),
        -1.0f32..10.0,
    ]
}

fn any_times() -> impl Strategy<Value = Vec<f32>> {
    prop_oneof![
        prop::collection::vec(any_time(), 1..40),
        (any_time(), 0.001f32..0.3, 1usize..60).prop_map(|(start, step, frames)| {
            (0..frames)
                .map(|frame| (start + step * frame as f32) % 9.0)
                .collect()
        }),
    ]
}

proptest! {
    #[test]
    fn sampling_from_a_cursor_equals_a_fresh_search_whatever_the_cursor_last_saw(
        keys in prop::collection::vec(any_keys(), 1..4),
        times in any_times(),
    ) {
        let mut cursor = KeyCursor::default();
        for channel in keys.iter().map(Keys::channel) {
            for &time in &times {
                let mut searched = untouched();
                channel.sample_transform(time, &mut searched);
                let mut followed = untouched();
                channel.sample_transform_from(time, &mut cursor, &mut followed);
                prop_assert_eq!(&followed, &searched, "at {} over {:?}", time, channel.key_times());
            }
        }
    }
}
