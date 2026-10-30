use std::fmt;

use concerto_foundation::assets::{Asset, LoadableAsset};
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

/// Where one feature channel's values sit in a frame's feature vector.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelSpan {
    pub type_name: String,
    pub offset: usize,
    pub dims: usize,
}

/// One frame's local pose and velocities, one entry per bone.
#[derive(Clone, Copy, Debug)]
pub struct PoseView<'a> {
    pub positions: &'a [Vec3],
    pub rotations: &'a [Quat],
    pub velocities: &'a [Vec3],
    pub angular_velocities: &'a [Vec3],
}

/// Baked animation frames, dense and frame-major; bone 0 is the simulation bone and bone `i + 1` is skeleton bone `i`.
#[derive(Asset, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MotionDatabase {
    pub names: Vec<String>,
    pub parents: Vec<Option<u16>>,
    pub rate: f32,
    pub bone_count: usize,
    pub frame_count: usize,
    pub positions: Vec<Vec3>,
    pub rotations: Vec<Quat>,
    pub velocities: Vec<Vec3>,
    pub angular_velocities: Vec<Vec3>,
    pub contact_bones: Vec<u16>,
    pub contacts: Vec<bool>,
    pub range_starts: Vec<usize>,
    pub range_stops: Vec<usize>,
    pub features: Vec<f32>,
    pub feature_dims: usize,
    pub channels: Vec<ChannelSpan>,
}

impl MotionDatabase {
    /// The local pose and velocities of `frame`.
    pub fn pose(&self, frame: usize) -> PoseView<'_> {
        let start = frame * self.bone_count;
        let end = start + self.bone_count;
        PoseView {
            positions: &self.positions[start..end],
            rotations: &self.rotations[start..end],
            velocities: &self.velocities[start..end],
            angular_velocities: &self.angular_velocities[start..end],
        }
    }

    /// Whether each contact bone touches the ground in `frame`, in `contact_bones` order.
    pub fn contacts(&self, frame: usize) -> &[bool] {
        let count = self.contact_bones.len();
        &self.contacts[frame * count..(frame + 1) * count]
    }

    /// The feature vector of `frame`.
    pub fn features(&self, frame: usize) -> &[f32] {
        let dims = self.feature_dims;
        &self.features[frame * dims..(frame + 1) * dims]
    }

    /// The index of the entry whose range contains `frame`.
    pub fn range_of(&self, frame: usize) -> Option<usize> {
        self.range_starts
            .iter()
            .zip(&self.range_stops)
            .position(|(start, stop)| (*start..*stop).contains(&frame))
    }
}

impl LoadableAsset for MotionDatabase {}

/// Why a motion database could not be baked.
#[derive(Clone, Debug, PartialEq)]
pub enum BakeError {
    MissingJoint(String),
    ClipHasNoBones { entry: usize },
    ClipCountMismatch { expected: usize, actual: usize },
    UnregisteredChannel(String),
    ChannelValue { type_name: String, reason: String },
    RangeTooShort { entry: usize, frames: usize },
    InvalidRange { entry: usize },
    InvalidWindow { name: &'static str, window: usize },
    InvalidRate(f32),
    InvalidSpeed(f32),
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
        }
    }
}

impl std::error::Error for BakeError {}
