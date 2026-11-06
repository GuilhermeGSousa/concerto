use std::fmt;
use std::ops::Range;

use anyhow::Context;
use concerto_animation::clip::AnimationClip;
use concerto_animation::mirror::{MirrorError, PoseMirror};
use concerto_foundation::assets::{
    asset_server::AssetLoadContext,
    derived::{DerivedData, DerivedInputs},
    handle::AssetHandle,
    Asset, AssetId, CreatableAsset, LoadableAsset,
};
use concerto_foundation::math::spring_damper::angular_velocity;
use concerto_foundation::transform::Transform;
use concerto_mesh::{JointPose, Skeleton};
use concerto_reflect::TypeRegistry;
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::bake::{bake_with_progress, model_pose, reexpress_roots, sample_pose};
use crate::channel::{BodyMotion, FeatureChannel, FeatureChannelData};
use crate::definition::{MirrorAxis, MotionEntry, SimulationBoneSettings, TrajectorySettings};
use crate::features::reference_channels;
use crate::simulation::TrajectorySample;
use crate::staleness::weightless_hash;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Span {
    offset: usize,
    dims: usize,
}

impl Span {
    pub(crate) fn new(offset: usize, dims: usize) -> Self {
        Self { offset, dims }
    }

    pub(crate) fn dims(&self) -> usize {
        self.dims
    }

    pub(crate) fn range(&self) -> Range<usize> {
        self.offset..self.offset + self.dims
    }
}

/// What a bake computes from a motion database's inputs: per-frame features and simulation bone, where each entry's frames sit, and the normalization.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BakedDatabase {
    bone_count: usize,
    frame_count: usize,
    range_starts: Vec<usize>,
    range_stops: Vec<usize>,
    clip_starts: Vec<f32>,
    simulation: Vec<(Vec3, Quat)>,
    features: Vec<f32>,
    feature_dims: usize,
    spans: Vec<Span>,
    offset: Vec<f32>,
    deviations: Vec<f32>,
}

impl BakedDatabase {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        bone_count: usize,
        frame_count: usize,
        range_starts: Vec<usize>,
        range_stops: Vec<usize>,
        clip_starts: Vec<f32>,
        simulation: Vec<(Vec3, Quat)>,
        features: Vec<f32>,
        feature_dims: usize,
        spans: Vec<Span>,
        offset: Vec<f32>,
        deviations: Vec<f32>,
    ) -> Self {
        Self {
            bone_count,
            frame_count,
            range_starts,
            range_stops,
            clip_starts,
            simulation,
            features,
            feature_dims,
            spans,
            offset,
            deviations,
        }
    }

    fn fits(&self, database: &MotionDatabase) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.spans.len() == database.channels.len(),
            "the blob has {} channels, the database {}",
            self.spans.len(),
            database.channels.len()
        );
        anyhow::ensure!(
            self.range_starts.len() == database.entries.len(),
            "the blob has {} entries, the database {}",
            self.range_starts.len(),
            database.entries.len()
        );
        Ok(())
    }

    fn ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.range_starts
            .iter()
            .zip(&self.range_stops)
            .map(|(start, stop)| *start..*stop)
    }

    fn entry_of(&self, frame: usize) -> Option<usize> {
        self.ranges().position(|range| range.contains(&frame))
    }

    fn range_of(&self, frame: usize) -> Option<Range<usize>> {
        self.ranges().nth(self.entry_of(frame)?)
    }

    fn simulation(&self, frame: usize) -> Option<Transform> {
        let (translation, rotation) = self.simulation.get(frame)?;
        Some(Transform::from_translation_rotation(
            *translation,
            *rotation,
        ))
    }
}

/// A motion database: the clips, sampling, simulation bone, trajectory and feature channels it is built from, and once derived, the baked frames.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MotionDatabase {
    pub skeleton: AssetHandle<Skeleton>,
    pub entries: Vec<MotionEntry>,
    pub rate: f32,
    pub speed: f32,
    pub simulation: SimulationBoneSettings,
    pub trajectory: TrajectorySettings,
    pub channels: Vec<FeatureChannelData>,
    pub mirror_axis: MirrorAxis,
    #[serde(skip)]
    pub baked: Option<BakedDatabase>,
}

const DEVIATION_FLOOR: f32 = 1e-6;

impl Default for MotionDatabase {
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
            baked: None,
        }
    }
}

impl MotionDatabase {
    /// This database with `baked` as its computed part, for code that bakes inline.
    pub fn with_baked(mut self, baked: BakedDatabase) -> Self {
        self.baked = Some(baked);
        self
    }

