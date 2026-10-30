use concerto_animation::clip::AnimationClip;
use concerto_foundation::assets::{handle::AssetHandle, Asset, LoadableAsset};
use concerto_mesh::Skeleton;
use concerto_reflect::{Reflect, ReflectedValue};
use glam::Vec3;
use serde::{Deserialize, Serialize};

/// A span of a clip in seconds, from `start` to `end`.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TimeRange {
    pub start: f32,
    pub end: f32,
}

/// One clip of a motion database and the part of it to bake; no range means the whole clip.
#[derive(Reflect, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MotionEntry {
    pub clip: AssetHandle<AnimationClip>,
    pub range: Option<TimeRange>,
}

/// Which joints place and orient the simulation bone, and how much to smooth it.
#[derive(Reflect, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SimulationBoneSettings {
    pub position_joint: String,
    pub facing_joint: String,
    pub facing_axis: Vec3,
    pub position_window: usize,
    pub facing_window: usize,
}

impl Default for SimulationBoneSettings {
    fn default() -> Self {
        Self {
            position_joint: "Spine2".into(),
            facing_joint: "Hips".into(),
            facing_axis: Vec3::Z,
            position_window: 31,
            facing_window: 61,
        }
    }
}

/// The bones whose ground contact is baked, the speed below which one touches, and the median filter window.
#[derive(Reflect, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContactSettings {
    pub bones: Vec<String>,
    pub speed_threshold: f32,
    pub median_window: usize,
}

impl Default for ContactSettings {
    fn default() -> Self {
        Self {
            bones: Vec::new(),
            speed_threshold: 0.15,
            median_window: 6,
        }
    }
}

/// How the trajectory a channel sees is sampled: `sample_count` points, `sample_dt` seconds apart, starting one step ahead.
#[derive(Reflect, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrajectorySettings {
    pub sample_dt: f32,
    pub sample_count: usize,
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

/// What to bake into a [`MotionDatabase`](crate::MotionDatabase): the clips, sampling, simulation bone, contacts and feature channels.
#[derive(Asset, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MotionDatabaseDefinition {
    pub skeleton: AssetHandle<Skeleton>,
    pub entries: Vec<MotionEntry>,
    pub rate: f32,
    pub speed: f32,
    pub simulation: SimulationBoneSettings,
    pub contacts: ContactSettings,
    pub trajectory: TrajectorySettings,
    pub channels: Vec<StoredChannel>,
}

impl Default for MotionDatabaseDefinition {
    fn default() -> Self {
        Self {
            skeleton: AssetHandle::default(),
            entries: Vec::new(),
            rate: 60.0,
            speed: 1.1,
            simulation: SimulationBoneSettings::default(),
            contacts: ContactSettings::default(),
            trajectory: TrajectorySettings::default(),
            channels: Vec::new(),
        }
    }
}

impl LoadableAsset for MotionDatabaseDefinition {}
