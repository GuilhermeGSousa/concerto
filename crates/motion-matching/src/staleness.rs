use concerto_foundation::hash::Fnv1a;
use concerto_reflect::{to_value, Reflect, ReflectedValue};

use crate::{
    channel::FeatureChannelData,
    database::{BakeError, MotionDatabase},
};

const WEIGHT_FIELD: &str = "weight";

fn channel(hash: &mut Fnv1a, channel: &FeatureChannelData) {
    hash.bytes(channel.type_name().as_bytes());
    let value = channel.value().expect("a feature channel encodes");
    match value.as_ref() {
        ReflectedValue::Struct(fields) => {
            let kept = || fields.iter().filter(|(name, _)| name != WEIGHT_FIELD);
            hash.raw(&[1]);
            hash.count(kept().count());
            for (name, value) in kept() {
                hash.bytes(name.as_bytes());
                hash.value(value);
            }
        }
        value => hash.value(value),
    }
}

fn stored(value: &dyn Reflect) -> ReflectedValue {
    to_value(value).expect("a definition's settings encode")
}

impl MotionDatabase {
    /// A hash, stable across builds, of everything a bake reads except each channel's top-level `weight` field.
    pub fn weightless_hash(&self, skeleton_bytes: &[u8], clip_bytes: &[&[u8]]) -> u64 {
        let mut hash = Fnv1a::new();
        hash.value(&stored(&self.skeleton));
        hash.bytes(skeleton_bytes);
        hash.count(clip_bytes.len());
        for clip in clip_bytes {
            hash.bytes(clip);
        }
        hash.value(&stored(&self.entries));
        hash.value(&stored(&self.rate));
        hash.value(&stored(&self.speed));
        hash.value(&stored(&self.simulation));
        hash.value(&stored(&self.trajectory));
        hash.count(self.channels.len());
        for feature in &self.channels {
            channel(&mut hash, feature);
        }
        if self.entries.iter().any(|entry| entry.mirrored) {
            hash.value(&stored(&self.mirror_axis));
        }
        hash.finish()
    }
}

impl MotionDatabase {
    /// The weight of each channel of this database, in order, read from its live channels.
    ///
    /// # Errors
    ///
    /// Returns [`BakeError::UnloadedChannel`] when a channel is not loaded.
    pub fn weights(&self) -> Result<Vec<f32>, BakeError> {
        Ok(self
            .live_channels()?
            .iter()
            .map(|channel| channel.weight())
            .collect())
    }
}
