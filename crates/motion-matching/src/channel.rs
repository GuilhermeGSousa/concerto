use std::fmt;

use concerto_ecs::reflect::AppTypeRegistry;
use concerto_foundation::transform::Transform;
use concerto_mesh::{JointPose, Skeleton};
use concerto_reflect::{
    apply_value, to_value, EncodeError, FromType, Reflect, ReflectedValue, TypeData, TypeRegistry,
};
use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::{database::BakeError, definition::TrajectorySettings, simulation::TrajectorySample};

/// One group of values a motion database stores per frame; implementors derive `Reflect` and `Default` and declare `#[reflect(FeatureChannel)]`.
pub trait FeatureChannel: Reflect + Send + Sync + 'static {
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

pub enum FeatureChannelData {
    Live {
        type_name: String,
        offset: usize,
        data: Box<dyn FeatureChannel>,
    },
    Stored {
        type_name: String,
        offset: usize,
        dims: usize,
        weight: f32,
        source: ChannelSource,
        value: ReflectedValue,
    },
}

impl FeatureChannelData {
    pub fn stored(&self) -> Self {
        match self {
            FeatureChannelData::Live { type_name, offset, data } => Self::Stored {
                type_name: type_name.clone(),
                offset: *offset,
                dims: data.dims(),
                weight: data.weight(),
                source: data.source(),
                value: to_value(data.as_ref()).expect(""),
            },
            FeatureChannelData::Stored {
                type_name,
                offset,
                dims,
                weight,
                source,
                value,
            } => Self::Stored {
                type_name: type_name.clone(),
                offset: *offset,
                dims: *dims,
                weight: *weight,
                source: *source,
                value: value.clone(),
            },
        }
    }

    pub fn load(&mut self, registry: &AppTypeRegistry) {
        todo!()
    }

    pub fn dims(&self) -> usize
    {
        match self {
            FeatureChannelData::Live { type_name: _, offset: _, data } => data.dims(),
            FeatureChannelData::Stored { type_name: _, offset: _, dims, weight: _, source: _, value: _ } => *dims,
        }
    }

    pub fn weight(&self) -> f32
    {
        match self {
            FeatureChannelData::Live { type_name: _, offset: _, data } => data.weight(),
            FeatureChannelData::Stored { type_name: _, offset: _, dims: _, weight, source: _, value: _ } => *weight,
        }
    }

    pub fn source(&self) -> ChannelSource
    {
        match self {
            FeatureChannelData::Live { type_name: _, offset: _, data } => data.source(),
            FeatureChannelData::Stored { type_name: _, offset: _, dims: _, weight: _, source, value: _ } => *source,
        }
    }

    pub fn offset(&self) -> usize
    {
        match self {
            FeatureChannelData::Live { type_name: _, offset, data: _ } => *offset,
            FeatureChannelData::Stored { type_name: _, offset, dims: _, weight: _, source: _, value: _ } => *offset,
        }
    }
}

impl Serialize for FeatureChannelData {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        todo!()
    }
}

impl<'de> Deserialize<'de> for FeatureChannelData {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        todo!()
    }
}

impl Clone for FeatureChannelData {
    fn clone(&self) -> Self {
        todo!()
    }
}

impl fmt::Debug for FeatureChannelData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        todo!()
    }
}

impl PartialEq for FeatureChannelData {
    fn eq(&self, other: &Self) -> bool {
        todo!()
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
pub fn extract_features(
    channels: &[Box<dyn FeatureChannel>],
    frame: &FrameContext,
    out: &mut [f32],
) {
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
