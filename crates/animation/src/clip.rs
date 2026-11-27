use std::borrow::Cow;
use std::cmp::Ordering;
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
use concerto_mesh::Skeleton;
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

/// The key values of an [`AnimationChannel`], one per key time, for the joint part it animates.
#[derive(Serialize, Deserialize)]
pub enum AnimationChanelOutput {
    /// Translations in the parent bone's space.
    Translation(Vec<Vec3>),
    /// Rotations relative to the parent bone.
    Rotation(Vec<Quat>),
    /// Per-axis scales.
    Scale(Vec<Vec3>),
}

impl AnimationChanelOutput {
    /// Translation keys from `[x, y, z]` arrays.
    pub fn from_translation(translations: impl Iterator<Item = [f32; 3]>) -> Self {
        Self::Translation(translations.map(Vec3::from_array).collect::<Vec<_>>())
    }

    /// Rotation keys from `[x, y, z, w]` quaternion arrays.
    pub fn from_rotation(rotations: impl Iterator<Item = [f32; 4]>) -> Self {
        Self::Rotation(rotations.map(Quat::from_array).collect::<Vec<_>>())
    }

    /// Scale keys from `[x, y, z]` arrays.
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

/// Where a channel's last sample landed among its keys, kept by whoever plays it.
///
/// A cursor from another channel, or from a channel that has changed since, is safe to pass.
#[derive(Clone, Copy, Debug, Default)]
pub struct KeyCursor {
    key: usize,
}

#[derive(Clone, Copy)]
enum KeySpan {
    At(usize),
    Between(usize),
}

impl KeySpan {
    fn key(self) -> usize {
        match self {
            Self::At(key) | Self::Between(key) => key,
        }
    }
}

/// The keys that animate one part of one joint: translation, rotation or scale.
///
/// Between keys, translation and scale are lerped and rotation is slerped; before the first key
/// and after the last the nearest key holds.
#[derive(Serialize, Deserialize)]
pub struct AnimationChannel {
    time_samples: Vec<f32>,
    outputs: AnimationChanelOutput,
}

impl AnimationChannel {
    /// A channel of `outputs` at `time_samples`, in seconds, ascending, one per output.
    pub fn new(time_samples: Vec<f32>, outputs: AnimationChanelOutput) -> Self {
        Self {
            time_samples,
            outputs,
        }
    }

    /// Writes the channel's value at `current_time` seconds into its part of `transform`.
    ///
    /// A channel with fewer than two keys leaves `transform` unchanged.
    pub fn sample_transform(&self, current_time: f32, transform: &mut JointPose) {
        if self.time_samples.len() < 2 {
            return;
        }

        self.set_transform_in(transform, self.search(current_time), current_time);
    }

    /// Samples as [`Self::sample_transform`] does, trying the keys at `cursor` before searching, and leaves `cursor` at the keys found.
    pub fn sample_transform_from(
        &self,
        current_time: f32,
        cursor: &mut KeyCursor,
        transform: &mut JointPose,
    ) {
        if self.time_samples.len() < 2 {
            return;
        }

        let span = self
            .span_at(current_time, cursor.key)
            .or_else(|| self.span_at(current_time, cursor.key.saturating_add(1)))
            .unwrap_or_else(|| self.search(current_time));
        cursor.key = span.key();
        self.set_transform_in(transform, span, current_time);
    }

    fn search(&self, current_time: f32) -> KeySpan {
        match self
            .time_samples
            .binary_search_by(|val| val.total_cmp(&current_time))
        {
            Ok(index) => KeySpan::At(index),
            Err(0) => KeySpan::At(0),
            Err(index) if index >= self.time_samples.len() => {
                KeySpan::At(self.time_samples.len() - 1)
            }
            Err(index) => KeySpan::Between(index - 1),
        }
    }

    fn span_at(&self, current_time: f32, key: usize) -> Option<KeySpan> {
        let time = self.time_samples.get(key)?;
        let after =
            |other: Option<&f32>| other.is_none_or(|other| other.total_cmp(&current_time).is_gt());
        let next = self.time_samples.get(key + 1);
        match time.total_cmp(&current_time) {
            Ordering::Less if next.is_none() => Some(KeySpan::At(key)),
            Ordering::Less => after(next).then_some(KeySpan::Between(key)),
            Ordering::Greater => (key == 0).then_some(KeySpan::At(key)),
            Ordering::Equal => {
                let previous = key
                    .checked_sub(1)
                    .map(|previous| &self.time_samples[previous]);
                let alone = after(next)
                    && previous.is_none_or(|previous| previous.total_cmp(&current_time).is_lt());
                alone.then_some(KeySpan::At(key))
            }
        }
    }

