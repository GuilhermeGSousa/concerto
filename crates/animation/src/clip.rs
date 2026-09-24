use std::collections::{HashMap, hash_map::Keys};

use concerto_core::assets::{Asset, LoadableAsset};
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

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
}

impl LoadableAsset for AnimationClip {}
