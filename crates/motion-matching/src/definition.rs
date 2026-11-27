use concerto_animation::clip::AnimationClip;
use concerto_foundation::assets::handle::AssetHandle;
use concerto_mesh::JointName;
use concerto_reflect::Reflect;
use glam::Vec3;
use serde::{Deserialize, Serialize};

/// A span of a clip in seconds, from `start` to `end`.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TimeRange {
    /// The start, in seconds from the clip's start.
    pub start: f32,
    /// The end, in seconds from the clip's start.
    pub end: f32,
}

/// One clip of a motion database, the part of it to bake, and whether it plays mirrored.
///
/// No range means the whole clip.
#[derive(Reflect, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MotionEntry {
    /// The clip to bake.
    pub clip: AssetHandle<AnimationClip>,
    /// The span of the clip to bake; the whole clip when absent.
    pub range: Option<TimeRange>,
    /// Whether the entry is baked mirrored across the database's mirror plane.
    pub mirrored: bool,
}

/// A signed axis of a joint's local space.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum JointAxis {
    /// The joint's +X axis.
    X,
    /// The joint's -X axis.
    NegX,
    /// The joint's +Y axis.
    #[default]
    Y,
    /// The joint's -Y axis.
    NegY,
    /// The joint's +Z axis.
    Z,
    /// The joint's -Z axis.
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
    /// The plane `x = 0`.
    #[default]
    X,
    /// The plane `z = 0`.
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
    /// The joint whose model-space position, projected onto the ground and smoothed, places the
    /// simulation bone.
    pub position_joint: JointName,
    /// The joint whose facing, flattened onto the ground and smoothed, turns the simulation bone.
    pub facing_joint: JointName,
    /// The axis of `facing_joint`'s model-space rotation the character faces along.
    pub facing_axis: JointAxis,
    /// The width, in frames, of the filter that smooths the position; odd and greater than 3.
    pub position_window: usize,
    /// The width, in frames, of the filter that smooths the facing; odd and greater than 3.
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

/// How the trajectory a channel sees is sampled.
///
/// It has `sample_count` points, `sample_dt` seconds apart, starting one step
/// ahead of the current frame.
#[derive(Reflect, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrajectorySettings {
    /// The time between samples, in seconds.
    pub sample_dt: f32,
    /// The number of samples.
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
