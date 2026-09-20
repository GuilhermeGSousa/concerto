use std::num::NonZero;

use crate::entity::{Entity, EntityLocation};

#[derive(Clone, Copy)]
struct EntityData {
    current_generation: NonZero<u32>,
    location: EntityLocation,
}

impl EntityData {
    const EMPTY: EntityData = EntityData {
        current_generation: NonZero::<u32>::MIN,
        location: EntityLocation::INVALID,
    };
}

pub(crate) struct EntityStore {
    metadata: Vec<EntityData>,
    pending: Vec<u32>,
    total: u32,
}

impl EntityStore {
    pub fn new() -> Self {
        EntityStore {
            metadata: Vec::new(),
            pending: Vec::new(),
            total: 0,
        }
    }

    pub fn alloc(&mut self) -> Entity {
        self.total += 1;

        if let Some(index) = self.pending.pop() {
            Entity::new(index, self.metadata[index as usize].current_generation)
        } else {
            let index = self.metadata.len() as u32;
            self.metadata.push(EntityData::EMPTY);
            Entity::new(index, NonZero::<u32>::MIN)
        }
    }

    pub fn free(&mut self, entity: Entity) {
        self.total -= 1;

        let meta = &mut self.metadata[entity.index() as usize];
        meta.current_generation =
            NonZero::new(meta.current_generation.get() + 1).expect("Entity generation overflow");
        meta.location = EntityLocation::INVALID;

        self.pending.push(entity.index());
    }

    pub fn set_location(&mut self, entity: Entity, location: EntityLocation) {
        let meta = &mut self.metadata[entity.index() as usize];

        if meta.current_generation != entity.generation() {
            return;
        }

        meta.location = location;
    }

    pub fn find_location(&self, entity: Entity) -> Option<EntityLocation> {
        if let Some(data) = self.metadata.get(entity.index() as usize) {
            if data.location == EntityLocation::INVALID {
                return None;
            }

            if entity.generation() == data.current_generation {
                return Some(data.location);
            }
            None
        } else {
            None
        }
    }
}
