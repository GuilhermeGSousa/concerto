use std::fmt;
use std::ops::Range;

use concerto_animation::clip::AnimationClip;
use concerto_foundation::assets::{handle::AssetHandle, Asset, LoadableAsset};
use concerto_foundation::transform::Transform;
use concerto_mesh::{JointPose, Skeleton};
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use concerto_reflect::ReflectedValue;

use crate::bake::{model_pose, reexpress_roots, sample_pose};
use crate::channel::ChannelSource;
use crate::definition::TrajectorySettings;

/// Where one feature channel's values sit in a frame's feature vector.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChannelSpan {
    pub type_name: String,
    pub offset: usize,
    pub dims: usize,
    pub weight: f32,
    pub source: ChannelSource,
    pub value: ReflectedValue,
}

/// Baked animation frames: per-frame features and simulation bone, and where each frame plays in its clip.
#[derive(Asset, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MotionDatabase {
    skeleton: AssetHandle<Skeleton>,
    bone_count: usize,
    clips: Vec<AssetHandle<AnimationClip>>,
    rate: f32,
    speed: f32,
    trajectory: TrajectorySettings,
    frame_count: usize,
    range_starts: Vec<usize>,
    range_stops: Vec<usize>,
    clip_starts: Vec<f32>,
    simulation: Vec<(Vec3, Quat)>,
    features: Vec<f32>,
    feature_dims: usize,
    channels: Vec<ChannelSpan>,
    offset: Vec<f32>,
    deviations: Vec<f32>,
}

const DEVIATION_FLOOR: f32 = 1e-6;

impl MotionDatabase {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        skeleton: AssetHandle<Skeleton>,
        bone_count: usize,
        clips: Vec<AssetHandle<AnimationClip>>,
        rate: f32,
        speed: f32,
        trajectory: TrajectorySettings,
        frame_count: usize,
        range_starts: Vec<usize>,
        range_stops: Vec<usize>,
        clip_starts: Vec<f32>,
        simulation: Vec<(Vec3, Quat)>,
        features: Vec<f32>,
        feature_dims: usize,
        channels: Vec<ChannelSpan>,
        offset: Vec<f32>,
        deviations: Vec<f32>,
    ) -> Self {
        Self {
            skeleton,
            bone_count,
            clips,
            rate,
            speed,
            trajectory,
            frame_count,
            range_starts,
            range_stops,
            clip_starts,
            simulation,
            features,
            feature_dims,
            channels,
            offset,
            deviations,
        }
    }

    /// The number of baked frames.
    pub fn frame_count(&self) -> usize {
        self.frame_count
    }

    /// The skeleton the database was baked for.
    pub fn skeleton(&self) -> &AssetHandle<Skeleton> {
        &self.skeleton
    }

    /// The skeleton's bone count at bake time.
    pub fn bone_count(&self) -> usize {
        self.bone_count
    }

    /// One clip per entry, in entry order.
    pub fn clips(&self) -> &[AssetHandle<AnimationClip>] {
        &self.clips
    }

    /// Database frames per second.
    pub fn rate(&self) -> f32 {
        self.rate
    }

    /// Clip seconds per database second.
    pub fn speed(&self) -> f32 {
        self.speed
    }

    /// How the trajectory is sampled for the channels.
    pub fn trajectory(&self) -> &TrajectorySettings {
        &self.trajectory
    }

    /// The length of one feature vector.
    pub fn feature_dims(&self) -> usize {
        self.feature_dims
    }

    /// Where each feature channel sits in a feature vector.
    pub fn channels(&self) -> &[ChannelSpan] {
        &self.channels
    }

    /// The index of the entry whose range contains `frame`.
    pub fn entry_of(&self, frame: usize) -> Option<usize> {
        self.range_starts
            .iter()
            .zip(&self.range_stops)
            .position(|(start, stop)| (*start..*stop).contains(&frame))
    }

    /// The database frames of the entry that contains `frame`.
    pub fn range_of(&self, frame: usize) -> Option<Range<usize>> {
        let entry = self.entry_of(frame)?;
        Some(self.range_starts[entry]..self.range_stops[entry])
    }

    /// The entry of `frame` and the clip time in seconds that the frame plays.
    pub fn clip_time(&self, frame: usize) -> Option<(usize, f32)> {
        let entry = self.entry_of(frame)?;
        let offset = (frame - self.range_starts[entry]) as f32 * self.speed / self.rate;
        Some((entry, self.clip_starts[entry] + offset))
    }

    /// The local pose of `frame`, sampled from its clip with each parentless bone re-expressed under the simulation bone.
    pub fn pose(
        &self,
        frame: usize,
        skeleton: &Skeleton,
        clips: &[&AnimationClip],
    ) -> Result<Vec<JointPose>, BakeError> {
        if skeleton.bone_count() != self.bone_count {
            return Err(BakeError::BoneCountMismatch {
                expected: self.bone_count,
                actual: skeleton.bone_count(),
            });
        }
        if clips.len() != self.clips.len() {
            return Err(BakeError::ClipCountMismatch {
                expected: self.clips.len(),
                actual: clips.len(),
            });
        }
        let (entry, time) = self.clip_time(frame).ok_or(BakeError::FrameOutOfRange {
            frame,
            frame_count: self.frame_count,
        })?;
        let mut local = sample_pose(skeleton, clips[entry], time);
        let model = model_pose(skeleton, &local);
        reexpress_roots(skeleton, &mut local, &model, self.simulation(frame));
        Ok(local)
    }

    /// The model-space simulation bone of `frame`.
    pub fn simulation(&self, frame: usize) -> Transform {
        let (translation, rotation) = self.simulation[frame];
        Transform::from_translation_rotation(translation, rotation)
    }

    /// The feature vector of `frame`.
    pub fn features(&self, frame: usize) -> &[f32] {
        let dims = self.feature_dims;
        &self.features[frame * dims..(frame + 1) * dims]
    }

    /// The mean of every feature dimension over all frames.
    pub fn offset(&self) -> &[f32] {
        &self.offset
    }

    /// The divisor of every feature dimension: its group's floored deviation over the group's weight.
    pub fn scales(&self) -> Vec<f32> {
        let mut scales = vec![1.0; self.feature_dims];
        for (span, deviation) in self.channels.iter().zip(&self.deviations) {
            let scale = deviation.max(DEVIATION_FLOOR) / span.weight;
            scales[span.offset..span.offset + span.dims].fill(scale);
        }
        scales
    }

    /// Subtracts the offset and divides by the scale in place; a group with no deviation becomes 0.
    pub fn normalise(&self, features: &mut [f32]) {
        let scales = self.scales();
        for (span, deviation) in self.channels.iter().zip(&self.deviations) {
            let range = span.offset..span.offset + span.dims;
            if *deviation < DEVIATION_FLOOR {
                features[range].fill(0.0);
                continue;
            }
            for dim in range {
                features[dim] = (features[dim] - self.offset[dim]) / scales[dim];
            }
        }
    }

    /// The feature vector of `frame` after `normalise`.
    pub fn normalised_features(&self, frame: usize) -> Vec<f32> {
        let mut features = self.features(frame).to_vec();
        self.normalise(&mut features);
        features
    }

    /// Replaces every channel's weight, one per channel in order, without a re-bake.
    pub fn set_weights(&mut self, weights: &[f32]) -> Result<(), BakeError> {
        if weights.len() != self.channels.len() {
            return Err(BakeError::WeightCountMismatch {
                expected: self.channels.len(),
                actual: weights.len(),
            });
        }
        if let Some(weight) = weights
            .iter()
            .find(|weight| !(weight.is_finite() && **weight > 0.0))
        {
            return Err(BakeError::InvalidWeight(*weight));
        }
        for (span, weight) in self.channels.iter_mut().zip(weights) {
            span.weight = *weight;
        }
        Ok(())
    }
}

