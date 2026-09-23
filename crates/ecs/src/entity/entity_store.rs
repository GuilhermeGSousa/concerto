use std::num::NonZero;

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
