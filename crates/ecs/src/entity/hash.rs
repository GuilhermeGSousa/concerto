use std::{
    collections::{HashMap, HashSet},
    hash::{BuildHasher, Hasher},
};

use crate::entity::Entity;

/// A [`BuildHasher`] for keys that hash as one entity's bits.
#[derive(Clone, Copy, Default, Debug)]
pub struct EntityHash;

impl BuildHasher for EntityHash {
    type Hasher = EntityHasher;

    fn build_hasher(&self) -> Self::Hasher {
        EntityHasher::default()
    }
}

/// A [`Hasher`] that takes exactly one `u64`, an entity's [`to_bits`](Entity::to_bits).
#[derive(Default, Debug)]
pub struct EntityHasher {
    hash: u64,
}

impl Hasher for EntityHasher {
    fn finish(&self) -> u64 {
        self.hash
    }

    fn write(&mut self, _: &[u8]) {
        panic!("EntityHasher can only hash u64 fields.")
    }

    fn write_u64(&mut self, bits: u64) {
        self.hash = bits.wrapping_mul(0x9e37_79b9_0000_0001);
    }
}

/// A [`HashMap`] keyed by [`Entity`] with the [`EntityHash`] hasher.
pub type EntityHashMap<V> = HashMap<Entity, V, EntityHash>;

/// A [`HashSet`] of [`Entity`] with the [`EntityHash`] hasher.
pub type EntityHashSet = HashSet<Entity, EntityHash>;

#[cfg(test)]
mod tests {
    use std::num::NonZero;

    use super::EntityHashMap;
    use crate::entity::Entity;

    #[test]
    fn an_entity_hash_map_finds_entities_that_share_an_index() {
        let older = Entity::new(7, NonZero::new(1).unwrap());
        let newer = Entity::new(7, NonZero::new(2).unwrap());
        let mut map = EntityHashMap::default();
        map.insert(older, "older");
        map.insert(newer, "newer");
        assert_eq!(map.len(), 2);
        assert_eq!(map.get(&older), Some(&"older"));
        assert_eq!(map.get(&newer), Some(&"newer"));
    }
}