impl LoadableAsset for MotionDatabase {}

/// Why a motion database could not be baked, or a baked frame's pose could not be sampled.
#[derive(Clone, Debug, PartialEq)]
pub enum BakeError {
    MissingJoint(String),
    ClipHasNoBones { entry: usize },
    ClipCountMismatch { expected: usize, actual: usize },
    BoneCountMismatch { expected: usize, actual: usize },
    FrameOutOfRange { frame: usize, frame_count: usize },
    UnregisteredChannel(String),
    ChannelValue { type_name: String, reason: String },
    RangeTooShort { entry: usize, frames: usize },
    InvalidRange { entry: usize },
    InvalidWindow { name: &'static str, window: usize },
    InvalidRate(f32),
    InvalidSpeed(f32),
    InvalidSampleDt(f32),
    InvalidWeight(f32),
    WeightCountMismatch { expected: usize, actual: usize },
    ChannelCountMismatch { expected: usize, actual: usize },
    TrajectoryTooShort { needed: usize, actual: usize },
}

impl fmt::Display for BakeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BakeError::MissingJoint(name) => write!(f, "the skeleton has no joint `{name}`"),
            BakeError::ClipHasNoBones { entry } => {
                write!(
                    f,
                    "the clip of entry {entry} animates none of the skeleton's bones"
                )
            }
            BakeError::ClipCountMismatch { expected, actual } => {
                write!(f, "{actual} clips given for {expected} entries")
            }
            BakeError::BoneCountMismatch { expected, actual } => {
                write!(f, "the database has {expected} bones, {actual} given")
            }
            BakeError::FrameOutOfRange { frame, frame_count } => {
                write!(f, "frame {frame} is outside the {frame_count} baked frames")
            }
            BakeError::UnregisteredChannel(name) => {
                write!(f, "feature channel type `{name}` is not registered")
            }
            BakeError::ChannelValue { type_name, reason } => {
                write!(f, "feature channel `{type_name}` cannot be built: {reason}")
            }
            BakeError::RangeTooShort { entry, frames } => {
                write!(
                    f,
                    "entry {entry} bakes {frames} frames, at least 2 are needed"
                )
            }
            BakeError::InvalidRange { entry } => {
                write!(
                    f,
                    "entry {entry} has a range outside its clip or with start at or after end"
                )
            }
            BakeError::InvalidWindow { name, window } => {
                write!(f, "{name} window {window} must be odd and greater than 3")
            }
            BakeError::InvalidRate(rate) => write!(f, "rate {rate} must be positive"),
            BakeError::InvalidSpeed(speed) => write!(f, "speed {speed} must be positive"),
            BakeError::InvalidSampleDt(dt) => {
                write!(f, "trajectory sample_dt {dt} must be positive")
            }
            BakeError::InvalidWeight(weight) => {
                write!(f, "channel weight {weight} must be positive and finite")
            }
            BakeError::WeightCountMismatch { expected, actual } => {
                write!(f, "{actual} weights given for {expected} channels")
            }
            BakeError::ChannelCountMismatch { expected, actual } => {
                write!(f, "the database has {actual} channels, {expected} expected")
            }
            BakeError::TrajectoryTooShort { needed, actual } => {
                write!(
                    f,
                    "the trajectory has {actual} samples, a channel reads {needed}"
                )
            }
        }
    }
}

impl std::error::Error for BakeError {}
