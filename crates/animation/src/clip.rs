use std::collections::{HashMap, hash_map::Keys};

use concerto_foundation::assets::{Asset, LoadableAsset};
use concerto_foundation::math::spring_damper::angular_velocity;
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::inertialization::PoseVelocity;
use crate::pose::JointPose;

#[derive(Serialize, Deserialize)]
pub enum AnimationChanelOutput {
    Translation(Vec<Vec3>),
    Rotation(Vec<Quat>),
    Scale(Vec<Vec3>),
}

impl AnimationChanelOutput {
    pub fn from_translation(translations: impl Iterator<Item = [f32; 3]>) -> Self {
        Self::Translation(translations.map(Vec3::from_array).collect::<Vec<_>>())
    }

    pub fn from_rotation(rotations: impl Iterator<Item = [f32; 4]>) -> Self {
        Self::Rotation(rotations.map(Quat::from_array).collect::<Vec<_>>())
    }

    pub fn from_scale(scales: impl Iterator<Item = [f32; 3]>) -> Self {
        Self::Scale(scales.map(Vec3::from_array).collect::<Vec<_>>())
    }
}

#[derive(Serialize, Deserialize)]
pub struct AnimationChannel {
    time_samples: Vec<f32>,
    outputs: AnimationChanelOutput,
}

impl AnimationChannel {
    pub fn new(time_samples: Vec<f32>, outputs: AnimationChanelOutput) -> Self {
        Self {
            time_samples,
            outputs,
        }
    }

    pub fn sample_transform(&self, current_time: f32, transform: &mut JointPose) {
        if self.time_samples.len() < 2 {
            return;
        }

        match self
            .time_samples
            .binary_search_by(|val| val.total_cmp(&current_time))
        {
            Ok(index) => {
                self.set_transform_at(transform, index);
            }
            Err(index) => {
                if index == 0 {
                    self.set_transform_at(transform, index);
                } else if index >= self.time_samples.len() {
                    self.set_transform_at(transform, self.time_samples.len() - 1);
                } else {
                    let after_time = self.time_samples[index];
                    let before_time = self.time_samples[index - 1];

                    let normalized_time = (current_time - before_time) / (after_time - before_time);

                    self.interpolate_between(transform, index - 1, normalized_time);
                }
            }
        }
    }

    pub fn duration(&self) -> Option<f32> {
        self.time_samples.last().copied()
    }

    fn median_key_spacing(&self) -> Option<f32> {
        let mut spacings: Vec<f32> = self
            .time_samples
            .windows(2)
            .map(|keys| keys[1] - keys[0])
            .collect();
        spacings.sort_by(f32::total_cmp);
        let upper = spacings.len() / 2;
        match spacings.len() {
            0 => None,
            count if count % 2 == 0 => Some((spacings[upper - 1] + spacings[upper]) / 2.0),
            _ => Some(spacings[upper]),
        }
    }

    fn interpolate_between(
        &self,
        transform: &mut JointPose,
        from_index: usize,
        normalized_time: f32,
    ) {
        match &self.outputs {
            AnimationChanelOutput::Translation(pos) => {
                let before_pos = pos[from_index];
                let after_pos = pos[from_index + 1];

                transform.translation = before_pos.lerp(after_pos, normalized_time);
            }
            AnimationChanelOutput::Rotation(rot) => {
                let before_rot = rot[from_index];
                let after_rot = rot[from_index + 1];

                transform.rotation = before_rot.slerp(after_rot, normalized_time);
            }
            AnimationChanelOutput::Scale(scl) => {
                let before_scl = scl[from_index];
                let after_scl = scl[from_index + 1];

                transform.scale = before_scl.lerp(after_scl, normalized_time);
            }
        }
    }

    fn set_transform_at(&self, transform: &mut JointPose, index: usize) {
        match &self.outputs {
            AnimationChanelOutput::Translation(pos) => {
                transform.translation = pos[index];
            }
            AnimationChanelOutput::Rotation(rots) => {
                transform.rotation = rots[index];
            }
            AnimationChanelOutput::Scale(scl) => {
                // TODO: Handle this better
                transform.scale = scl[index];
            }
        }
    }
}

#[derive(Asset, Serialize, Deserialize)]
pub struct AnimationClip {
    channels: HashMap<Uuid, Vec<AnimationChannel>>,
    duration: f32,
    frame_rate: Option<f32>,
}

impl Default for AnimationClip {
    fn default() -> Self {
        Self {
            channels: HashMap::new(),
            duration: 0.0,
            frame_rate: None,
        }
    }
}

fn snap_rate(rate: f32) -> f32 {
    let whole = rate.round();
    if (rate - whole).abs() <= 0.01 {
        whole
    } else {
        rate
    }
}

impl AnimationClip {
    pub fn add_channel(&mut self, id: Uuid, channel: AnimationChannel) {
        if let Some(channel_duration) = channel.duration() {
            self.duration = self.duration.max(channel_duration);
        }

        self.channels.entry(id).or_default().push(channel);
    }

