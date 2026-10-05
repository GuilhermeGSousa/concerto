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

    /// A compact text id for tools outside the process, e.g. `42v3`.
    ///
    /// The generation is part of the id, so a tool holding an id from before a
    /// slot was reused gets a stale-entity error instead of the new occupant.
    pub fn to_id_string(&self) -> String {
        format!("{}v{}", self.index, self.generation)
    }

    /// Parses an id written by [`to_id_string`](Self::to_id_string). The entity
    /// may no longer exist; check with
    /// [`World::entity_is_valid`](crate::world::World::entity_is_valid).
    pub fn parse_id(id: &str) -> Option<Entity> {
        let (index, generation) = id.trim().split_once('v')?;
        Some(Entity::new(
            index.parse().ok()?,
            NonZero::new(generation.parse().ok()?)?,
        ))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_strings_round_trip() {
        let mut world = World::new();
        world.spawn(());
        let entity = world.spawn(());
        let id = entity.to_id_string();
        assert_eq!(id, format!("{}v{}", entity.index(), entity.generation()));
        assert_eq!(Entity::parse_id(&id), Some(entity));
    }

    #[test]
    fn malformed_ids_do_not_parse() {
        for id in ["", "42", "v3", "42v", "42v0", "-1v1", "42x3", "42v3v1"] {
            assert_eq!(Entity::parse_id(id), None, "{id:?} must not parse");
        }
    }
}