    fn set_transform_in(&self, transform: &mut JointPose, span: KeySpan, current_time: f32) {
        match span {
            KeySpan::At(index) => self.set_transform_at(transform, index),
            KeySpan::Between(index) => {
                let before_time = self.time_samples[index];
                let after_time = self.time_samples[index + 1];

                let normalized_time = (current_time - before_time) / (after_time - before_time);

                self.interpolate_between(transform, index, normalized_time);
            }
        }
    }

    /// The time of the last key in seconds, or `None` for a channel with no keys.
    pub fn duration(&self) -> Option<f32> {
        self.time_samples.last().copied()
    }

    /// The channel's key times in seconds, in order.
    pub fn key_times(&self) -> &[f32] {
        &self.time_samples
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
    /// The clip time the event fires at, in seconds.
    pub time: f32,
    /// The event value.
    pub event: ClipEventData,
}

/// A clip event as a clip holds it: `Stored` until the clip loads, `Live` once built.
pub enum ClipEventData {
    /// As read from the file, before the event's type is looked up.
    Stored {
        /// The registry key of the event's type.
        type_name: String,
        /// The event's reflected fields.
        value: ReflectedValue,
    },
    /// Built into the event's type, ready to trigger.
    Live {
        /// The registry key of the event's type.
        type_name: String,
        /// The event value.
        value: Box<dyn Reflect>,
        /// How the event's type is built and triggered.
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
    ///
    /// # Errors
    ///
    /// Returns an [`EncodeError`] when a live event cannot be encoded through reflection.
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
    /// The clip is a pose, played as it is.
    #[default]
    None,
    /// The clip is a delta in each joint's local space, taken against a base clip.
    LocalSpace(AdditiveBase),
}

/// The clip a delta is taken against, the spans of both clips that are aligned, and the bones whose translation is not added.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AdditiveBase {
    /// The clip the delta is taken against.
    pub clip: AssetHandle<AnimationClip>,
    /// The span of the base clip aligned with [`Self::range`]; `None` for the whole clip.
    pub base_range: Option<ClipRange>,
    /// The span of this clip aligned with [`Self::base_range`]; `None` for the whole clip.
    pub range: Option<ClipRange>,
    /// The ids of the bones whose translation is left out of the delta.
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
    /// The start, in seconds from the clip's start.
    pub start: f32,
    /// The end, in seconds from the clip's start; after `start`.
    pub end: f32,
}

impl ClipRange {
    /// The span from `start` to `end` seconds.
    pub fn new(start: f32, end: f32) -> Self {
        Self { start, end }
    }
}

/// One clip's `[settings.additive.<clip>]` table in an importer's sidecar.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdditiveSetting {
    /// The clip the delta is taken against.
    pub base: AssetId,
    /// The span of the base clip that is aligned; the whole clip when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_range: Option<ClipRange>,
    /// The span of this clip that is aligned; the whole clip when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<ClipRange>,
}

impl AdditiveSetting {
    /// Checks `range` against a clip of `duration` seconds and builds the clip's [`Additive::LocalSpace`] with `roots`.
    ///
    /// # Errors
    ///
    /// Returns a message when `range` is set and is not finite with
    /// `0 <= start < end <= duration`.
    pub fn additive(&self, duration: f32, roots: Vec<Uuid>) -> Result<Additive, String> {
        if let Some(range) = self.range {
            range
                .check(duration, "range")
                .map_err(|err| err.to_string())?;
        }
        Ok(Additive::LocalSpace(AdditiveBase {
            clip: AssetHandle::weak(self.base),
            base_range: self.base_range,
            range: self.range,
            roots,
        }))
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
            let this = self.this.pose_at(time.this).rotation;
            let base = self.base.pose_at(time.base).rotation;
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
                self.this.pose_at(time.this).translation - self.base.pose_at(time.base).translation
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

impl AnimationChannel {
    fn pose_at(&self, time: f32) -> JointPose {
        let mut pose = JointPose::IDENTITY;
        if !self.constant(&mut pose) {
            self.sample_transform(time, &mut pose);
        }
        pose
    }
}

impl ClipRange {
    fn whole(duration: f32) -> Self {
        Self::new(0.0, duration)
    }

    fn check(self, duration: f32, name: &str) -> anyhow::Result<Self> {
        let range = self;
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

/// A skeletal animation asset: keyed channels per bone id, clip events, and whether the clip is
/// additive.
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
    /// Adds a channel for the bone `id`, after any it has, and extends the duration to its last
    /// key.
    pub fn add_channel(&mut self, id: Uuid, channel: AnimationChannel) {
        if let Some(channel_duration) = channel.duration() {
            self.duration = self.duration.max(channel_duration);
        }

        self.channels.entry(id).or_default().push(channel);
    }

    /// The ids of the bones the clip has channels for.
    pub fn target_ids(&self) -> Keys<'_, Uuid, Vec<AnimationChannel>> {
        self.channels.keys()
    }

    /// The channels for the bone `id`, in the order they were added.
    pub fn get_channels(&self, id: &Uuid) -> Option<&Vec<AnimationChannel>> {
        self.channels.get(id)
    }

    /// The clip's length in seconds: the last key time of any channel.
    pub fn duration(&self) -> f32 {
        self.duration
    }

    /// The skeleton's rest pose with every channel this clip has for its bones sampled at `time`.
    pub fn sample_pose(&self, skeleton: &Skeleton, time: f32) -> Vec<JointPose> {
        let mut pose = skeleton.rest_pose.to_vec();
        for (joint, id) in pose.iter_mut().zip(skeleton.bone_ids.iter()) {
            if let Some(channels) = self.get_channels(id) {
                for channel in channels {
                    channel.sample_transform(time, joint);
                }
            }
        }
        pose
    }

    /// Whether the clip is additive, and against what.
    pub fn additive(&self) -> &Additive {
        &self.additive
    }

    /// Makes the clip additive against a base, or plain with [`Additive::None`], dropping any delta already built.
    pub fn set_additive(&mut self, additive: Additive) {
        self.additive = additive;
        self.delta = None;
    }

    /// The additive delta of `bone` at `time`, clamped to the delta's span.
    ///
    /// Returns identity for a bone with no delta, a plain clip, or a delta not yet built.
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
        let range = additive
            .range
            .unwrap_or_else(|| ClipRange::whole(self.duration))
            .check(self.duration, "range")?;
        let base_range = additive
            .base_range
            .unwrap_or_else(|| ClipRange::whole(base.duration))
            .check(base.duration, "base_range")?;
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
    ///
    /// # Panics
    ///
    /// In debug builds, panics if `frame_rate` is not finite and above 0.
    pub fn set_frame_rate(&mut self, frame_rate: f32) {
        debug_assert!(frame_rate.is_finite() && frame_rate > 0.0);
        self.frame_rate = Some(snap_rate(frame_rate));
    }

    /// Writes the velocity of each bone in `bone_ids` between `time - dt` and `time`.
    ///
    /// Both times are clamped to the clip. Velocities are per second; a span of zero length
    /// writes zero.
    ///
    /// # Panics
    ///
    /// In debug builds, panics if `output` does not hold one linear and one angular velocity
    /// per bone in `bone_ids`. Release builds stop at the shorter length.
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

    /// Replaces the clip's events, sorted by time with equal times kept in the given order.
    ///
    /// # Errors
    ///
    /// Returns a message, and keeps the current events, when an event's time is negative,
    /// non-finite or past the clip's end.
    pub fn replace_events(&mut self, mut events: Vec<ClipEventKey>) -> Result<(), String> {
        if let Some(bad) = events
            .iter()
            .find(|key| !key.time.is_finite() || key.time < 0.0 || key.time > self.duration)
        {
            return Err(format!(
                "event `{}` at {} must be finite and within the clip's 0 to {} seconds",
                bad.event.type_name(),
                bad.time,
                self.duration
            ));
        }
        events.sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap_or(Ordering::Equal));
        self.events = events;
        Ok(())
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

    /// Adds an [`AnimationEvent`] per name and distinct time.
    ///
    /// # Errors
    ///
    /// Returns a message when a time is negative, non-finite or past the clip's end. Names
    /// before the failing one, in map order, have already been added.
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

    /// Builds every stored event from the registry, loading handles through `server`.
    ///
    /// An event that cannot be built is dropped and leaves a warning naming its type.
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
impl AnimationClip {
    pub(crate) fn with_delta_against(mut self, base: &AnimationClip) -> Self {
        let delta = self
            .delta_against(base)
            .expect("a test clip has a delta against its base");
        self.delta = Some(delta);
        self
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

    fn stored(time: f32, type_name: &str) -> ClipEventKey {
        ClipEventKey {
            time,
            event: ClipEventData::Stored {
                type_name: type_name.to_owned(),
                value: ReflectedValue::Struct(vec![(
                    "strength".to_owned(),
                    ReflectedValue::Leaf(vec![0, 0, 32, 64, 7]),
                )]),
            },
        }
    }

    fn type_names(clip: &AnimationClip) -> Vec<NamedKey<'_>> {
        clip.events()
            .iter()
            .map(|key| NamedKey {
                time: key.time,
                name: key.event.type_name(),
            })
            .collect()
    }

    #[test]
    fn replace_events_sorts_by_time_and_keeps_equal_times_in_the_given_order() {
        let mut clip = linear_clip(Uuid::from_u128(1));
        clip.add_event(0.3, named("old"));

        clip.replace_events(vec![
            stored(0.5, "second"),
            stored(0.1, "first"),
            stored(1.0, "fourth"),
            stored(0.5, "third"),
            stored(0.0, "zeroth"),
        ])
        .unwrap();

        assert_eq!(
            type_names(&clip),
            [
                key(0.0, "zeroth"),
                key(0.1, "first"),
                key(0.5, "second"),
                key(0.5, "third"),
                key(1.0, "fourth")
            ]
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
}