    /// Whether the computed part is present.
    pub fn is_baked(&self) -> bool {
        self.baked.is_some()
    }

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

    /// Whether every loaded channel's weight is positive and finite, which a search needs.
    pub fn check_weights(&self) -> Result<(), BakeError> {
        self.channels
            .iter()
            .filter_map(|channel| channel.channel().ok())
            .map(|channel| channel.weight())
            .find(|weight| !(weight.is_finite() && *weight > 0.0))
            .map_or(Ok(()), |weight| Err(BakeError::InvalidWeight(weight)))
    }

    /// One clip per entry, in entry order.
    pub fn clips(&self) -> impl Iterator<Item = &AssetHandle<AnimationClip>> + '_ {
        self.entries.iter().map(|entry| &entry.clip)
    }

    /// The number of baked frames.
    pub fn frame_count(&self) -> Option<usize> {
        Some(self.baked.as_ref()?.frame_count)
    }

    /// The skeleton's bone count at bake time.
    pub fn bone_count(&self) -> Option<usize> {
        Some(self.baked.as_ref()?.bone_count)
    }

    /// The length of one feature vector.
    pub fn feature_dims(&self) -> Option<usize> {
        Some(self.baked.as_ref()?.feature_dims)
    }

    /// The feature indices channel `channel` wrote.
    pub fn channel_range(&self, channel: usize) -> Option<Range<usize>> {
        Some(self.baked.as_ref()?.spans.get(channel)?.range())
    }

    /// The database frames of every entry, in entry order.
    pub fn ranges(&self) -> Option<impl Iterator<Item = Range<usize>> + '_> {
        Some(self.baked.as_ref()?.ranges())
    }

    /// The number of frames entry `index` baked.
    pub fn frames_of_entry(&self, index: usize) -> Option<usize> {
        self.ranges()?.nth(index).map(|range| range.len())
    }

    /// The index of the entry whose range contains `frame`.
    pub fn entry_of(&self, frame: usize) -> Option<usize> {
        self.baked.as_ref()?.entry_of(frame)
    }

    /// The database frames of the entry that contains `frame`.
    pub fn range_of(&self, frame: usize) -> Option<Range<usize>> {
        self.baked.as_ref()?.range_of(frame)
    }

    /// The simulation bone's velocities at `frame`, by central difference and one-sided at the ends of its range.
    pub fn body_motion(&self, frame: usize) -> Option<BodyMotion> {
        let range = self.range_of(frame)?;
        let previous = if frame > range.start {
            frame - 1
        } else {
            frame
        };
        let next = if frame + 1 < range.end {
            frame + 1
        } else {
            frame
        };
        if previous == next {
            return Some(BodyMotion::default());
        }
        let dt = (next - previous) as f32 / self.rate;
        let previous = self.simulation_bone(previous)?;
        let next = self.simulation_bone(next)?;
        Some(BodyMotion {
            velocity: (next.translation - previous.translation) / dt,
            angular_velocity: angular_velocity(next.rotation, previous.rotation, dt),
        })
    }

    /// The trajectory samples of `frame` as the bake saw them, in sample order and clamped to its range's last frame.
    pub fn trajectory_samples(&self, frame: usize) -> Option<Vec<TrajectorySample>> {
        let range = self.range_of(frame)?;
        (0..self.trajectory.sample_count)
            .map(|sample| {
                let target = frame
                    .saturating_add(self.trajectory.frame_offset(sample, self.rate))
                    .min(range.end - 1);
                let simulation = self.simulation_bone(target)?;
                Some(TrajectorySample {
                    position: simulation.translation,
                    velocity: self.body_motion(target)?.velocity,
                    rotation: simulation.rotation,
                })
            })
            .collect()
    }

    /// The entry of `frame` and the clip time in seconds that the frame plays.
    pub fn clip_time(&self, frame: usize) -> Option<(usize, f32)> {
        let baked = self.baked.as_ref()?;
        let entry = baked.entry_of(frame)?;
        let offset = (frame - baked.range_starts[entry]) as f32 * self.speed / self.rate;
        Some((entry, baked.clip_starts[entry] + offset))
    }

    /// The local pose of `frame`, sampled from its clip with each parentless bone re-expressed under the simulation bone.
    pub fn pose(
        &self,
        frame: usize,
        skeleton: &Skeleton,
        clips: &[&AnimationClip],
    ) -> Result<Vec<JointPose>, BakeError> {
        let baked = self.baked.as_ref().ok_or(BakeError::NotBaked)?;
        if skeleton.bone_count() != baked.bone_count {
            return Err(BakeError::BoneCountMismatch {
                expected: baked.bone_count,
                actual: skeleton.bone_count(),
            });
        }
        if clips.len() != self.entries.len() {
            return Err(BakeError::ClipCountMismatch {
                expected: self.entries.len(),
                actual: clips.len(),
            });
        }
        let out_of_range = BakeError::FrameOutOfRange {
            frame,
            frame_count: baked.frame_count,
        };
        let (entry, time) = self.clip_time(frame).ok_or(out_of_range.clone())?;
        let clip = clips.get(entry).ok_or(out_of_range.clone())?;
        let simulation = baked.simulation(frame).ok_or(out_of_range)?;
        let mut local = sample_pose(skeleton, clip, time);
        if self.entries.get(entry).is_some_and(|entry| entry.mirrored) {
            let sampled = local.clone();
            PoseMirror::new(skeleton, self.mirror_axis.normal())
                .and_then(|mirror| mirror.mirror_into(&sampled, &mut local))
                .map_err(BakeError::Mirror)?;
        }
        let model = model_pose(skeleton, &local);
        reexpress_roots(skeleton, &mut local, &model, simulation);
        Ok(local)
    }

    /// The model-space simulation bone of `frame`.
    pub fn simulation_bone(&self, frame: usize) -> Option<Transform> {
        self.baked.as_ref()?.simulation(frame)
    }

    /// The feature vector of `frame`.
    pub fn features(&self, frame: usize) -> Option<&[f32]> {
        let baked = self.baked.as_ref()?;
        let dims = baked.feature_dims;
        baked.features.get(frame * dims..(frame + 1) * dims)
    }

    /// The mean of every feature dimension over all frames.
    pub fn offset(&self) -> Option<&[f32]> {
        Some(&self.baked.as_ref()?.offset)
    }

    /// The mean standard deviation of each channel's group, one per channel in order.
    pub fn deviations(&self) -> Option<&[f32]> {
        Some(&self.baked.as_ref()?.deviations)
    }

    /// The bytes held by the feature, simulation and range arrays.
    pub fn size_in_bytes(&self) -> Option<usize> {
        let baked = self.baked.as_ref()?;
        Some(
            size_of_val(baked.features.as_slice())
                + size_of_val(baked.simulation.as_slice())
                + size_of_val(baked.range_starts.as_slice())
                + size_of_val(baked.range_stops.as_slice())
                + size_of_val(baked.clip_starts.as_slice()),
        )
    }

    fn weighted_spans(&self) -> Option<Vec<(Range<usize>, f32, f32)>> {
        let baked = self.baked.as_ref()?;
        if baked.spans.len() != self.channels.len() {
            return None;
        }
        baked
            .spans
            .iter()
            .zip(&baked.deviations)
            .zip(&self.channels)
            .map(|((span, deviation), channel)| {
                let weight = channel.channel().ok()?.weight();
                (weight.is_finite() && weight > 0.0).then(|| (span.range(), *deviation, weight))
            })
            .collect()
    }

    /// The divisor of every feature dimension: its group's floored deviation over the weight its live channel has now.
    pub fn scales(&self) -> Option<Vec<f32>> {
        let mut scales = vec![1.0; self.feature_dims()?];
        for (range, deviation, weight) in self.weighted_spans()? {
            scales[range].fill(deviation.max(DEVIATION_FLOOR) / weight);
        }
        Some(scales)
    }

    pub(crate) fn divisors(&self) -> Option<Vec<f32>> {
        let mut divisors = self.scales()?;
        for (range, deviation, _) in self.weighted_spans()? {
            if deviation < DEVIATION_FLOOR {
                divisors[range].fill(f32::INFINITY);
            }
        }
        Some(divisors)
    }

    /// Subtracts the offset and divides by the scale in place; a group with no deviation becomes 0.
    pub fn normalise(&self, features: &mut [f32]) -> Option<()> {
        let scales = self.scales()?;
        let offset = self.offset()?;
        for (range, deviation, _) in self.weighted_spans()? {
            if deviation < DEVIATION_FLOOR {
                features.get_mut(range)?.fill(0.0);
                continue;
            }
            for dim in range {
                let value = *features.get(dim)?;
                features[dim] = (value - offset[dim]) / scales[dim];
            }
        }
        Some(())
    }

    /// The feature vector of `frame` after `normalise`.
    pub fn normalised_features(&self, frame: usize) -> Option<Vec<f32>> {
        let mut features = self.features(frame)?.to_vec();
        self.normalise(&mut features)?;
        Some(features)
    }

    fn input_bytes<'a>(&self, inputs: &'a DerivedInputs, id: AssetId) -> anyhow::Result<&'a [u8]> {
        inputs
            .bytes(id)
            .with_context(|| format!("motion database input {id:?} was not read"))
    }
}

