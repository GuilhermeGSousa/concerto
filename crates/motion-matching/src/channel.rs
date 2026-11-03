use std::{borrow::Cow, fmt};

use concerto_foundation::transform::Transform;
use concerto_mesh::{JointPose, Skeleton};
use concerto_reflect::{
    apply_value, to_value, EncodeError, FromType, Reflect, ReflectedValue, TypeData, TypeRegistry,
};
use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::{database::BakeError, definition::TrajectorySettings, simulation::TrajectorySample};

/// One group of values a motion database stores per frame; implementors derive `Reflect`, `Default` and `Clone` and declare `#[reflect(FeatureChannel)]`.
pub trait FeatureChannel: Reflect + DynCloneChannel + Send + Sync + 'static {
    /// How many `f32` values the channel writes per frame.
    fn dims(&self) -> usize;

    /// Writes `dims()` values for `frame` into `out`.
    fn extract(&self, frame: &FrameContext, out: &mut [f32]);

    /// How strongly the channel's group counts against the others; positive and finite.
    fn weight(&self) -> f32 {
        1.0
    }

    /// Whether the channel reads only the simulation bone and the trajectory, or the pose too.
    fn source(&self) -> ChannelSource {
        ChannelSource::Pose
    }

    /// Checks the channel against the skeleton and trajectory settings before a bake; the reason of an `Err` becomes `BakeError::ChannelValue`.
    fn validate(&self, skeleton: &Skeleton, trajectory: &TrajectorySettings) -> Result<(), String> {
        let _ = (skeleton, trajectory);
        Ok(())
    }

    /// Adds the shapes that draw `features`, this channel's raw `dims()` values, to `out`; `frame` only places what the values hold no position for.
    fn preview(&self, frame: &FrameContext, features: &[f32], out: &mut Vec<FeatureShape>) {
        let _ = (frame, features, out);
    }
}

/// A feature channel as an asset holds it: its registered type name with the stored value, or the live channel once loaded.
pub enum FeatureChannelData {
    Stored {
        type_name: String,
        value: ReflectedValue,
    },
    Live {
        type_name: String,
        channel: Box<dyn FeatureChannel>,
    },
}

impl FeatureChannelData {
    /// `channel`, live, under its type's short name.
    pub fn live<T: FeatureChannel>(channel: T) -> Self {
        Self::Live {
            type_name: short_name(std::any::type_name::<T>()).to_owned(),
            channel: Box::new(channel),
        }
    }

    /// The registered type name a save writes back.
    pub fn type_name(&self) -> &str {
        match self {
            Self::Stored { type_name, .. } | Self::Live { type_name, .. } => type_name,
        }
    }

    /// The live channel; a channel not yet loaded is an error.
    pub fn channel(&self) -> Result<&dyn FeatureChannel, BakeError> {
        match self {
            Self::Live { channel, .. } => Ok(channel.as_ref()),
            Self::Stored { type_name, .. } => Err(BakeError::UnloadedChannel(type_name.clone())),
        }
    }

    /// The reflected value a save writes.
    pub fn value(&self) -> Result<Cow<'_, ReflectedValue>, EncodeError> {
        match self {
            Self::Stored { value, .. } => Ok(Cow::Borrowed(value)),
            Self::Live { channel, .. } => to_value(channel.as_ref()).map(Cow::Owned),
        }
    }

    /// Builds the live channel from the stored value; a type the registry lacks, or one without [`ReflectFeatureChannel`], is an error.
    pub fn load(&mut self, registry: &TypeRegistry) -> Result<(), BakeError> {
        let Self::Stored { type_name, value } = self else {
            return Ok(());
        };
        let data = registry
            .get_by_name(type_name)
            .and_then(|registration| registration.data::<ReflectFeatureChannel>().copied())
            .ok_or_else(|| BakeError::UnregisteredChannel(type_name.clone()))?;
        let channel = (data.from_value)(value).map_err(|reason| BakeError::ChannelValue {
            type_name: type_name.clone(),
            reason,
        })?;
        let type_name = std::mem::take(type_name);
        *self = Self::Live { type_name, channel };
        Ok(())
    }
}

#[derive(Serialize)]
struct StoredRef<'a> {
    type_name: &'a str,
    value: Cow<'a, ReflectedValue>,
}

#[derive(Deserialize)]
struct Stored {
    type_name: String,
    value: ReflectedValue,
}

impl Serialize for FeatureChannelData {
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

impl<'de> Deserialize<'de> for FeatureChannelData {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let Stored { type_name, value } = Stored::deserialize(deserializer)?;
        Ok(Self::Stored { type_name, value })
    }
}

