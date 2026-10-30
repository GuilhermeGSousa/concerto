use concerto_animation::clip::AnimationClip;
use concerto_mesh::{JointPose, Skeleton};

use crate::{database::BakeError, definition::MotionDatabaseDefinition};

/// Where one entry's frames sit in the database and which clip time its first frame samples.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntryPlan {
    pub start: f32,
    pub first_frame: usize,
    pub frames: usize,
}

impl EntryPlan {
    /// The clip time that frame `index` of this entry samples.
    pub fn frame_time(&self, index: usize, rate: f32, speed: f32) -> f32 {
        self.start + index as f32 * speed / rate
    }

    /// The database frames of this entry as a half-open range.
    pub fn range(&self) -> std::ops::Range<usize> {
        self.first_frame..self.first_frame + self.frames
    }
}

/// Checks the rate, speed, clips and ranges of a definition and lays out the frames of every entry.
pub fn plan_entries(
    definition: &MotionDatabaseDefinition,
    skeleton: &Skeleton,
    clips: &[&AnimationClip],
) -> Result<Vec<EntryPlan>, BakeError> {
    if !(definition.rate.is_finite() && definition.rate > 0.0) {
        return Err(BakeError::InvalidRate(definition.rate));
    }
    if !(definition.speed.is_finite() && definition.speed > 0.0) {
        return Err(BakeError::InvalidSpeed(definition.speed));
    }
    if clips.len() != definition.entries.len() {
        return Err(BakeError::ClipCountMismatch {
            expected: definition.entries.len(),
            actual: clips.len(),
        });
    }

    let mut plans = Vec::with_capacity(clips.len());
    let mut first_frame = 0;
    for (entry, (spec, clip)) in definition.entries.iter().zip(clips).enumerate() {
        if !skeleton
            .bone_ids
            .iter()
            .any(|id| clip.get_channels(id).is_some())
        {
            return Err(BakeError::ClipHasNoBones { entry });
        }
        let (start, end) = match spec.range {
            Some(range) => (range.start, range.end),
            None => (0.0, clip.duration()),
        };
        let valid = start.is_finite()
            && end.is_finite()
            && start >= 0.0
            && end <= clip.duration()
            && start < end;
        if !valid {
            return Err(BakeError::InvalidRange { entry });
        }
        let frames = ((end - start) * definition.rate / definition.speed).floor() as usize;
        if frames < 2 {
            return Err(BakeError::RangeTooShort { entry, frames });
        }
        plans.push(EntryPlan {
            start,
            first_frame,
            frames,
        });
        first_frame += frames;
    }
    Ok(plans)
}

/// The skeleton's rest pose with every bone's channels in `clip` sampled at `time`.
pub fn sample_pose(skeleton: &Skeleton, clip: &AnimationClip, time: f32) -> Vec<JointPose> {
    let mut pose = skeleton.rest_pose.to_vec();
    for (joint, id) in pose.iter_mut().zip(skeleton.bone_ids.iter()) {
        if let Some(channels) = clip.get_channels(id) {
            for channel in channels {
                channel.sample_transform(time, joint);
            }
        }
    }
    pose
}
