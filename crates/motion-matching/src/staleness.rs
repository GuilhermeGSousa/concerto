use concerto_reflect::{to_value, Reflect, ReflectedValue, TypeRegistry};

use crate::{
    channel::build_channels,
    database::{BakeError, MotionDatabase},
    definition::{MotionDatabaseDefinition, StoredChannel},
};

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

const WEIGHT_FIELD: &str = "weight";

struct Fnv1a(u64);

impl Fnv1a {
    fn new() -> Self {
        Self(FNV_OFFSET)
    }

    fn raw(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 = (self.0 ^ u64::from(*byte)).wrapping_mul(FNV_PRIME);
        }
    }

    fn count(&mut self, count: usize) {
        self.raw(&(count as u64).to_le_bytes());
    }

    fn bytes(&mut self, bytes: &[u8]) {
        self.count(bytes.len());
        self.raw(bytes);
    }

    fn fields<'a>(
        &mut self,
        count: usize,
        fields: impl Iterator<Item = &'a (String, ReflectedValue)>,
    ) {
        self.count(count);
        for (name, value) in fields {
            self.bytes(name.as_bytes());
            self.value(value);
        }
    }

    fn value(&mut self, value: &ReflectedValue) {
        match value {
            ReflectedValue::Leaf(bytes) => {
                self.raw(&[0]);
                self.bytes(bytes);
            }
            ReflectedValue::Struct(fields) => {
                self.raw(&[1]);
                self.fields(fields.len(), fields.iter());
            }
            ReflectedValue::Variant { name, fields } => {
                self.raw(&[2]);
                self.bytes(name.as_bytes());
                self.fields(fields.len(), fields.iter());
            }
            ReflectedValue::List(elements) => {
                self.raw(&[3]);
                self.count(elements.len());
                for element in elements {
                    self.value(element);
                }
            }
        }
    }

    fn channel(&mut self, channel: &StoredChannel) {
        self.bytes(channel.type_name.as_bytes());
        match &channel.value {
            ReflectedValue::Struct(fields) => {
                let kept = || fields.iter().filter(|(name, _)| name != WEIGHT_FIELD);
                self.raw(&[1]);
                self.fields(kept().count(), kept());
            }
            value => self.value(value),
        }
    }
}

fn stored(value: &dyn Reflect) -> ReflectedValue {
    to_value(value).expect("a definition's settings encode")
}

/// A hash, stable across builds, of everything a bake reads except each channel's top-level `weight` field.
pub fn weightless_hash(
    definition: &MotionDatabaseDefinition,
    skeleton_bytes: &[u8],
    clip_bytes: &[&[u8]],
) -> u64 {
    let mut hash = Fnv1a::new();
    hash.bytes(skeleton_bytes);
    hash.count(clip_bytes.len());
    for clip in clip_bytes {
        hash.bytes(clip);
    }
    hash.value(&stored(&definition.entries));
    hash.value(&stored(&definition.rate));
    hash.value(&stored(&definition.speed));
    hash.value(&stored(&definition.simulation));
    hash.value(&stored(&definition.trajectory));
    hash.count(definition.channels.len());
    for channel in &definition.channels {
        hash.channel(channel);
    }
    hash.0
}

/// The weight of each channel of `definition`, in order, read from its live channels.
pub fn weights_of(
    definition: &MotionDatabaseDefinition,
    registry: &TypeRegistry,
) -> Result<Vec<f32>, BakeError> {
    Ok(build_channels(&definition.channels, registry)?
        .iter()
        .map(|channel| channel.weight())
        .collect())
}

fn weights_differ(
    database: &MotionDatabase,
    definition: &MotionDatabaseDefinition,
    registry: &TypeRegistry,
) -> bool {
    weights_of(definition, registry).map_or(true, |weights| {
        !database
            .channels()
            .iter()
            .map(|span| span.weight)
            .eq(weights)
    })
}

/// Whether `database` no longer matches `definition`: `hash`, from [`weightless_hash`], differs from its source, or a channel's weight does.
pub fn is_stale(
    database: &MotionDatabase,
    definition: &MotionDatabaseDefinition,
    hash: u64,
    registry: &TypeRegistry,
) -> bool {
    database.source_hash() != hash || weights_differ(database, definition, registry)
}

/// [`is_stale`] for the case [`MotionDatabase::set_weights`] can fix: `hash` equals the database's source and a channel's weight differs.
pub fn weights_changed_only(
    database: &MotionDatabase,
    definition: &MotionDatabaseDefinition,
    hash: u64,
    registry: &TypeRegistry,
) -> bool {
    database.source_hash() == hash && weights_differ(database, definition, registry)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(bytes: &[u8]) -> u64 {
        let mut hash = Fnv1a::new();
        hash.raw(bytes);
        hash.0
    }

    #[test]
    fn the_hash_is_fnv_1a() {
        assert_eq!(digest(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(digest(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(digest(b"foobar"), 0x8594_4171_f739_67e8);
    }
}
