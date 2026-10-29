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
}

impl Default for AnimationClip {
    fn default() -> Self {
        Self {
            channels: HashMap::new(),
            duration: 0.0,
        }
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
