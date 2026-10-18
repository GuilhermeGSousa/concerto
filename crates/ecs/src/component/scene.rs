use concerto_reflect::{
    FromType, GetTypeRegistration, Leaf, LeafError, Reflect, ReflectMut, ReflectRef,
    TypeRegistration, decode_leaf, encode_leaf,
};

use crate::{entity::Entity, reflect::ReflectSceneLeaf};

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

impl Default for SceneEntityRef {
    /// An unresolved reference: it names no node, so [`entity`](Self::entity) is `None`.
    fn default() -> Self {
        Self::Index(usize::MAX)
    }
}

impl Leaf for SceneEntityRef {
    fn encode(&self) -> Result<Vec<u8>, LeafError> {
        match self {
            Self::Index(index) => encode_leaf(&(*index as u64)),
            Self::Entity(_) => Err(LeafError(
                "a resolved scene entity can only be stored while its scene is captured".into(),
            )),
        }
    }

    fn decode(&mut self, bytes: &[u8]) -> Result<(), LeafError> {
        let mut index = 0_u64;
        decode_leaf(&mut index, bytes)?;
        *self = Self::Index(index as usize);
        Ok(())
    }
}

impl Reflect for SceneEntityRef {
    fn reflect_ref(&self) -> ReflectRef<'_> {
        ReflectRef::Leaf(self)
    }

    fn reflect_mut(&mut self) -> ReflectMut<'_> {
        ReflectMut::Leaf(self)
    }
}

impl GetTypeRegistration for SceneEntityRef {
    fn get_type_registration() -> TypeRegistration {
        let mut registration = TypeRegistration::of::<Self>();
        registration.insert(<ReflectSceneLeaf as FromType<Self>>::from_type());
        registration
    }
}

impl FromType<SceneEntityRef> for ReflectSceneLeaf {
    fn from_type() -> Self {
        Self {
            spawn: |leaf, bytes, spawn| {
                let reference = (leaf as &mut dyn std::any::Any)
                    .downcast_mut::<SceneEntityRef>()
                    .ok_or_else(|| LeafError("not a SceneEntityRef".into()))?;
                let mut index = 0_u64;
                decode_leaf(&mut index, bytes)?;
                let entity = spawn
                    .node_entities
                    .get(index as usize)
                    .copied()
                    .ok_or_else(|| {
                        LeafError(format!("node {index} is outside the scene being spawned"))
                    })?;
                *reference = SceneEntityRef::Entity(entity);
                Ok(())
            },
            capture: |leaf, capture| {
                let reference = (leaf as &dyn std::any::Any)
                    .downcast_ref::<SceneEntityRef>()
                    .ok_or_else(|| LeafError("not a SceneEntityRef".into()))?;
                let index = match *reference {
                    SceneEntityRef::Index(index) => index,
                    SceneEntityRef::Entity(entity) => {
                        capture.index_of(entity).ok_or_else(|| {
                            LeafError(format!(
                                "references {entity:?}, which is outside the scene being captured"
                            ))
                        })?
                    }
                };
                encode_leaf(&(index as u64))
            },
        }
    }
}