    pub fn target_ids(&self) -> Keys<'_, Uuid, Vec<AnimationChannel>> {
        self.channels.keys()
    }

    pub fn get_channels(&self, id: &Uuid) -> Option<&Vec<AnimationChannel>> {
        self.channels.get(id)
    }

    pub fn duration(&self) -> f32 {
        self.duration
    }

    /// Estimates the sample rate from the median key spacing of the channel with the most keys.
    pub fn detected_frame_rate(&self) -> Option<f32> {
        self.channels
            .values()
            .flatten()
            .filter_map(|channel| Some((channel.time_samples.len(), channel.median_key_spacing()?)))
            .max_by(|(keys, spacing), (other_keys, other_spacing)| {
                keys.cmp(other_keys)
                    .then_with(|| other_spacing.total_cmp(spacing))
            })
            .map(|(_, spacing)| spacing)
            .filter(|spacing| *spacing > 0.0)
            .map(|spacing| snap_rate(spacing.recip()))
            .filter(|rate| *rate > 0.0)
    }

    /// The authored frame rate, when the source recorded one.
    pub fn frame_rate(&self) -> Option<f32> {
        self.frame_rate
    }

    /// Stores the authored frame rate, snapping a rate within 0.01 of a whole number to it.
    pub fn set_frame_rate(&mut self, frame_rate: f32) {
        debug_assert!(frame_rate.is_finite() && frame_rate > 0.0);
        self.frame_rate = Some(snap_rate(frame_rate));
    }

    /// Writes the velocity of each bone in `bone_ids` between `time - dt` and `time`, both clamped to the clip.
    pub fn sample_velocity(
        &self,
        time: f32,
        dt: f32,
        bone_ids: &[Uuid],
        output: &mut PoseVelocity,
    ) {
        debug_assert_eq!(bone_ids.len(), output.linear.len());
        debug_assert_eq!(bone_ids.len(), output.angular.len());
        let end = time.clamp(0.0, self.duration);
        let start = (time - dt).clamp(0.0, self.duration);
        let elapsed = end - start;
        for (bone, id) in bone_ids.iter().enumerate() {
            if bone >= output.linear.len() || bone >= output.angular.len() {
                break;
            }
            if elapsed <= 0.0 {
                output.linear[bone] = Vec3::ZERO;
                output.angular[bone] = Vec3::ZERO;
                continue;
            }
            let mut before = JointPose {
                translation: Vec3::ZERO,
                rotation: Quat::IDENTITY,
                scale: Vec3::ONE,
            };
            let mut after = before.clone();
            if let Some(channels) = self.channels.get(id) {
                for channel in channels {
                    channel.sample_transform(start, &mut before);
                    channel.sample_transform(end, &mut after);
                }
            }
            output.linear[bone] = (after.translation - before.translation) / elapsed;
            output.angular[bone] = angular_velocity(after.rotation, before.rotation, elapsed);
        }
    }
}

impl LoadableAsset for AnimationClip {}

#[cfg(test)]
mod tests {
    use super::*;

    fn linear_clip(id: Uuid) -> AnimationClip {
        let mut clip = AnimationClip::default();
        clip.add_channel(
            id,
            AnimationChannel::new(
                vec![0.0, 1.0],
                AnimationChanelOutput::Translation(vec![Vec3::ZERO, Vec3::new(2.0, 0.0, -4.0)]),
            ),
        );
        clip
    }

    fn channel_with_keys(time_samples: Vec<f32>) -> AnimationChannel {
        let outputs = vec![Vec3::ZERO; time_samples.len()];
        AnimationChannel::new(time_samples, AnimationChanelOutput::Translation(outputs))
    }

    fn channel_at(rate: f32, keys: u32) -> AnimationChannel {
        channel_with_keys((0..keys).map(|key| key as f32 / rate).collect())
    }

    #[test]
    fn frame_rate_is_none_by_default() {
        assert_eq!(AnimationClip::default().frame_rate(), None);
    }

    #[test]
    fn set_frame_rate_stores_the_rate() {
        let mut clip = AnimationClip::default();
        clip.set_frame_rate(30.0);

        assert_eq!(clip.frame_rate(), Some(30.0));
    }

    #[test]
    fn set_frame_rate_snaps_float_noise_to_a_whole_rate() {
        let mut clip = AnimationClip::default();
        clip.set_frame_rate(30.0003);

        assert_eq!(clip.frame_rate(), Some(30.0));
    }

    #[test]
    fn set_frame_rate_keeps_a_rate_that_is_not_whole() {
        let mut clip = AnimationClip::default();
        clip.set_frame_rate(29.97);

        assert_eq!(clip.frame_rate(), Some(29.97));
    }