impl Clone for FeatureChannelData {
    fn clone(&self) -> Self {
        match self {
            Self::Stored { type_name, value } => Self::Stored {
                type_name: type_name.clone(),
                value: value.clone(),
            },
            Self::Live { type_name, channel } => Self::Live {
                type_name: type_name.clone(),
                channel: channel.dyn_clone(),
            },
        }
    }
}

impl fmt::Debug for FeatureChannelData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let variant = match self {
            Self::Stored { .. } => "Stored",
            Self::Live { .. } => "Live",
        };
        f.debug_struct(variant)
            .field("type_name", &self.type_name())
            .field("value", &self.value())
            .finish()
    }
}

impl PartialEq for FeatureChannelData {
    fn eq(&self, other: &Self) -> bool {
        self.type_name() == other.type_name()
            && matches!((self.value(), other.value()), (Ok(mine), Ok(theirs)) if mine == theirs)
    }
}

fn short_name(type_path: &str) -> &str {
    let without_generics = type_path.split('<').next().unwrap_or(type_path);
    let start = without_generics.rfind("::").map_or(0, |index| index + 2);
    &type_path[start..]
}

/// Boxes a copy of a feature channel; every `Clone` channel has it.
pub trait DynCloneChannel {
    fn dyn_clone(&self) -> Box<dyn FeatureChannel>;
}

impl<T: FeatureChannel + Clone> DynCloneChannel for T {
    fn dyn_clone(&self) -> Box<dyn FeatureChannel> {
        Box::new(self.clone())
    }
}

/// One thing a channel draws, relative to the simulation bone; a `Ring` lies flat in the bone's ground plane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FeatureShape {
    Ring { center: Vec3, radius: f32 },
    Line { start: Vec3, end: Vec3 },
    Arrow { start: Vec3, end: Vec3 },
}

/// What a channel reads: `Trajectory` channels use only `FrameContext::simulation` and `FrameContext::trajectory`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChannelSource {
    Pose,
    Trajectory,
}

/// Everything a channel reads to extract the features of one frame, baked offline or queried online.
pub struct FrameContext<'a> {
    pub skeleton: &'a Skeleton,
    pub local_pose: &'a [JointPose],
    pub model_pose: &'a [Transform],
    pub model_velocities: &'a [Vec3],
    pub simulation: Transform,
    pub body: BodyMotion,
    pub trajectory: &'a [TrajectorySample],
}

/// The simulation bone's linear and angular velocity, in the same space as `FrameContext::simulation`, not in the bone's own frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BodyMotion {
    pub velocity: Vec3,
    pub angular_velocity: Vec3,
}

/// Runs every channel into its span of `out`, in list order; `out` holds exactly the sum of their `dims()`.
pub fn extract_features(channels: &[&dyn FeatureChannel], frame: &FrameContext, out: &mut [f32]) {
    let mut offset = 0;
    for channel in channels {
        let dims = channel.dims();
        channel.extract(frame, &mut out[offset..offset + dims]);
        offset += dims;
    }
}

/// The shapes of every channel for one raw feature vector, one list per channel in order.
pub fn preview_features(
    channels: &[Box<dyn FeatureChannel>],
    frame: &FrameContext,
    features: &[f32],
) -> Vec<Vec<FeatureShape>> {
    let mut offset = 0;
    channels
        .iter()
        .map(|channel| {
            let dims = channel.dims();
            let mut shapes = Vec::new();
            channel.preview(frame, &features[offset..offset + dims], &mut shapes);
            offset += dims;
            shapes
        })
        .collect()
}

/// Builds a live [`FeatureChannel`] from a stored value and stores one back.
#[derive(Clone, Copy)]
pub struct ReflectFeatureChannel {
    pub from_value: fn(&ReflectedValue) -> Result<Box<dyn FeatureChannel>, String>,
    pub to_value: fn(&dyn FeatureChannel) -> Result<ReflectedValue, EncodeError>,
}

impl TypeData for ReflectFeatureChannel {}

impl<T: FeatureChannel + Default> FromType<T> for ReflectFeatureChannel {
    fn from_type() -> Self {
        Self {
            from_value: from_value::<T>,
            to_value: store,
        }
    }
}

fn from_value<T: FeatureChannel + Default>(
    stored: &ReflectedValue,
) -> Result<Box<dyn FeatureChannel>, String> {
    let mut channel = T::default();
    let warnings = apply_value(&mut channel, stored);
    if warnings.is_empty() {
        Ok(Box::new(channel))
    } else {
        Err(warnings
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; "))
    }
}

fn store(channel: &dyn FeatureChannel) -> Result<ReflectedValue, EncodeError> {
    to_value(channel)
}
