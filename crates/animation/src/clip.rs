use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, hash_map::Keys};
use std::ops::Range;

use anyhow::Context;
use concerto_foundation::assets::asset_server::{AssetLoadContext, AssetServer};
use concerto_foundation::assets::content::ContentAssetHeader;
use concerto_foundation::assets::derived::{DerivedData, DerivedInputs};
use concerto_foundation::assets::handle::{AssetHandle, LiveLeaves};
use concerto_foundation::assets::{Asset, AssetId, LoadableAsset};
use concerto_foundation::hash::Fnv1a;
use concerto_foundation::math::spring_damper::angular_velocity;
use concerto_name::Name;
use concerto_reflect::{
    EncodeError, FromType, Reflect, ReflectedValue, TypeRegistry, ValueWarning, to_value,
};
use glam::{Quat, Vec3};
use log::warn;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::event::{AnimationEvent, ClipEvent, ReflectClipEvent};
use crate::graph::{Stored, StoredRef};
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChannelComponent {
    Translation,
    Rotation,
    Scale,
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

    pub(crate) fn writes(&self) -> ChannelComponent {
        match self.outputs {
            AnimationChanelOutput::Translation(_) => ChannelComponent::Translation,
            AnimationChanelOutput::Rotation(_) => ChannelComponent::Rotation,
            AnimationChanelOutput::Scale(_) => ChannelComponent::Scale,
        }
    }

    pub(crate) fn constant(&self, transform: &mut JointPose) -> bool {
        if self.time_samples.len() != 1 {
            return false;
        }
        self.set_transform_at(transform, 0);
        true
    }

    fn hash_into(&self, hash: &mut Fnv1a) {
        let tag: u8 = match self.writes() {
            ChannelComponent::Translation => 0,
            ChannelComponent::Rotation => 1,
            ChannelComponent::Scale => 2,
        };
        hash.raw(&[tag]);
        hash.count(self.time_samples.len());
        for time in &self.time_samples {
            hash_float(hash, *time);
        }
        match &self.outputs {
            AnimationChanelOutput::Translation(values) | AnimationChanelOutput::Scale(values) => {
                for value in values.iter().flat_map(|value| value.to_array()) {
                    hash_float(hash, value);
                }
            }
            AnimationChanelOutput::Rotation(values) => {
                for value in values.iter().flat_map(|value| value.to_array()) {
                    hash_float(hash, value);
                }
            }
        }
    }

    fn median_key_spacing(&self) -> Option<f32> {
        let mut spacings: Vec<f32> = self
            .time_samples
            .windows(2)
            .map(|keys| keys[1] - keys[0])
            .collect();
        spacings.sort_by(f32::total_cmp);
        let upper = spacings.len() / 2;
        match spacings.len() {
            0 => None,
            count if count % 2 == 0 => Some((spacings[upper - 1] + spacings[upper]) / 2.0),
            _ => Some(spacings[upper]),
        }
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

/// One event on a clip: the value to trigger and the clip time it fires at.
#[derive(Serialize, Deserialize)]
pub struct ClipEventKey {
    pub time: f32,
    pub event: ClipEventData,
}

/// A clip event as a clip holds it: `Stored` until the clip loads, `Live` once built.
pub enum ClipEventData {
    Stored {
        type_name: String,
        value: ReflectedValue,
    },
    Live {
        type_name: String,
        value: Box<dyn Reflect>,
        data: ReflectClipEvent,
    },
}

impl ClipEventData {
    /// The registry key of the event's type.
    pub fn type_name(&self) -> &str {
        match self {
            Self::Stored { type_name, .. } | Self::Live { type_name, .. } => type_name,
        }
    }

    /// The reflected value a save writes.
    pub fn value(&self) -> Result<Cow<'_, ReflectedValue>, EncodeError> {
        match self {
            Self::Stored { value, .. } => Ok(Cow::Borrowed(value)),
            Self::Live { value, .. } => to_value(value.as_ref()).map(Cow::Owned),
        }
    }
}

impl Serialize for ClipEventData {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        StoredRef {
            type_name: self.type_name(),
            value: self.value().map_err(serde::ser::Error::custom)?,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ClipEventData {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let Stored { type_name, value } = Stored::deserialize(deserializer)?;
        Ok(Self::Stored { type_name, value })
    }
}

/// Whether a clip is additive, and against what.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Additive {
    #[default]
    None,
    LocalSpace(AdditiveBase),
}

/// The clip a delta is taken against, the spans of both clips that are aligned, and the bones whose translation is not added.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AdditiveBase {
    pub clip: AssetHandle<AnimationClip>,
    pub base_range: Option<ClipRange>,
    pub range: Option<ClipRange>,
    pub roots: Vec<Uuid>,
}

impl AdditiveBase {
    /// A base over the whole of `clip` and the whole of this clip, with no roots.
    pub fn new(clip: AssetHandle<AnimationClip>) -> Self {
        Self {
            clip,
            base_range: None,
            range: None,
            roots: Vec::new(),
        }
    }

    /// Aligns only this span of the base clip.
    pub fn with_base_range(mut self, base_range: ClipRange) -> Self {
        self.base_range = Some(base_range);
        self
    }

    /// Aligns only this span of the additive clip.
    pub fn with_range(mut self, range: ClipRange) -> Self {
        self.range = Some(range);
        self
    }

    /// Adds a bone whose translation is not added.
    pub fn with_root(mut self, root: Uuid) -> Self {
        self.roots.push(root);
        self
    }
}

/// A span of a clip in seconds, from `start` to `end`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClipRange {
    pub start: f32,
    pub end: f32,
}