    #[test]
    fn detected_frame_rate_is_none_when_the_rate_rounds_to_nothing() {
        let mut clip = AnimationClip::default();
        clip.add_channel(Uuid::from_u128(1), channel_with_keys(vec![0.0, 200.0]));

        assert_eq!(clip.detected_frame_rate(), None);
    }

    #[test]
    fn detected_frame_rate_is_the_reciprocal_of_the_key_spacing() {
        let mut clip = AnimationClip::default();
        clip.add_channel(Uuid::from_u128(1), channel_at(30.0, 31));

        assert_eq!(clip.detected_frame_rate(), Some(30.0));
    }

    #[test]
    fn detected_frame_rate_uses_the_channel_with_the_most_keys() {
        let mut clip = AnimationClip::default();
        clip.add_channel(Uuid::from_u128(1), channel_at(10.0, 4));
        clip.add_channel(Uuid::from_u128(2), channel_at(60.0, 61));
        clip.add_channel(Uuid::from_u128(3), channel_at(24.0, 5));

        assert_eq!(clip.detected_frame_rate(), Some(60.0));
    }

    #[test]
    fn detected_frame_rate_takes_the_middle_of_the_spacings() {
        let mut clip = AnimationClip::default();
        clip.add_channel(
            Uuid::from_u128(1),
            channel_with_keys(vec![0.0, 0.5, 1.0, 5.0, 5.5]),
        );

        assert_eq!(clip.detected_frame_rate(), Some(2.0));
    }

    #[test]
    fn detected_frame_rate_breaks_a_tie_on_the_smallest_spacing_in_any_order() {
        let id = Uuid::from_u128(1);
        let mut coarse_first = AnimationClip::default();
        coarse_first.add_channel(id, channel_at(30.0, 10));
        coarse_first.add_channel(id, channel_at(60.0, 10));
        let mut fine_first = AnimationClip::default();
        fine_first.add_channel(id, channel_at(60.0, 10));
        fine_first.add_channel(id, channel_at(30.0, 10));

        assert_eq!(coarse_first.detected_frame_rate(), Some(60.0));
        assert_eq!(fine_first.detected_frame_rate(), Some(60.0));
    }

    #[test]
    fn detected_frame_rate_rounds_float_noise_to_a_whole_rate() {
        let mut clip = AnimationClip::default();
        clip.add_channel(
            Uuid::from_u128(1),
            channel_with_keys((0..100).map(|key| key as f32 * 0.033_333).collect()),
        );

        assert_eq!(clip.detected_frame_rate(), Some(30.0));
    }

    #[test]
    fn detected_frame_rate_keeps_a_rate_that_is_not_whole() {
        let mut clip = AnimationClip::default();
        clip.add_channel(Uuid::from_u128(1), channel_at(29.97, 100));

        let rate = clip.detected_frame_rate().expect("the clip has keys");
        assert!((rate - 29.97).abs() < 1e-3, "{rate}");
    }

    #[test]
    fn detected_frame_rate_is_none_without_a_channel_of_two_keys() {
        let mut clip = AnimationClip::default();
        assert_eq!(clip.detected_frame_rate(), None);

        clip.add_channel(Uuid::from_u128(1), channel_with_keys(vec![0.5]));
        assert_eq!(clip.detected_frame_rate(), None);
    }

    #[test]
    fn detected_frame_rate_is_none_when_the_spacing_is_not_positive() {
        let mut clip = AnimationClip::default();
        clip.add_channel(
            Uuid::from_u128(1),
            channel_with_keys(vec![0.5, 0.5, 0.5, 0.5]),
        );

        assert_eq!(clip.detected_frame_rate(), None);
    }

    #[test]
    fn sample_velocity_gives_the_slope_of_a_linear_clip() {
        let id = Uuid::from_u128(1);
        let clip = linear_clip(id);
        let mut velocity = PoseVelocity::zero(1);

        clip.sample_velocity(0.5, 0.1, &[id], &mut velocity);

        assert!(
            velocity.linear[0].distance(Vec3::new(2.0, 0.0, -4.0)) < 1e-4,
            "{:?}",
            velocity.linear[0]
        );
        assert!(velocity.angular[0].length() < 1e-4);
    }

    #[test]
    fn sample_velocity_is_one_sided_at_the_clip_start() {
        let id = Uuid::from_u128(1);
        let clip = linear_clip(id);
        let mut velocity = PoseVelocity::zero(1);

        clip.sample_velocity(0.05, 0.1, &[id], &mut velocity);

        assert!(velocity.linear[0].distance(Vec3::new(2.0, 0.0, -4.0)) < 1e-3);
    }

    #[test]
    fn sample_velocity_is_zero_for_a_bone_without_channels() {
        let clip = linear_clip(Uuid::from_u128(1));
        let mut velocity = PoseVelocity::zero(1);

        clip.sample_velocity(0.5, 0.1, &[Uuid::from_u128(2)], &mut velocity);

        assert!(velocity.linear[0].length() < 1e-4);
    }
}
