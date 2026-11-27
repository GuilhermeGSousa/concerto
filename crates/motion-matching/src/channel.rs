use std::{borrow::Cow, fmt};

use concerto_foundation::transform::Transform;
use concerto_mesh::{JointPose, Skeleton};
use concerto_reflect::{
    apply_value, to_value, EncodeError, FromType, Reflect, ReflectedValue, TypeData, TypeRegistry,
};
use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::{database::BakeError, definition::TrajectorySettings, simulation::TrajectorySample};

/// One group of values a motion database stores per frame.
///
/// Implementors derive `Reflect`, `Default` and `Clone` and declare
/// `#[reflect(FeatureChannel)]`, so a database can store the channel by type
/// name and rebuild it through [`ReflectFeatureChannel`].
pub trait FeatureChannel: Reflect + DynCloneChannel + Send + Sync + 'static {
    /// How many `f32` values the channel writes per frame.
    fn dims(&self) -> usize;

    /// Writes [`dims`](Self::dims) values for `frame` into `out`.
    fn extract(&self, frame: &FrameContext, out: &mut [f32]);

    /// How strongly the channel's group counts against the others.
    ///
    /// It must be positive and finite; a database with any other weight
    /// searches nothing.
    fn weight(&self) -> f32 {
        1.0
    }

    /// Whether the channel reads only the simulation bone and the trajectory, or the pose too.
    fn source(&self) -> ChannelSource {
        ChannelSource::Pose
    }

    /// Checks the channel against the skeleton and trajectory settings before a bake.
    ///
    /// # Errors
    ///
    /// Returns the reason the channel cannot be baked with these settings; the
    /// bake reports it as [`BakeError::ChannelValue`].
    fn validate(&self, skeleton: &Skeleton, trajectory: &TrajectorySettings) -> Result<(), String> {
        let _ = (skeleton, trajectory);
        Ok(())
    }

    /// Adds the shapes that draw `features`, this channel's raw [`dims`](Self::dims) values, to `out`.
    ///
    /// `frame` only places what the values hold no position for.
    fn preview(&self, frame: &FrameContext, features: &[f32], out: &mut Vec<FeatureShape>) {
        let _ = (frame, features, out);
    }
}

/// A feature channel as an asset holds it.
///
/// `Stored` is its registered type name with the stored value, as a load
/// decodes it; `Live` is the channel built from it by [`load`](Self::load).
pub enum FeatureChannelData {
    /// Read from a file, not yet built into the channel's type.
    Stored {
        /// The registered short name of the channel's type.
        type_name: String,
        /// The channel's reflected fields.
        value: ReflectedValue,
    },
    /// Built into the channel's type, ready to extract.
    Live {
        /// The registered short name of the channel's type.
        type_name: String,
        /// The built channel.
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

    /// The live channel.
    ///
    /// # Errors
    ///
    /// Returns [`BakeError::UnloadedChannel`] for a channel not yet loaded.
    pub fn channel(&self) -> Result<&dyn FeatureChannel, BakeError> {
        match self {
            Self::Live { channel, .. } => Ok(channel.as_ref()),
            Self::Stored { type_name, .. } => Err(BakeError::UnloadedChannel(type_name.clone())),
        }
    }

    /// The reflected value a save writes.
    ///
    /// # Errors
    ///
    /// Returns an error when a live channel does not encode.
    pub fn value(&self) -> Result<Cow<'_, ReflectedValue>, EncodeError> {
        match self {
            Self::Stored { value, .. } => Ok(Cow::Borrowed(value)),
            Self::Live { channel, .. } => to_value(channel.as_ref()).map(Cow::Owned),
        }
    }

    /// Builds the live channel from the stored value.
    ///
    /// A channel already live is left as it is.
    ///
    /// # Errors
    ///
    /// Returns [`BakeError::UnregisteredChannel`] when the registry lacks the
    /// type or it has no [`ReflectFeatureChannel`], and
    /// [`BakeError::ChannelValue`] when the stored value does not apply to it.
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
    /// A boxed copy of the channel.
    fn dyn_clone(&self) -> Box<dyn FeatureChannel>;
}

impl<T: FeatureChannel + Clone> DynCloneChannel for T {
    fn dyn_clone(&self) -> Box<dyn FeatureChannel> {
        Box::new(self.clone())
    }
}

/// One thing a channel draws, relative to the simulation bone.
///
/// A `Ring` lies flat in the bone's ground plane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FeatureShape {
    /// A circle.
    Ring {
        /// The centre, relative to the simulation bone.
        center: Vec3,
        /// The radius, in metres.
        radius: f32,
    },
    /// A line segment.
    Line {
        /// One end, relative to the simulation bone.
        start: Vec3,
        /// The other end, relative to the simulation bone.
        end: Vec3,
    },
    /// An arrow from `start` pointing at `end`.
    Arrow {
        /// The tail, relative to the simulation bone.
        start: Vec3,
        /// The head, relative to the simulation bone.
        end: Vec3,
    },
}

/// What a channel reads.
///
/// `Trajectory` channels use only [`FrameContext::simulation`] and
/// [`FrameContext::trajectory`]; `Pose` channels read the pose too.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChannelSource {
    /// The channel reads the pose as well as the simulation bone and the trajectory.
    Pose,
    /// The channel reads only the simulation bone and the trajectory.
    Trajectory,
}

/// Everything a channel reads to extract the features of one frame, baked offline or queried online.
pub struct FrameContext<'a> {
    /// The skeleton the pose is of.
    pub skeleton: &'a Skeleton,
    /// Each bone's local pose, parentless bones relative to the simulation bone; empty online.
    pub local_pose: &'a [JointPose],
    /// Each bone's model-space transform, in bone order; empty online.
    pub model_pose: &'a [Transform],
    /// Each bone's model-space velocity in metres per second, in bone order; empty online.
    pub model_velocities: &'a [Vec3],
    /// The simulation bone's transform, in model space when baking and world space online.
    pub simulation: Transform,
    /// The simulation bone's velocities.
    pub body: BodyMotion,
    /// The trajectory samples ahead of the frame, in the same space as `simulation`.
    pub trajectory: &'a [TrajectorySample],
}

/// The simulation bone's linear and angular velocity.
///
/// Both are in the same space as [`FrameContext::simulation`], not in the
/// bone's own frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BodyMotion {
    /// The linear velocity, in metres per second.
    pub velocity: Vec3,
    /// The angular velocity, as a scaled axis in radians per second.
    pub angular_velocity: Vec3,
}

/// Runs every channel into its span of `out`, in list order.
///
/// `out` holds exactly the sum of their [`dims`](FeatureChannel::dims).
///
/// # Panics
///
/// Panics if `out` is shorter than the sum of the channels' dims.
pub fn extract_features(channels: &[&dyn FeatureChannel], frame: &FrameContext, out: &mut [f32]) {
    let mut offset = 0;
    for channel in channels {
        let dims = channel.dims();
        channel.extract(frame, &mut out[offset..offset + dims]);
        offset += dims;
    }
}

/// The shapes of every channel for one raw feature vector, one list per channel in order.
///
/// # Panics
///
/// Panics if `features` is shorter than the sum of the channels' dims.
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
    /// Builds the channel from its stored fields.
    ///
    /// # Errors
    ///
    /// Returns the reason the stored fields do not apply to the channel's type.
    pub from_value: fn(&ReflectedValue) -> Result<Box<dyn FeatureChannel>, String>,
    /// Stores the channel's fields.
    ///
    /// # Errors
    ///
    /// Returns an [`EncodeError`] when the channel does not encode through reflection.
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