impl ClipRange {
    /// The span from `start` to `end` seconds.
    pub fn new(start: f32, end: f32) -> Self {
        Self { start, end }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AdditiveDelta {
    duration: f32,
    frames: usize,
    bones: Vec<BoneDelta>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct BoneDelta {
    bone: Uuid,
    translation: Option<Vec<Vec3>>,
    rotation: Option<Vec<Quat>>,
}

impl AdditiveDelta {
    fn check(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.frames >= 2,
            "an additive delta has {} frames, at least 2 are needed",
            self.frames
        );
        anyhow::ensure!(
            self.duration.is_finite() && self.duration > 0.0,
            "an additive delta lasts {} seconds; it must be finite and above 0",
            self.duration
        );
        for bone in &self.bones {
            let lengths = bone
                .translation
                .as_ref()
                .map(Vec::len)
                .into_iter()
                .chain(bone.rotation.as_ref().map(Vec::len));
            for length in lengths {
                anyhow::ensure!(
                    length == self.frames,
                    "bone {} of an additive delta has {length} keys, not {}",
                    bone.bone,
                    self.frames
                );
            }
        }
        Ok(())
    }

    fn sample(&self, time: f32, bone: &Uuid) -> JointPose {
        let Ok(index) = self.bones.binary_search_by_key(bone, |delta| delta.bone) else {
            return JointPose::IDENTITY;
        };
        let delta = &self.bones[index];
        let position = time.clamp(0.0, self.duration) / self.duration * (self.frames - 1) as f32;
        let from = (position.floor() as usize).min(self.frames - 2);
        let blend = position - from as f32;
        let mut pose = JointPose::IDENTITY;
        if let Some(keys) = &delta.translation {
            pose.translation = keys[from].lerp(keys[from + 1], blend);
        }
        if let Some(keys) = &delta.rotation {
            pose.rotation = keys[from].slerp(keys[from + 1], blend);
        }
        pose
    }
}

struct FrameTime {
    this: f32,
    base: f32,
}

struct ChannelPair<'a> {
    this: &'a AnimationChannel,
    base: &'a AnimationChannel,
}

impl<'a> ChannelPair<'a> {
    fn of(
        this: &'a [AnimationChannel],
        base: &'a [AnimationChannel],
        component: ChannelComponent,
    ) -> Option<Self> {
        Some(Self {
            this: last_channel(this, component)?,
            base: last_channel(base, component)?,
        })
    }

    fn rotations(&self, times: &[FrameTime]) -> Vec<Quat> {
        let mut keys: Vec<Quat> = Vec::with_capacity(times.len());
        for time in times {
            let this = pose_at(self.this, time.this).rotation;
            let base = pose_at(self.base, time.base).rotation;
            let mut key = (this * base.inverse()).normalize();
            if keys.last().is_some_and(|previous| previous.dot(key) < 0.0) {
                key = -key;
            }
            keys.push(key);
        }
        keys
    }

