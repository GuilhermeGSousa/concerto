use concerto_foundation::transform::Transform;
use concerto_mesh::{JointPose, Skeleton};
use concerto_reflect::{
    apply_value, to_value, EncodeError, FromType, Reflect, ReflectedValue, TypeData, TypeRegistry,
};
use glam::Vec3;

use crate::{database::BakeError, definition::StoredChannel};

/// One group of values a motion database stores per frame; implementors derive `Reflect` and `Default` and declare `#[reflect(FeatureChannel)]`.
pub trait FeatureChannel: Reflect + Send + Sync + 'static {
    /// How many `f32` values the channel writes per frame.
    fn dims(&self) -> usize;

    /// Writes `dims()` values for `frame` into `out`.
    fn extract(&self, frame: &FrameContext, out: &mut [f32]);
}

/// Everything known about one database frame while its features are extracted.
pub struct FrameContext<'a> {
    pub skeleton: &'a Skeleton,
    pub local_pose: &'a [JointPose],
    pub model_pose: &'a [Transform],
    pub model_velocities: &'a [Vec3],
    pub simulation: Transform,
    pub range_simulation: &'a [Transform],
    pub range_index: usize,
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