impl Asset for MotionDatabase {
    const FORMAT_VERSION: u32 = 3;

    fn name() -> &'static str {
        "MotionDatabase"
    }

    fn migrate(version: u32, _payload: &[u8]) -> anyhow::Result<Self> {
        anyhow::bail!(
            "MotionDatabase version {version} is a bake; a baked motion database is now derived from its definition; delete this file"
        )
    }
}

impl CreatableAsset for MotionDatabase {
    fn new_asset() -> Self {
        Self::default()
    }
}

impl LoadableAsset for MotionDatabase {
    fn on_load(&mut self, context: &AssetLoadContext) -> anyhow::Result<()> {
        let server = context.asset_server();
        self.skeleton = server.load(self.skeleton.id());
        for entry in &mut self.entries {
            entry.clip = server.load(entry.clip.id());
        }
        let registry = context.type_registry().read();
        for channel in &mut self.channels {
            if let Err(error) = channel.load(&registry) {
                log::warn!("motion database {:?}: {error}", context.asset_id());
            }
        }
        if let Err(error) = self.check_weights() {
            log::warn!(
                "motion database {:?} searches nothing: {error}",
                context.asset_id()
            );
        }
        Ok(())
    }
}

impl DerivedData for MotionDatabase {
    const DERIVED_VERSION: u32 = 1;

