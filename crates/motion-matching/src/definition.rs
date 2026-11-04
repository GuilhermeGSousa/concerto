use concerto_animation::clip::AnimationClip;
use concerto_foundation::assets::{handle::AssetHandle, Asset, CreatableAsset, LoadableAsset};
use concerto_mesh::{JointName, Skeleton};
use concerto_reflect::{Reflect, ReflectedValue};
use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::features::reference_channels;

/// A span of a clip in seconds, from `start` to `end`.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TimeRange {
    pub start: f32,
    pub end: f32,
}

/// One clip of a motion database, the part of it to bake, and whether it plays mirrored; no range means the whole clip.
#[derive(Reflect, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MotionEntry {
    pub clip: AssetHandle<AnimationClip>,
    pub range: Option<TimeRange>,
    pub mirrored: bool,
}

/// A signed axis of a joint's local space.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum JointAxis {
    X,
    NegX,
    #[default]
    Y,
    NegY,
    Z,
    NegZ,
}

impl JointAxis {
    /// The unit vector along this axis.
    pub fn vector(self) -> Vec3 {
        match self {
            JointAxis::X => Vec3::X,
            JointAxis::NegX => Vec3::NEG_X,
            JointAxis::Y => Vec3::Y,
            JointAxis::NegY => Vec3::NEG_Y,
            JointAxis::Z => Vec3::Z,
            JointAxis::NegZ => Vec3::NEG_Z,
        }
    }
}

/// The horizontal model-space axis normal to the plane that mirrored entries are mirrored across.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MirrorAxis {
    #[default]
    X,
    Z,
}

impl MirrorAxis {
    /// The unit normal of the mirror plane.
    pub fn normal(self) -> Vec3 {
        match self {
            MirrorAxis::X => Vec3::X,
            MirrorAxis::Z => Vec3::Z,
        }
    }
}

/// Which joints place and orient the simulation bone, and how much to smooth it.
#[derive(Reflect, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SimulationBoneSettings {
    pub position_joint: JointName,
    pub facing_joint: JointName,
    pub facing_axis: JointAxis,
    pub position_window: usize,
    pub facing_window: usize,
}

impl Default for SimulationBoneSettings {
    fn default() -> Self {
        Self {
            position_joint: "Spine2".into(),
            facing_joint: "Hips".into(),
            facing_axis: JointAxis::Y,
            position_window: 31,
            facing_window: 61,
        }
    }
}

/// How the trajectory a channel sees is sampled: `sample_count` points, `sample_dt` seconds apart, starting one step ahead.
#[derive(Reflect, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrajectorySettings {
    pub sample_dt: f32,
    pub sample_count: usize,
}

impl TrajectorySettings {
    /// How many frames ahead sample `sample` sits at `rate`, rounded to a whole frame.
    pub fn frame_offset(&self, sample: usize, rate: f32) -> usize {
        (self.sample_dt * (sample + 1) as f32 * rate).round() as usize
    }
}

impl Default for TrajectorySettings {
    fn default() -> Self {
        Self {
            sample_dt: 1.0 / 3.0,
            sample_count: 3,
        }
    }
}

/// A feature channel as stored on a definition: its registered type name and its reflected value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredChannel {
    pub type_name: String,
    pub value: ReflectedValue,
}

/// What to bake into a [`MotionDatabase`](crate::MotionDatabase): the clips, sampling, simulation bone, trajectory and feature channels.
#[derive(Asset, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[asset(version = 2)]
pub struct MotionDatabaseDefinition {
    pub skeleton: AssetHandle<Skeleton>,
    pub entries: Vec<MotionEntry>,
    pub rate: f32,
    pub speed: f32,
    pub simulation: SimulationBoneSettings,
    pub trajectory: TrajectorySettings,
    pub channels: Vec<StoredChannel>,
    pub mirror_axis: MirrorAxis,
}

impl Default for MotionDatabaseDefinition {
    fn default() -> Self {
        Self {
            skeleton: AssetHandle::default(),
            entries: Vec::new(),
            rate: 60.0,
            speed: 1.1,
            simulation: SimulationBoneSettings::default(),
            trajectory: TrajectorySettings::default(),
            channels: reference_channels(),
            mirror_axis: MirrorAxis::X,
        }
    }
}

impl CreatableAsset for MotionDatabaseDefinition {
    fn new_asset() -> Self {
        Self::default()
    }
}

impl LoadableAsset for MotionDatabaseDefinition {}
