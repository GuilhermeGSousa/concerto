use std::{
    fmt::{Debug, Display},
    num::NonZero,
};

use crate::{
    component::bundle::ComponentBundle,
    entity::hierarchy::{ChildOf, Children},
    table::TableRowIndex,
    world::World,
};

pub mod entity_store;
pub mod hierarchy;

/// A lightweight, copyable handle that uniquely identifies a game object in the [`World`](crate::world::World).
///
/// Entities are created with [`World::spawn`](crate::world::World::spawn) and destroyed with
/// [`World::despawn`](crate::world::World::despawn).  An entity is just an `(index, generation)`
/// pair — the generation is bumped each time a slot is reused so stale handles can be detected.
#[derive(Eq, Hash, PartialEq, Clone, Copy)]
pub struct Entity {
    index: u32,
    generation: NonZero<u32>,
}

impl Entity {
    pub(crate) fn new(index: u32, generation: NonZero<u32>) -> Self {
        Self { index, generation }
    }

    /// Returns the slot index of this entity within the entity store.
    pub fn index(&self) -> u32 {
        self.index
    }

    /// Returns the generation counter that distinguishes this entity from previously-occupying
    /// entities at the same index.
    pub fn generation(&self) -> NonZero<u32> {
        self.generation
    }
}

impl Debug for Entity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Entity")
            .field("index", &self.index)
            .field("generation", &self.generation)
            .finish()
    }
}

impl Display for Entity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Entity(index: {}, gen: {})", self.index, self.generation)
    }
}

#[derive(Eq, Hash, PartialEq, Clone)]
pub struct EntityType(pub u64);

#[derive(Clone, Copy, PartialEq)]
pub struct EntityLocation {
    pub(crate) archetype_index: u32,
    pub(crate) row: TableRowIndex,
}

impl EntityLocation {
    pub(crate) const INVALID: EntityLocation = EntityLocation {
        archetype_index: u32::MAX,
        row: TableRowIndex::new(usize::MAX),
    };
}

/// An opaque version of an entity's component set; compare only for the same entity handle.
/// Wraps after 65,536 changes, so a full cycle between observations is indistinguishable
/// from no change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntityStructuralVersion(u16);

impl EntityStructuralVersion {
    fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

/// A borrow of a single entity, used to edit its place in the hierarchy.
pub struct EntityWorldMut<'w> {
    world: &'w mut World,
    entity: Entity,
}

impl<'w> EntityWorldMut<'w> {
    pub(crate) fn new(world: &'w mut World, entity: Entity) -> Self {
        Self { world, entity }
    }

    pub fn id(&self) -> Entity {
        self.entity
    }

    /// Makes `child` a child of this entity, detaching it from its previous parent.
    pub fn add_child(&mut self, child: Entity) -> &mut Self {
        let parent = self
            .world
            .get_component_for_entity::<ChildOf>(child)
            .map(ChildOf::parent);
        if parent != Some(self.entity) {
            self.world.insert(ChildOf::new(self.entity), child);
        }
        self
    }

    pub fn add_children(&mut self, children: &[Entity]) -> &mut Self {
        // Who is already attached is one lookup on this entity, not one per child:
        // `Children` holds exactly the entities whose `ChildOf` points here.
        let attached: Vec<Entity> = self
            .world
            .get_component_for_entity::<Children>(self.entity)
            .map(|attached| attached.iter().copied().collect())
            .unwrap_or_default();

        for child in children {
            if !attached.contains(child) {
                self.world.insert(ChildOf::new(self.entity), *child);
            }
        }
        self
    }

    /// Spawns an entity as a child of this one and borrows it in turn.
    pub fn spawn_child<T: ComponentBundle>(&mut self, bundle: T) -> EntityWorldMut<'_> {
        let child = self.world.spawn(bundle);
        self.add_child(child);
        EntityWorldMut::new(self.world, child)
    }
}

#[cfg(test)]
mod structural_version_tests {
    use super::EntityStructuralVersion;

    #[test]
    fn structural_version_wraps_and_continues_incrementing() {
        let wrapped = EntityStructuralVersion(u16::MAX).next();
        assert_eq!(wrapped, EntityStructuralVersion(0));
        assert_eq!(wrapped.next(), EntityStructuralVersion(1));
    }
}