    fn derived_inputs(&self) -> Vec<AssetId> {
        std::iter::once(self.skeleton.id())
            .chain(self.clips().map(AssetHandle::id))
            .collect()
    }

    fn derived_key(&self, inputs: &DerivedInputs) -> anyhow::Result<u64> {
        let skeleton = self.input_bytes(inputs, self.skeleton.id())?;
        let clips = self
            .clips()
            .map(|clip| self.input_bytes(inputs, clip.id()))
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(weightless_hash(self, skeleton, &clips))
    }

    fn derive(
        &self,
        inputs: &DerivedInputs,
        progress: &mut dyn FnMut(usize, usize),
    ) -> anyhow::Result<Vec<u8>> {
        let skeleton: Skeleton = inputs.decode(self.skeleton.id())?;
        let clips = self
            .clips()
            .map(|clip| inputs.decode::<AnimationClip>(clip.id()))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let clips: Vec<&AnimationClip> = clips.iter().collect();
        let baked = bake_with_progress(self, &skeleton, &clips, progress)?;
        bincode::serialize(&baked).context("failed to encode a baked motion database")
    }

    fn apply_derived(&mut self, blob: &[u8]) -> anyhow::Result<()> {
        let baked: BakedDatabase =
            bincode::deserialize(blob).context("failed to decode a baked motion database")?;
        baked.fits(self)?;
        self.baked = Some(baked);
        Ok(())
    }
}

/// Why a motion database could not be baked, or a baked frame's pose could not be sampled.
#[derive(Clone, Debug, PartialEq)]
pub enum BakeError {
    MissingJoint(String),
    ClipHasNoBones { entry: usize },
    ClipCountMismatch { expected: usize, actual: usize },
    BoneCountMismatch { expected: usize, actual: usize },
    FrameOutOfRange { frame: usize, frame_count: usize },
    UnregisteredChannel(String),
    UnloadedChannel(String),
    ChannelValue { type_name: String, reason: String },
    RangeTooShort { entry: usize, frames: usize },
    InvalidRange { entry: usize },
    InvalidWindow { name: &'static str, window: usize },
    InvalidRate(f32),
    InvalidSpeed(f32),
    InvalidSampleDt(f32),
    InvalidWeight(f32),
    ChannelCountMismatch { expected: usize, actual: usize },
    TrajectoryTooShort { needed: usize, actual: usize },
    Mirror(MirrorError),
    NotBaked,
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
            BakeError::UnloadedChannel(name) => {
                write!(
                    f,
                    "feature channel `{name}` has not been loaded against a type registry"
                )
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
            BakeError::ChannelCountMismatch { expected, actual } => {
                write!(f, "the database has {actual} channels, {expected} expected")
            }
            BakeError::TrajectoryTooShort { needed, actual } => {
                write!(
                    f,
                    "the trajectory has {actual} samples, a channel reads {needed}"
                )
            }
            BakeError::Mirror(error) => write!(f, "the pose cannot be mirrored: {error}"),
            BakeError::NotBaked => write!(f, "the motion database has not been derived yet"),
        }
    }
}

impl std::error::Error for BakeError {}
