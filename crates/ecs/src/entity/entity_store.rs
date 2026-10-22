use std::{
    num::NonZero,
    sync::atomic::{AtomicI64, Ordering},
};

use crate::entity::{Entity, EntityLocation, EntityStructuralVersion};

#[derive(Clone, Copy)]
struct EntityData {
    current_generation: NonZero<u32>,
    location: EntityLocation,
    structural_version: EntityStructuralVersion,
}

impl EntityData {
    const EMPTY: EntityData = EntityData {
        current_generation: NonZero::<u32>::MIN,
        location: EntityLocation::INVALID,
        structural_version: EntityStructuralVersion(0),
    };
}

pub(crate) struct EntityStore {
    metadata: Vec<EntityData>,
    pending: Vec<u32>,
    free_cursor: AtomicI64,
}

impl EntityStore {
    pub fn new() -> Self {
        EntityStore {
            metadata: Vec::new(),
            pending: Vec::new(),
            free_cursor: AtomicI64::new(0),
        }
    }

    pub fn reserve(&self) -> Entity {
        let cursor = self.free_cursor.fetch_sub(1, Ordering::Relaxed);
        if cursor > 0 {
            let index = self.pending[cursor as usize - 1];
            Entity::new(index, self.metadata[index as usize].current_generation)
        } else {
            let index =
                u32::try_from(self.metadata.len() as i64 - cursor).expect("Entity index overflow");
            Entity::new(index, NonZero::<u32>::MIN)
        }
    }

    pub fn flush(&mut self) {
        let cursor = *self.free_cursor.get_mut();
        let free_len = self.pending.len() as i64;
        if cursor == free_len {
            return;
        }

        if cursor >= 0 {
            self.pending.truncate(cursor as usize);
        } else {
            self.pending.clear();
            let fresh = (-cursor) as usize;
            self.metadata
                .resize(self.metadata.len() + fresh, EntityData::EMPTY);
        }
        *self.free_cursor.get_mut() = self.pending.len() as i64;
    }

    pub fn alloc(&mut self) -> Entity {
        let entity = self.reserve();
        self.flush();
        entity
    }

    pub fn free(&mut self, entity: Entity) {
        self.flush();

        let meta = &mut self.metadata[entity.index() as usize];
        meta.current_generation =
            NonZero::new(meta.current_generation.get() + 1).expect("Entity generation overflow");
        meta.location = EntityLocation::INVALID;

        self.pending.push(entity.index());
        *self.free_cursor.get_mut() = self.pending.len() as i64;
    }

    pub fn set_location(&mut self, entity: Entity, location: EntityLocation) {
        self.flush();

        let meta = &mut self.metadata[entity.index() as usize];

        if meta.current_generation != entity.generation() {
            return;
        }

        if meta.location.archetype_index != location.archetype_index {
            meta.structural_version = meta.structural_version.next();
        }
        meta.location = location;
    }

    pub fn structural_version(&self, entity: Entity) -> Option<EntityStructuralVersion> {
        self.metadata
            .get(entity.index() as usize)
            .filter(|data| {
                data.location != EntityLocation::INVALID
                    && data.current_generation == entity.generation()
            })
            .map(|data| data.structural_version)
    }

    pub fn find_location(&self, entity: Entity) -> Option<EntityLocation> {
        self.metadata
            .get(entity.index() as usize)
            .filter(|data| {
                data.location != EntityLocation::INVALID
                    && data.current_generation == entity.generation()
            })
            .map(|data| data.location)
    }
}

impl Default for EntityStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::EntityStore;
    use crate::entity::EntityLocation;

    fn place(store: &mut EntityStore, entity: super::Entity) {
        store.set_location(
            entity,
            EntityLocation {
                archetype_index: 0,
                row: crate::table::TableRowIndex::new(0),
            },
        );
    }

    #[test]
    fn a_reserved_entity_is_not_found_until_flushed_and_placed() {
        let mut store = EntityStore::new();
        let entity = store.reserve();
        assert!(store.find_location(entity).is_none());

        store.flush();
        assert!(store.find_location(entity).is_none());

        place(&mut store, entity);
        assert!(store.find_location(entity).is_some());
    }

    #[test]
    fn reservations_reuse_freed_indices_last_freed_first_with_a_new_generation() {
        let mut store = EntityStore::new();
        let a = store.alloc();
        let b = store.alloc();
        store.free(a);
        store.free(b);

        let first = store.reserve();
        let second = store.reserve();
        let fresh = store.reserve();
        store.flush();

        assert_eq!(first.index(), b.index());
        assert_ne!(first.generation(), b.generation());
        assert_eq!(second.index(), a.index());
        assert_eq!(fresh.index(), 2);

        let after = store.alloc();
        assert_eq!(after.index(), 3);
    }

    #[test]
    fn flush_consumes_only_the_reserved_part_of_the_free_list() {
        let mut store = EntityStore::new();
        let entities: Vec<_> = (0..3).map(|_| store.alloc()).collect();
        for entity in &entities {
            store.free(*entity);
        }

        let reused = store.reserve();
        store.flush();

        let next = store.alloc();
        assert_ne!(next.index(), reused.index());
        assert!(entities.iter().any(|entity| entity.index() == next.index()));
    }

    #[test]
    fn concurrent_reservations_are_unique() {
        let mut store = EntityStore::new();
        let recycled: Vec<_> = (0..64).map(|_| store.alloc()).collect();
        for entity in &recycled {
            store.free(*entity);
        }

        let reserved: Vec<_> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..4)
                .map(|_| scope.spawn(|| (0..1000).map(|_| store.reserve()).collect::<Vec<_>>()))
                .collect();
            handles
                .into_iter()
                .flat_map(|handle| handle.join().unwrap())
                .collect()
        });
        store.flush();

        let unique: HashSet<_> = reserved.iter().map(|entity| entity.index()).collect();
        assert_eq!(unique.len(), reserved.len());
        for entity in &reserved {
            place(&mut store, *entity);
            assert!(store.find_location(*entity).is_some());
        }
    }
}
