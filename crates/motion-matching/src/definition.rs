use concerto_animation::clip::AnimationClip;
use concerto_foundation::assets::{
    asset_server::AssetLoadContext, handle::AssetHandle, Asset, CreatableAsset, LoadableAsset,
};
use concerto_mesh::{JointName, Skeleton};
use concerto_reflect::{Reflect, TypeRegistry};
use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::{
    channel::{FeatureChannel, FeatureChannelData},
    database::BakeError,
    features::reference_channels,
};

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
    pub position_joint: JointName,
    pub facing_joint: JointName,
    pub facing_axis: Vec3,
    pub position_window: usize,
    pub facing_window: usize,
}

impl Default for SimulationBoneSettings {
    fn default() -> Self {
        Self {
            position_joint: "Spine2".into(),
            facing_joint: "Hips".into(),
            facing_axis: Vec3::Y,
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

/// What to bake into a [`MotionDatabase`](crate::MotionDatabase): the clips, sampling, simulation bone, trajectory and feature channels.
#[derive(Asset, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MotionDatabaseDefinition {
    pub skeleton: AssetHandle<Skeleton>,
    pub entries: Vec<MotionEntry>,
    pub rate: f32,
    pub speed: f32,
    pub simulation: SimulationBoneSettings,
    pub trajectory: TrajectorySettings,
    pub channels: Vec<FeatureChannelData>,
}

impl MotionDatabaseDefinition {
    /// Builds every stored channel into a live one; the first channel that can't be built is the error.
    pub fn load_channels(&mut self, registry: &TypeRegistry) -> Result<(), BakeError> {
        self.channels
            .iter_mut()
            .try_for_each(|channel| channel.load(registry))
    }

    /// The live channels, in order; a channel not yet loaded is an error.
    pub fn live_channels(&self) -> Result<Vec<&dyn FeatureChannel>, BakeError> {
        self.channels
            .iter()
            .map(FeatureChannelData::channel)
            .collect()
    }
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
        }
    }
}

impl CreatableAsset for MotionDatabaseDefinition {
    fn new_asset() -> Self {
        Self::default()
    }
}

impl LoadableAsset for MotionDatabaseDefinition {
    fn on_load(&mut self, context: &AssetLoadContext) -> anyhow::Result<()> {
        let registry = context.type_registry().read();
        for channel in &mut self.channels {
            if let Err(error) = channel.load(&registry) {
                log::warn!(
                    "motion database definition {:?}: {error}",
                    context.asset_id()
                );
            }
        }
        Ok(())
    }
}