    fn translations(&self, times: &[FrameTime]) -> Vec<Vec3> {
        times
            .iter()
            .map(|time| {
                pose_at(self.this, time.this).translation
                    - pose_at(self.base, time.base).translation
            })
            .collect()
    }
}

fn last_channel(
    channels: &[AnimationChannel],
    component: ChannelComponent,
) -> Option<&AnimationChannel> {
    channels
        .iter()
        .rev()
        .find(|channel| channel.writes() == component && !channel.time_samples.is_empty())
}

fn pose_at(channel: &AnimationChannel, time: f32) -> JointPose {
    let mut pose = JointPose::IDENTITY;
    if !channel.constant(&mut pose) {
        channel.sample_transform(time, &mut pose);
    }
    pose
}

fn checked_range(range: Option<ClipRange>, duration: f32, name: &str) -> anyhow::Result<ClipRange> {
    let range = range.unwrap_or(ClipRange::new(0.0, duration));
    let fits = range.start.is_finite()
        && range.end.is_finite()
        && (0.0..range.end).contains(&range.start)
        && range.end <= duration;
    anyhow::ensure!(
        fits,
        "{name} {} to {} must be finite with 0 <= start < end <= {duration}, the clip's length",
        range.start,
        range.end
    );
    Ok(range)
}

fn hash_float(hash: &mut Fnv1a, value: f32) {
    hash.raw(&value.to_bits().to_le_bytes());
}

fn hash_range(hash: &mut Fnv1a, range: Option<ClipRange>) {
    match range {
        None => hash.raw(&[0]),
        Some(range) => {
            hash.raw(&[1]);
            hash_float(hash, range.start);
            hash_float(hash, range.end);
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct AnimationClip {
    additive: Additive,
    channels: HashMap<Uuid, Vec<AnimationChannel>>,
    duration: f32,
    frame_rate: Option<f32>,
    events: Vec<ClipEventKey>,
    #[serde(skip)]
    delta: Option<AdditiveDelta>,
}

impl Default for AnimationClip {
    fn default() -> Self {
        Self {
            additive: Additive::None,
            channels: HashMap::new(),
            duration: 0.0,
            frame_rate: None,
            events: Vec::new(),
            delta: None,
        }
    }
}

impl Asset for AnimationClip {
    const FORMAT_VERSION: u32 = 3;

    fn name() -> &'static str {
        "AnimationClip"
    }

    fn migrate(version: u32, payload: &[u8]) -> anyhow::Result<Self> {
        match version {
            1 => {
                let old: legacy::AnimationClipV1 = bincode::deserialize(payload)?;
                Ok(legacy::AnimationClipV2::from(old).into())
            }
            2 => {
                let old: legacy::AnimationClipV2 = bincode::deserialize(payload)?;
                Ok(old.into())
            }
            _ => anyhow::bail!(
                "AnimationClip version {version} has no migration to version {}; re-import its source",
                Self::FORMAT_VERSION
            ),
        }
    }
}

mod legacy {
    use std::collections::HashMap;

    use serde::Deserialize;
    use uuid::Uuid;

    use super::{Additive, AnimationChannel, AnimationClip, ClipEventKey};

    #[derive(Deserialize)]
    pub(super) struct AnimationClipV1 {
        channels: HashMap<Uuid, Vec<AnimationChannel>>,
        duration: f32,
        frame_rate: Option<f32>,
    }

    #[derive(Deserialize)]
    pub(super) struct AnimationClipV2 {
        channels: HashMap<Uuid, Vec<AnimationChannel>>,
        duration: f32,
        frame_rate: Option<f32>,
        events: Vec<ClipEventKey>,
    }

    impl From<AnimationClipV1> for AnimationClipV2 {
        fn from(old: AnimationClipV1) -> Self {
            Self {
                channels: old.channels,
                duration: old.duration,
                frame_rate: old.frame_rate,
                events: Vec::new(),
            }
        }
    }

    impl From<AnimationClipV2> for AnimationClip {
        fn from(old: AnimationClipV2) -> Self {
            Self {
                additive: Additive::None,
                channels: old.channels,
                duration: old.duration,
                frame_rate: old.frame_rate,
                events: old.events,
                delta: None,
            }
        }
    }
}

fn snap_rate(rate: f32) -> f32 {
    let whole = rate.round();
    if (rate - whole).abs() <= 0.01 {
        whole
    } else {
        rate
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

    /// Whether the clip is additive, and against what.
    pub fn additive(&self) -> &Additive {
        &self.additive
    }

    /// Makes the clip additive against a base, or plain with [`Additive::None`].
    pub fn set_additive(&mut self, additive: Additive) {
        self.additive = additive;
    }

    /// The additive delta of `bone` at `time`, clamped to the delta's span; identity for a bone with no delta, a plain clip, or a delta not yet built.
    pub fn sample_additive(&self, time: f32, bone: &Uuid) -> JointPose {
        self.delta
            .as_ref()
            .map_or(JointPose::IDENTITY, |delta| delta.sample(time, bone))
    }

    /// The length in seconds of the built additive delta, or 0 without one.
    pub fn additive_duration(&self) -> f32 {
        self.delta.as_ref().map_or(0.0, |delta| delta.duration)
    }

    fn additive_base(&self) -> anyhow::Result<&AdditiveBase> {
        match &self.additive {
            Additive::LocalSpace(base) if !base.clip.id().is_nil() => Ok(base),
            Additive::LocalSpace(_) => anyhow::bail!("No base clip set"),
            Additive::None => anyhow::bail!("the clip is not additive"),
        }
    }

    fn key_with_base(&self, base_file: &[u8]) -> anyhow::Result<u64> {
        let additive = self.additive_base()?;
        let mut hash = Fnv1a::new();
        let mut bones: Vec<&Uuid> = self.channels.keys().collect();
        bones.sort();
        hash.count(bones.len());
        for bone in bones {
            let channels = &self.channels[bone];
            hash.raw(bone.as_bytes());
            hash.count(channels.len());
            for channel in channels {
                channel.hash_into(&mut hash);
            }
        }
        hash_float(&mut hash, self.duration);
        match self.frame_rate {
            None => hash.raw(&[0]),
            Some(rate) => {
                hash.raw(&[1]);
                hash_float(&mut hash, rate);
            }
        }
        hash.raw(&[1]);
        hash.raw(&additive.clip.id().to_bytes());
        hash_range(&mut hash, additive.base_range);
        hash_range(&mut hash, additive.range);
        let mut roots = additive.roots.clone();
        roots.sort();
        hash.count(roots.len());
        for root in &roots {
            hash.raw(root.as_bytes());
        }
        hash.bytes(base_file);
        Ok(hash.finish())
    }

    fn delta_against(&self, base: &AnimationClip) -> anyhow::Result<AdditiveDelta> {
        let additive = self.additive_base()?;
        let range = checked_range(additive.range, self.duration, "range")?;
        let base_range = checked_range(additive.base_range, base.duration, "base_range")?;
        let rate = self
            .frame_rate
            .or_else(|| self.detected_frame_rate())
            .unwrap_or(30.0);
        let length = range.end - range.start;
        let frames = ((length * rate).round() as usize + 1).max(2);
        let times: Vec<FrameTime> = (0..frames)
            .map(|frame| {
                let fraction = frame as f32 / (frames - 1) as f32;
                FrameTime {
                    this: range.start + length * fraction,
                    base: base_range.start + fraction * (base_range.end - base_range.start),
                }
            })
            .collect();
        let mut ids: Vec<&Uuid> = self.channels.keys().collect();
        ids.sort();
        let mut bones = Vec::new();
        for id in ids {
            let Some(base_channels) = base.channels.get(id) else {
                continue;
            };
            let channels = &self.channels[id];
            let rotation = ChannelPair::of(channels, base_channels, ChannelComponent::Rotation)
                .map(|pair| pair.rotations(&times));
            let translation = if additive.roots.contains(id) {
                None
            } else {
                ChannelPair::of(channels, base_channels, ChannelComponent::Translation)
                    .map(|pair| pair.translations(&times))
            };
            if rotation.is_none() && translation.is_none() {
                continue;
            }
            bones.push(BoneDelta {
                bone: *id,
                translation,
                rotation,
            });
        }
        Ok(AdditiveDelta {
            duration: length,
            frames,
            bones,
        })
    }

    /// Estimates the sample rate from the median key spacing of the channel with the most keys.
    pub fn detected_frame_rate(&self) -> Option<f32> {
        self.channels
            .values()
            .flatten()
            .filter_map(|channel| Some((channel.time_samples.len(), channel.median_key_spacing()?)))
            .max_by(|(keys, spacing), (other_keys, other_spacing)| {
                keys.cmp(other_keys)
                    .then_with(|| other_spacing.total_cmp(spacing))
            })
            .map(|(_, spacing)| spacing)
            .filter(|spacing| *spacing > 0.0)
            .map(|spacing| snap_rate(spacing.recip()))
            .filter(|rate| *rate > 0.0)
    }

    /// The authored frame rate, when the source recorded one.
    pub fn frame_rate(&self) -> Option<f32> {
        self.frame_rate
    }

    /// Stores the authored frame rate, snapping a rate within 0.01 of a whole number to it.
    pub fn set_frame_rate(&mut self, frame_rate: f32) {
        debug_assert!(frame_rate.is_finite() && frame_rate > 0.0);
        self.frame_rate = Some(snap_rate(frame_rate));
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

    /// Adds `event` at clip time `time`, after any event already at that time.
    pub fn add_event<S: ClipEvent>(&mut self, time: f32, event: S) {
        let index = self.events.partition_point(|key| key.time <= time);
        self.events.insert(
            index,
            ClipEventKey {
                time,
                event: ClipEventData::Live {
                    type_name: std::any::type_name::<S>().to_owned(),
                    value: Box::new(event),
                    data: <ReflectClipEvent as FromType<S>>::from_type(),
                },
            },
        );
    }

    /// The clip's events, sorted by time.
    pub fn events(&self) -> &[ClipEventKey] {
        &self.events
    }

    /// The indices of the events after `from`, or at it when `include_from`, up to and including `to`.
    pub fn events_in(&self, from: f32, to: f32, include_from: bool) -> Range<usize> {
        let start = if include_from {
            self.events.partition_point(|key| key.time < from)
        } else {
            self.events.partition_point(|key| key.time <= from)
        };
        let end = self.events.partition_point(|key| key.time <= to);
        start..end.max(start)
    }

    /// Adds an [`AnimationEvent`] per name and distinct time, failing on a time that is negative, non-finite or past the clip's end.
    pub fn add_animation_events(
        &mut self,
        events: &BTreeMap<String, Vec<f32>>,
    ) -> Result<(), String> {
        for (name, times) in events {
            let mut times = times.clone();
            for &time in &times {
                if !time.is_finite() || time < 0.0 || time > self.duration {
                    return Err(format!(
                        "event `{name}` at {time} must be finite and within the clip's 0 to {} seconds",
                        self.duration
                    ));
                }
            }
            times.sort_by(f32::total_cmp);
            times.dedup();
            for time in times {
                self.add_event(
                    time,
                    AnimationEvent {
                        name: Name::new(name),
                    },
                );
            }
        }
        Ok(())
    }

    /// Builds every stored event from the registry, loading handles through `server`; an event that cannot be built is dropped and leaves a warning naming its type.
    pub fn make_live(
        &mut self,
        registry: &TypeRegistry,
        server: Option<&AssetServer>,
    ) -> Vec<ValueWarning> {
        let mut warnings = Vec::new();
        if !self
            .events
            .iter()
            .any(|key| matches!(key.event, ClipEventData::Stored { .. }))
        {
            return warnings;
        }
        let mut leaves = LiveLeaves::from_parts(registry, server);
        self.events.retain_mut(|key| {
            let ClipEventData::Stored { type_name, value } = &mut key.event else {
                return true;
            };
            let reason = match registry
                .get_by_name(type_name)
                .and_then(|registration| registration.data::<ReflectClipEvent>().copied())
            {
                Some(data) => {
                    let built = (data.from_value)(value, &mut leaves);
                    if built.warnings.is_empty() {
                        key.event = ClipEventData::Live {
                            type_name: std::mem::take(type_name),
                            value: built.value,
                            data,
                        };
                        return true;
                    }
                    built
                        .warnings
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("; ")
                }
                None => "type is not registered as a clip event".to_owned(),
            };
            warnings.push(ValueWarning {
                path: std::mem::take(type_name),
                reason: format!("event at {} dropped: {reason}", key.time),
            });
            false
        });
        warnings
    }
}

impl LoadableAsset for AnimationClip {
    fn on_load(&mut self, context: &AssetLoadContext) -> anyhow::Result<()> {
        if self.events.is_empty() {
            return Ok(());
        }
        let registry = context.type_registry().read();
        for warning in self.make_live(&registry, Some(context.asset_server())) {
            warn!("animation clip {:?}: {warning}", context.asset_id());
        }
        Ok(())
    }
}

impl DerivedData for AnimationClip {
    const DERIVED_VERSION: u32 = 1;

    fn is_derived(&self) -> bool {
        !matches!(self.additive, Additive::None)
    }

    fn file_is_derived(header: &ContentAssetHeader, payload: &[u8]) -> anyhow::Result<bool> {
        if header.kind_version < 3 {
            return Ok(false);
        }
        anyhow::ensure!(
            header.kind_version <= Self::FORMAT_VERSION,
            "AnimationClip payload is version {}, newer than this build's version {}",
            header.kind_version,
            Self::FORMAT_VERSION
        );
        let additive: Additive = bincode::deserialize(payload)
            .context("failed to read whether an AnimationClip is additive")?;
        Ok(!matches!(additive, Additive::None))
    }

    fn derived_inputs(&self) -> Vec<AssetId> {
        self.additive_base()
            .map(|base| vec![base.clip.id()])
            .unwrap_or_default()
    }

    fn derived_key(&self, inputs: &DerivedInputs) -> anyhow::Result<u64> {
        let base = self.additive_base()?.clip.id();
        let file = inputs
            .bytes(base)
            .with_context(|| format!("the base clip {base:?} was not read"))?;
        self.key_with_base(file)
    }

    fn derive(
        &self,
        inputs: &DerivedInputs,
        progress: &mut dyn FnMut(usize, usize),
    ) -> anyhow::Result<Vec<u8>> {
        let base_id = self.additive_base()?.clip.id();
        let base: AnimationClip = inputs
            .decode(base_id)
            .with_context(|| format!("failed to read the base clip {base_id:?}"))?;
        let delta = self.delta_against(&base)?;
        progress(1, 1);
        bincode::serialize(&delta).context("failed to encode an additive delta")
    }

    fn apply_derived(&mut self, blob: &[u8]) -> anyhow::Result<()> {
        let mut delta: AdditiveDelta =
            bincode::deserialize(blob).context("failed to decode an additive delta")?;
        delta.check()?;
        delta.bones.sort_by_key(|bone| bone.bone);
        self.delta = Some(delta);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use concerto_name::Name;

    use super::*;
    use crate::event::AnimationEvent;
    use crate::pose::apply_additive;

    fn named(text: &str) -> AnimationEvent {
        AnimationEvent {
            name: Name::new(text),
        }
    }

    #[derive(Debug, PartialEq)]
    struct NamedKey<'a> {
        time: f32,
        name: &'a str,
    }

    fn key(time: f32, name: &str) -> NamedKey<'_> {
        NamedKey { time, name }
    }

    fn event_names(clip: &AnimationClip) -> Vec<NamedKey<'_>> {
        clip.events()
            .iter()
            .map(|key| {
                let ClipEventData::Live { value, .. } = &key.event else {
                    panic!("an added event is live");
                };
                let event = value
                    .downcast_ref::<AnimationEvent>()
                    .expect("the event is an AnimationEvent");
                NamedKey {
                    time: key.time,
                    name: event.name.as_str(),
                }
            })
            .collect()
    }

    fn quarters() -> AnimationClip {
        let mut clip = AnimationClip::default();
        clip.add_event(0.0, named("a"));
        clip.add_event(0.25, named("b"));
        clip.add_event(0.5, named("c"));
        clip.add_event(0.75, named("d"));
        clip
    }

    #[test]
    fn add_event_keeps_events_sorted_by_time_and_equal_times_in_insertion_order() {
        let mut clip = AnimationClip::default();
        clip.add_event(0.5, named("second"));
        clip.add_event(0.1, named("first"));
        clip.add_event(0.9, named("fourth"));
        clip.add_event(0.5, named("third"));

        assert_eq!(
            event_names(&clip),
            [
                key(0.1, "first"),
                key(0.5, "second"),
                key(0.5, "third"),
                key(0.9, "fourth")
            ]
        );
    }

    #[test]
    fn events_in_excludes_from_unless_asked_and_includes_to() {
        let clip = quarters();

        assert_eq!(clip.events_in(0.25, 0.5, false), 2..3);
        assert_eq!(clip.events_in(0.25, 0.5, true), 1..3);
        assert_eq!(clip.events_in(0.1, 0.6, false), 1..3);
    }

    #[test]
    fn events_in_a_single_instant_holds_its_event_only_when_from_is_included() {
        let clip = quarters();

        assert_eq!(clip.events_in(0.0, 0.0, true), 0..1);
        assert!(clip.events_in(0.0, 0.0, false).is_empty());
    }

    #[test]
    fn events_in_is_empty_past_the_last_event_and_for_a_reversed_window() {
        let clip = quarters();

        assert!(clip.events_in(0.8, 1.0, true).is_empty());
        assert!(clip.events_in(0.6, 0.3, true).is_empty());
        assert!(
            AnimationClip::default()
                .events_in(0.0, 1.0, true)
                .is_empty()
        );
    }

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

    fn channel_with_keys(time_samples: Vec<f32>) -> AnimationChannel {
        let outputs = vec![Vec3::ZERO; time_samples.len()];
        AnimationChannel::new(time_samples, AnimationChanelOutput::Translation(outputs))
    }

    fn channel_at(rate: f32, keys: u32) -> AnimationChannel {
        channel_with_keys((0..keys).map(|key| key as f32 / rate).collect())
    }

    #[test]
    fn frame_rate_is_none_by_default() {
        assert_eq!(AnimationClip::default().frame_rate(), None);
    }

    #[test]
    fn set_frame_rate_stores_the_rate() {
        let mut clip = AnimationClip::default();
        clip.set_frame_rate(30.0);

        assert_eq!(clip.frame_rate(), Some(30.0));
    }

    #[test]
    fn set_frame_rate_snaps_float_noise_to_a_whole_rate() {
        let mut clip = AnimationClip::default();
        clip.set_frame_rate(30.0003);

        assert_eq!(clip.frame_rate(), Some(30.0));
    }

    #[test]
    fn set_frame_rate_keeps_a_rate_that_is_not_whole() {
        let mut clip = AnimationClip::default();
        clip.set_frame_rate(29.97);

        assert_eq!(clip.frame_rate(), Some(29.97));
    }

    #[test]
    fn detected_frame_rate_is_none_when_the_rate_rounds_to_nothing() {
        let mut clip = AnimationClip::default();
        clip.add_channel(Uuid::from_u128(1), channel_with_keys(vec![0.0, 200.0]));

        assert_eq!(clip.detected_frame_rate(), None);
    }

    #[test]
    fn detected_frame_rate_is_the_reciprocal_of_the_key_spacing() {
        let mut clip = AnimationClip::default();
        clip.add_channel(Uuid::from_u128(1), channel_at(30.0, 31));

        assert_eq!(clip.detected_frame_rate(), Some(30.0));
    }

    #[test]
    fn detected_frame_rate_uses_the_channel_with_the_most_keys() {
        let mut clip = AnimationClip::default();
        clip.add_channel(Uuid::from_u128(1), channel_at(10.0, 4));
        clip.add_channel(Uuid::from_u128(2), channel_at(60.0, 61));
        clip.add_channel(Uuid::from_u128(3), channel_at(24.0, 5));

        assert_eq!(clip.detected_frame_rate(), Some(60.0));
    }

    #[test]
    fn detected_frame_rate_takes_the_middle_of_the_spacings() {
        let mut clip = AnimationClip::default();
        clip.add_channel(
            Uuid::from_u128(1),
            channel_with_keys(vec![0.0, 0.5, 1.0, 5.0, 5.5]),
        );

        assert_eq!(clip.detected_frame_rate(), Some(2.0));
    }

    #[test]
    fn detected_frame_rate_breaks_a_tie_on_the_smallest_spacing_in_any_order() {
        let id = Uuid::from_u128(1);
        let mut coarse_first = AnimationClip::default();
        coarse_first.add_channel(id, channel_at(30.0, 10));
        coarse_first.add_channel(id, channel_at(60.0, 10));
        let mut fine_first = AnimationClip::default();
        fine_first.add_channel(id, channel_at(60.0, 10));
        fine_first.add_channel(id, channel_at(30.0, 10));

        assert_eq!(coarse_first.detected_frame_rate(), Some(60.0));
        assert_eq!(fine_first.detected_frame_rate(), Some(60.0));
    }

    #[test]
    fn detected_frame_rate_rounds_float_noise_to_a_whole_rate() {
        let mut clip = AnimationClip::default();
        clip.add_channel(
            Uuid::from_u128(1),
            channel_with_keys((0..100).map(|key| key as f32 * 0.033_333).collect()),
        );

        assert_eq!(clip.detected_frame_rate(), Some(30.0));
    }

    #[test]
    fn detected_frame_rate_keeps_a_rate_that_is_not_whole() {
        let mut clip = AnimationClip::default();
        clip.add_channel(Uuid::from_u128(1), channel_at(29.97, 100));

        let rate = clip.detected_frame_rate().expect("the clip has keys");
        assert!((rate - 29.97).abs() < 1e-3, "{rate}");
    }

    #[test]
    fn detected_frame_rate_is_none_without_a_channel_of_two_keys() {
        let mut clip = AnimationClip::default();
        assert_eq!(clip.detected_frame_rate(), None);

        clip.add_channel(Uuid::from_u128(1), channel_with_keys(vec![0.5]));
        assert_eq!(clip.detected_frame_rate(), None);
    }

    #[test]
    fn detected_frame_rate_is_none_when_the_spacing_is_not_positive() {
        let mut clip = AnimationClip::default();
        clip.add_channel(
            Uuid::from_u128(1),
            channel_with_keys(vec![0.5, 0.5, 0.5, 0.5]),
        );

        assert_eq!(clip.detected_frame_rate(), None);
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

    fn rotations(times: Vec<f32>, keys: Vec<Quat>) -> AnimationChannel {
        AnimationChannel::new(times, AnimationChanelOutput::Rotation(keys))
    }

    fn translations(times: Vec<f32>, keys: Vec<Vec3>) -> AnimationChannel {
        AnimationChannel::new(times, AnimationChanelOutput::Translation(keys))
    }

    fn still_rotation(rotation: Quat) -> AnimationChannel {
        rotations(vec![0.0, 1.0], vec![rotation, rotation])
    }

    fn still_translation(translation: Vec3) -> AnimationChannel {
        translations(vec![0.0, 1.0], vec![translation, translation])
    }

    fn x_ramp(duration: f32, from: f32, to: f32) -> AnimationChannel {
        translations(
            vec![0.0, duration],
            vec![Vec3::new(from, 0.0, 0.0), Vec3::new(to, 0.0, 0.0)],
        )
    }

    fn some_base() -> AdditiveBase {
        AdditiveBase::new(AssetHandle::weak(AssetId::new()))
    }

    fn additive(mut clip: AnimationClip, base: AdditiveBase) -> AnimationClip {
        clip.set_additive(Additive::LocalSpace(base));
        clip
    }

    fn sampled(clip: &AnimationClip, bone: Uuid, time: f32) -> JointPose {
        let mut pose = JointPose::IDENTITY;
        for channel in clip.get_channels(&bone).unwrap() {
            channel.sample_transform(time, &mut pose);
        }
        pose
    }

    fn bone_delta(delta: &AdditiveDelta, bone: Uuid) -> &BoneDelta {
        delta
            .bones
            .iter()
            .find(|delta| delta.bone == bone)
            .expect("the bone has a delta")
    }

    fn apply(clip: &mut AnimationClip, delta: &AdditiveDelta) {
        clip.apply_derived(&bincode::serialize(delta).unwrap())
            .unwrap();
    }

    fn assert_same_rotation(actual: Quat, expected: Quat) {
        let actual = actual.normalize();
        let mut expected = expected.normalize();
        if actual.dot(expected) < 0.0 {
            expected = -expected;
        }
        assert!(
            (actual - expected).length() < 1e-4,
            "{actual:?} vs {expected:?}"
        );
    }

    fn assert_near(actual: Vec3, expected: Vec3) {
        assert!(
            actual.distance(expected) < 1e-4,
            "{actual:?} vs {expected:?}"
        );
    }

    #[test]
    fn the_delta_at_weight_one_turns_the_base_pose_into_the_clip_pose_at_and_between_keys() {
        let bone = Uuid::from_u128(1);
        let rest = Quat::from_rotation_y(0.4);
        let turn = Quat::from_rotation_x(30f32.to_radians());
        let mut base = AnimationClip::default();
        base.add_channel(bone, still_rotation(rest));
        let mut clip = AnimationClip::default();
        clip.add_channel(bone, rotations(vec![0.0, 1.0], vec![rest, turn * rest]));
        clip.set_frame_rate(10.0);
        let mut clip = additive(clip, some_base());

        let delta = clip.delta_against(&base).unwrap();
        assert_eq!(delta.frames, 11);
        apply(&mut clip, &delta);

        for step in 0..=20 {
            let time = step as f32 * 0.05;
            let mut pose = sampled(&base, bone, time);
            apply_additive(&mut pose, &clip.sample_additive(time, &bone), 1.0);
            assert_same_rotation(pose.rotation, sampled(&clip, bone, time).rotation);
        }
    }

    #[test]
    fn translation_deltas_are_this_minus_base_and_a_root_keeps_only_its_rotation() {
        let root = Uuid::from_u128(1);
        let child = Uuid::from_u128(2);
        let rest = Quat::from_rotation_y(0.2);
        let lean = Quat::from_rotation_x(0.3);
        let mut base = AnimationClip::default();
        base.add_channel(root, still_translation(Vec3::new(0.0, 1.0, 0.0)));
        base.add_channel(root, still_rotation(rest));
        base.add_channel(child, still_translation(Vec3::new(0.0, 0.5, 0.0)));
        let mut clip = AnimationClip::default();
        clip.add_channel(
            root,
            translations(
                vec![0.0, 1.0],
                vec![Vec3::new(1.0, 1.0, 0.0), Vec3::new(2.0, 1.0, 0.0)],
            ),
        );
        clip.add_channel(root, still_rotation(lean * rest));
        clip.add_channel(
            child,
            translations(
                vec![0.0, 1.0],
                vec![Vec3::new(0.1, 0.5, 0.0), Vec3::new(0.3, 0.5, 0.0)],
            ),
        );
        clip.set_frame_rate(4.0);
        let clip = additive(clip, some_base().with_root(root));

        let delta = clip.delta_against(&base).unwrap();

        let root_delta = bone_delta(&delta, root);
        assert!(root_delta.translation.is_none());
        let root_rotation = root_delta.rotation.as_ref().expect("the root turns");
        for key in root_rotation {
            assert_same_rotation(*key, lean);
        }
        let child_delta = bone_delta(&delta, child);
        assert!(child_delta.rotation.is_none());
        let child_translation = child_delta.translation.as_ref().unwrap();
        assert_near(child_translation[0], Vec3::new(0.1, 0.0, 0.0));
        assert_near(child_translation[2], Vec3::new(0.2, 0.0, 0.0));
        assert_near(child_translation[4], Vec3::new(0.3, 0.0, 0.0));
    }

    #[test]
    fn with_two_rotation_channels_on_a_bone_the_last_is_used() {
        let bone = Uuid::from_u128(1);
        let mut base = AnimationClip::default();
        base.add_channel(bone, still_rotation(Quat::from_rotation_y(0.9)));
        base.add_channel(bone, still_rotation(Quat::IDENTITY));
        let mut clip = AnimationClip::default();
        clip.add_channel(bone, still_rotation(Quat::from_rotation_z(1.0)));
        clip.add_channel(bone, still_rotation(Quat::from_rotation_x(0.5)));
        let clip = additive(clip, some_base());

        let delta = clip.delta_against(&base).unwrap();

        for key in bone_delta(&delta, bone).rotation.as_ref().unwrap() {
            assert_same_rotation(*key, Quat::from_rotation_x(0.5));
        }
    }

    #[test]
    fn a_base_twice_as_long_is_met_at_normalized_time() {
        let bone = Uuid::from_u128(1);
        let mut base = AnimationClip::default();
        base.add_channel(bone, x_ramp(2.0, 0.0, 2.0));
        let mut clip = AnimationClip::default();
        clip.add_channel(bone, still_translation(Vec3::ZERO));
        clip.set_frame_rate(4.0);
        let clip = additive(clip, some_base());

        let delta = clip.delta_against(&base).unwrap();

        assert_eq!(delta.frames, 5);
        let keys = bone_delta(&delta, bone).translation.as_ref().unwrap();
        for (frame, key) in keys.iter().enumerate() {
            let local = frame as f32 / 4.0;
            assert_near(*key, Vec3::new(-2.0 * local, 0.0, 0.0));
        }
    }

    #[test]
    fn range_and_base_range_trim_each_clip_before_alignment() {
        let bone = Uuid::from_u128(1);
        let mut base = AnimationClip::default();
        base.add_channel(bone, x_ramp(4.0, 0.0, 4.0));
        let mut clip = AnimationClip::default();
        clip.add_channel(bone, x_ramp(4.0, 0.0, 4.0));
        clip.set_frame_rate(2.0);
        let clip = additive(
            clip,
            some_base()
                .with_range(ClipRange::new(1.0, 2.0))
                .with_base_range(ClipRange::new(2.0, 4.0)),
        );

        let delta = clip.delta_against(&base).unwrap();

        assert_eq!(delta.duration, 1.0);
        assert_eq!(delta.frames, 3);
        let keys = bone_delta(&delta, bone).translation.as_ref().unwrap();
        for (frame, key) in keys.iter().enumerate() {
            let local = frame as f32 * 0.5;
            assert_near(*key, Vec3::new(-1.0 - local, 0.0, 0.0));
        }
    }

    #[test]
    fn a_range_outside_the_clip_reversed_or_not_finite_fails_naming_which_range() {
        let bone = Uuid::from_u128(1);
        let mut base = AnimationClip::default();
        base.add_channel(bone, x_ramp(1.0, 0.0, 1.0));
        let bad = [
            ClipRange::new(0.0, 1.5),
            ClipRange::new(-0.1, 0.5),
            ClipRange::new(0.8, 0.2),
            ClipRange::new(0.5, 0.5),
            ClipRange::new(0.0, f32::NAN),
            ClipRange::new(0.0, f32::INFINITY),
        ];
        for range in bad {
            let mut clip = AnimationClip::default();
            clip.add_channel(bone, x_ramp(1.0, 0.0, 1.0));
            let clip = additive(clip, some_base().with_range(range));
            let message = clip.delta_against(&base).unwrap_err().to_string();
            assert!(message.starts_with("range "), "{range:?}: {message}");

            let mut clip = AnimationClip::default();
            clip.add_channel(bone, x_ramp(1.0, 0.0, 1.0));
            let clip = additive(clip, some_base().with_base_range(range));
            let message = clip.delta_against(&base).unwrap_err().to_string();
            assert!(message.starts_with("base_range "), "{range:?}: {message}");
        }
    }

    #[test]
    fn the_rate_is_the_stored_one_then_the_detected_one_then_30_and_keys_land_on_both_range_ends() {
        let bone = Uuid::from_u128(1);
        let mut base = AnimationClip::default();
        base.add_channel(bone, still_translation(Vec3::ZERO));
        let dense = || {
            let times: Vec<f32> = (0..=20).map(|key| key as f32 * 0.05).collect();
            let keys = times
                .iter()
                .map(|time| Vec3::new(*time, 0.0, 0.0))
                .collect();
            translations(times, keys)
        };

        let mut stored = AnimationClip::default();
        stored.add_channel(bone, dense());
        stored.set_frame_rate(10.0);
        let stored = additive(stored, some_base());
        assert_eq!(stored.delta_against(&base).unwrap().frames, 11);

        let mut detected = AnimationClip::default();
        detected.add_channel(bone, dense());
        let detected = additive(detected, some_base());
        assert_eq!(detected.delta_against(&base).unwrap().frames, 21);

        let mut fallback = AnimationClip::default();
        fallback.add_channel(bone, translations(vec![1.0], vec![Vec3::ONE]));
        let fallback = additive(fallback, some_base());
        assert_eq!(fallback.delta_against(&base).unwrap().frames, 31);

        let mut trimmed = AnimationClip::default();
        trimmed.add_channel(bone, dense());
        trimmed.set_frame_rate(10.0);
        let trimmed = additive(trimmed, some_base().with_range(ClipRange::new(0.25, 0.75)));
        let delta = trimmed.delta_against(&base).unwrap();
        assert_eq!(delta.frames, 6);
        let keys = bone_delta(&delta, bone).translation.as_ref().unwrap();
        assert_near(keys[0], Vec3::new(0.25, 0.0, 0.0));
        assert_near(keys[5], Vec3::new(0.75, 0.0, 0.0));
    }

    #[test]
    fn a_bone_or_component_missing_from_either_clip_gives_identity() {
        let turned = Uuid::from_u128(1);
        let moved = Uuid::from_u128(2);
        let only_here = Uuid::from_u128(3);
        let only_there = Uuid::from_u128(4);
        let mut base = AnimationClip::default();
        base.add_channel(turned, still_translation(Vec3::X));
        base.add_channel(moved, still_translation(Vec3::ZERO));
        base.add_channel(only_there, still_rotation(Quat::from_rotation_y(1.0)));
        let mut clip = AnimationClip::default();
        clip.add_channel(turned, still_rotation(Quat::from_rotation_x(0.5)));
        clip.add_channel(moved, still_translation(Vec3::Y));
        clip.add_channel(only_here, still_rotation(Quat::from_rotation_z(0.5)));
        let mut clip = additive(clip, some_base());

        let delta = clip.delta_against(&base).unwrap();
        let stored: Vec<Uuid> = delta.bones.iter().map(|bone| bone.bone).collect();
        assert_eq!(stored, [moved]);
        assert!(bone_delta(&delta, moved).rotation.is_none());
        apply(&mut clip, &delta);

        assert_eq!(clip.sample_additive(0.5, &turned), JointPose::IDENTITY);
        assert_eq!(clip.sample_additive(0.5, &only_here), JointPose::IDENTITY);
        assert_eq!(clip.sample_additive(0.5, &only_there), JointPose::IDENTITY);
        let moved_delta = clip.sample_additive(0.5, &moved);
        assert_eq!(moved_delta.rotation, Quat::IDENTITY);
        assert_near(moved_delta.translation, Vec3::Y);
    }

    #[test]
    fn rotation_keys_never_flip_hemisphere_between_neighbours() {
        let bone = Uuid::from_u128(1);
        let mut base = AnimationClip::default();
        base.add_channel(bone, still_rotation(Quat::IDENTITY));
        let mut clip = AnimationClip::default();
        clip.add_channel(
            bone,
            rotations(
                vec![0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0],
                vec![
                    Quat::from_rotation_x(0.1),
                    -Quat::from_rotation_x(0.2),
                    Quat::from_rotation_x(0.3),
                    -Quat::from_rotation_x(0.4),
                ],
            ),
        );
        clip.set_frame_rate(3.0);
        let clip = additive(clip, some_base());

        let delta = clip.delta_against(&base).unwrap();

        let keys = bone_delta(&delta, bone).rotation.as_ref().unwrap();
        assert_eq!(keys.len(), 4);
        for pair in keys.windows(2) {
            assert!(pair[0].dot(pair[1]) > 0.0, "{pair:?}");
        }
    }

    #[test]
    fn a_one_key_channel_counts_as_a_constant() {
        let bone = Uuid::from_u128(1);
        let timing = Uuid::from_u128(2);
        let rest = Quat::from_rotation_y(0.6);
        let lean = Quat::from_rotation_x(0.4);
        let mut base = AnimationClip::default();
        base.add_channel(bone, rotations(vec![0.0], vec![rest]));
        base.add_channel(timing, x_ramp(1.0, 0.0, 1.0));
        let mut clip = AnimationClip::default();
        clip.add_channel(bone, rotations(vec![0.0], vec![lean * rest]));
        clip.add_channel(timing, x_ramp(1.0, 0.0, 1.0));
        let clip = additive(clip, some_base());

        let delta = clip.delta_against(&base).unwrap();

        let keys = bone_delta(&delta, bone).rotation.as_ref().unwrap();
        for key in keys {
            assert_same_rotation(*key, lean);
        }
    }

    #[test]
    fn a_nil_base_fails_with_no_base_clip_set_before_any_input_is_read() {
        let mut clip = AnimationClip::default();
        clip.add_channel(Uuid::from_u128(1), x_ramp(1.0, 0.0, 1.0));
        let clip = additive(clip, AdditiveBase::default());

        assert!(clip.is_derived());
        assert!(clip.derived_inputs().is_empty());
        let error = clip.derived_key(&DerivedInputs::default()).unwrap_err();
        assert_eq!(error.to_string(), "No base clip set");
        let error = clip
            .derive(&DerivedInputs::default(), &mut |_, _| {})
            .unwrap_err();
        assert_eq!(error.to_string(), "No base clip set");
    }

    #[test]
    fn sample_additive_clamps_to_the_delta_and_is_identity_without_one() {
        let bone = Uuid::from_u128(1);
        let mut base = AnimationClip::default();
        base.add_channel(bone, still_translation(Vec3::ZERO));
        let mut plain = AnimationClip::default();
        plain.add_channel(bone, x_ramp(1.0, 0.0, 1.0));
        assert_eq!(plain.sample_additive(0.5, &bone), JointPose::IDENTITY);
        assert_eq!(plain.additive_duration(), 0.0);

        let mut clip = additive(plain, some_base());
        assert_eq!(clip.sample_additive(0.5, &bone), JointPose::IDENTITY);
        assert_eq!(clip.additive_duration(), 0.0);

        let delta = clip.delta_against(&base).unwrap();
        apply(&mut clip, &delta);

        assert_eq!(clip.additive_duration(), 1.0);
        assert_near(
            clip.sample_additive(0.25, &bone).translation,
            Vec3::new(0.25, 0.0, 0.0),
        );
        assert_near(clip.sample_additive(-1.0, &bone).translation, Vec3::ZERO);
        assert_near(clip.sample_additive(5.0, &bone).translation, Vec3::X);
        assert_eq!(
            clip.sample_additive(0.5, &Uuid::from_u128(9)),
            JointPose::IDENTITY
        );
    }

    #[test]
    fn apply_derived_refuses_a_blob_that_does_not_agree_with_itself() {
        let bone = Uuid::from_u128(1);
        let delta = |duration: f32, frames: usize, keys: usize| AdditiveDelta {
            duration,
            frames,
            bones: vec![BoneDelta {
                bone,
                translation: Some(vec![Vec3::ZERO; keys]),
                rotation: Some(vec![Quat::IDENTITY; frames]),
            }],
        };
        let refused = [
            delta(1.0, 1, 1),
            delta(0.0, 3, 3),
            delta(-1.0, 3, 3),
            delta(f32::NAN, 3, 3),
            delta(f32::INFINITY, 3, 3),
            delta(1.0, 3, 2),
        ];
        for blob in refused {
            let mut clip = additive(AnimationClip::default(), some_base());
            let bytes = bincode::serialize(&blob).unwrap();
            assert!(clip.apply_derived(&bytes).is_err(), "{blob:?}");
            assert_eq!(clip.additive_duration(), 0.0);
        }

        let mut clip = additive(AnimationClip::default(), some_base());
        apply(&mut clip, &delta(1.0, 3, 3));
        assert_eq!(clip.additive_duration(), 1.0);
    }

    fn key_of(clip: &AnimationClip, base_file: &[u8]) -> u64 {
        clip.key_with_base(base_file).unwrap()
    }

    fn keyed(order: &[u128], base: AdditiveBase, last_key: f32) -> AnimationClip {
        let mut clip = AnimationClip::default();
        for bone in order {
            clip.add_channel(Uuid::from_u128(*bone), x_ramp(1.0, 0.0, last_key));
            clip.add_channel(Uuid::from_u128(*bone), still_rotation(Quat::IDENTITY));
        }
        clip.set_frame_rate(30.0);
        additive(clip, base)
    }

    #[test]
    fn the_key_ignores_bone_order_and_follows_ranges_roots_the_base_bytes_and_keys() {
        let base = AdditiveBase::new(AssetHandle::weak(AssetId::from_bytes([7; 16])));
        let base_file: &[u8] = &[1, 2, 3];
        let original = key_of(&keyed(&[1, 2, 3], base.clone(), 1.0), base_file);

        assert_eq!(
            key_of(&keyed(&[3, 1, 2], base.clone(), 1.0), base_file),
            original
        );
        let ranged = base.clone().with_range(ClipRange::new(0.0, 0.5));
        assert_ne!(key_of(&keyed(&[1, 2, 3], ranged, 1.0), base_file), original);
        let base_ranged = base.clone().with_base_range(ClipRange::new(0.0, 0.5));
        assert_ne!(
            key_of(&keyed(&[1, 2, 3], base_ranged, 1.0), base_file),
            original
        );
        let rooted = base.clone().with_root(Uuid::from_u128(1));
        assert_ne!(key_of(&keyed(&[1, 2, 3], rooted, 1.0), base_file), original);
        assert_ne!(
            key_of(&keyed(&[1, 2, 3], base.clone(), 1.0), &[1, 2, 4][..]),
            original
        );
        assert_ne!(key_of(&keyed(&[1, 2, 3], base, 1.5), base_file), original);
    }

    fn header(kind_version: u32) -> ContentAssetHeader {
        ContentAssetHeader {
            asset_id: AssetId::new(),
            references: Vec::new(),
            kind: AnimationClip::name().to_owned(),
            kind_version,
            provenance: None,
            derived: None,
        }
    }

    #[test]
    fn file_is_derived_skips_old_versions_and_reads_only_the_leading_additive() {
        let garbage = [0xff_u8; 6];
        assert!(!AnimationClip::file_is_derived(&header(1), &garbage).unwrap());
        assert!(!AnimationClip::file_is_derived(&header(2), &garbage).unwrap());

        let mut leading = bincode::serialize(&Additive::LocalSpace(some_base())).unwrap();
        leading.extend_from_slice(&garbage);
        assert!(AnimationClip::file_is_derived(&header(3), &leading).unwrap());

        let plain = bincode::serialize(&linear_clip(Uuid::from_u128(1))).unwrap();
        assert!(!AnimationClip::file_is_derived(&header(3), &plain).unwrap());
        let layered =
            bincode::serialize(&additive(linear_clip(Uuid::from_u128(1)), some_base())).unwrap();
        assert!(AnimationClip::file_is_derived(&header(3), &layered).unwrap());
    }

    #[test]
    fn apply_additive_at_weight_zero_leaves_the_pose_and_never_touches_scale() {
        let pose = JointPose {
            translation: Vec3::new(1.0, 2.0, 3.0),
            rotation: Quat::from_rotation_y(0.3),
            scale: Vec3::splat(2.0),
        };
        let delta = JointPose {
            translation: Vec3::new(5.0, 0.0, 0.0),
            rotation: Quat::from_rotation_x(1.0),
            scale: Vec3::splat(9.0),
        };

        let mut unweighted = pose.clone();
        apply_additive(&mut unweighted, &delta, 0.0);
        assert_near(unweighted.translation, pose.translation);
        assert_same_rotation(unweighted.rotation, pose.rotation);
        assert_eq!(unweighted.scale, pose.scale);

        let mut weighted = pose.clone();
        apply_additive(&mut weighted, &delta, 1.0);
        assert_near(weighted.translation, Vec3::new(6.0, 2.0, 3.0));
        assert_same_rotation(weighted.rotation, delta.rotation * pose.rotation);
        assert_eq!(weighted.scale, pose.scale);
    }
}
