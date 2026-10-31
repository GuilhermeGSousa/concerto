use concerto_ecs::Component;
use concerto_foundation::transform::Transform;
use concerto_mesh::{JointPose, Skeleton};
use concerto_reflect::{
    apply_value, to_value, EncodeError, FromType, Reflect, ReflectedValue, TypeData, TypeRegistry,
};
use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::{
    database::BakeError,
    definition::{StoredChannel, TrajectorySettings},
    simulation::TrajectorySample,
};

/// One group of values a motion database stores per frame; implementors derive `Component`, `Reflect` and `Default` and declare `#[reflect(Component, FeatureChannel)]`.
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

/// The simulation bone's linear and angular velocity, in the space of `FrameContext::simulation`.
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

/// Builds a live [`FeatureChannel`] from a stored value and stores one back.
#[derive(Clone, Copy)]
pub struct ReflectFeatureChannel {
    pub from_value: fn(&ReflectedValue) -> Result<Box<dyn FeatureChannel>, String>,
    pub to_value: fn(&dyn FeatureChannel) -> Result<ReflectedValue, EncodeError>,
}

impl TypeData for ReflectFeatureChannel {}

impl<T: FeatureChannel + Component + Default> FromType<T> for ReflectFeatureChannel {
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

/// Builds the live channels of a definition, in order; a type name the registry lacks, or one without [`ReflectFeatureChannel`], is an error.
pub fn build_channels(
    stored: &[StoredChannel],
    registry: &TypeRegistry,
) -> Result<Vec<Box<dyn FeatureChannel>>, BakeError> {
    stored
        .iter()
        .map(|channel| {
            let data = registry
                .get_by_name(&channel.type_name)
                .and_then(|registration| registration.data::<ReflectFeatureChannel>().copied())
                .ok_or_else(|| BakeError::UnregisteredChannel(channel.type_name.clone()))?;
            (data.from_value)(&channel.value).map_err(|reason| BakeError::ChannelValue {
                type_name: channel.type_name.clone(),
                reason,
            })
        })
        .collect()
}
