use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{component::Component, entity::Entity, resource::Resource, world::World};

/// An entity reference stored as a scene-node index while serialized and
/// replaced with the spawned [`Entity`] when the scene is applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneEntityRef {
    Index(usize),
    Entity(Entity),
}

impl SceneEntityRef {
    pub fn entity(self) -> Option<Entity> {
        match self {
            Self::Index(_) => None,
            Self::Entity(entity) => Some(entity),
        }
    }
}

// Scene files contain stable node indices. Runtime entities are meaningful
// only in the World that spawned them and must never be persisted.
impl Serialize for SceneEntityRef {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Index(index) => index.serialize(serializer),
            Self::Entity(_) => Err(serde::ser::Error::custom(
                "a resolved scene entity cannot be serialized",
            )),
        }
    }
}

impl<'de> Deserialize<'de> for SceneEntityRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        usize::deserialize(deserializer).map(Self::Index)
    }
}

/// What a [`SceneComponent`] gets while a scene is being spawned.
pub struct SceneSpawnContext<'w> {
    world: &'w mut World,
    node_entities: &'w [Entity],
}

impl<'w> SceneSpawnContext<'w> {
    pub fn new(world: &'w mut World, node_entities: &'w [Entity]) -> Self {
        Self {
            world,
            node_entities,
        }
    }

    /// Adds a runtime component to any entity in the scene being spawned —
    /// not necessarily the one currently being applied to.
    pub fn insert<T: Component>(&mut self, component: T, entity: Entity) {
        self.world.insert(component, entity);
    }

    /// Resolves a node reference to its spawned entity. Returns `None` for an
    /// out-of-range index, so a malformed scene cannot panic.
    pub fn entity_for(&self, reference: SceneEntityRef) -> Option<Entity> {
        match reference {
            SceneEntityRef::Index(index) => self.node_entities.get(index).copied(),
            SceneEntityRef::Entity(entity) => Some(entity),
        }
    }

    /// Resolves a serialized node index in place. Returns `false` when the
    /// index is outside this scene's node table.
    pub fn resolve_entity(&self, reference: &mut SceneEntityRef) -> bool {
        let Some(entity) = self.entity_for(*reference) else {
            return false;
        };
        *reference = SceneEntityRef::Entity(entity);
        true
    }

    /// Reads a resource needed while materializing a scene component.
    pub fn get_resource<T: Resource>(&self) -> Option<&T> {
        self.world.get_resource::<T>()
    }
}

/// Data authored into a `Scene` that knows how to apply itself to a
/// spawned entity.
///
/// A type that is a runtime component inserts itself. A type that is really
/// authoring data expands into several runtime components — possibly on other
/// entities — and never inserts one of itself. Both are this one interface.
/// A component that can be written into a scene file and applied back out of
/// one. The `Serialize` half is also what lets tools read a live component's
/// value back out of the world; see [`World::component_types`].
pub trait SceneComponent: Component + Serialize + DeserializeOwned + Sized + 'static {
    fn apply(self, entity: Entity, ctx: &mut SceneSpawnContext<'_>);
}
