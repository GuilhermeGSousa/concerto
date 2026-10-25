use std::{fmt::Debug, marker::PhantomData, sync::Arc};

use super::{asset_server::AssetServer, Asset, AssetId, LoadableAsset};
use concerto_ecs::{reflect::ReflectSceneLeaf, Event};
use concerto_reflect::{
    decode_leaf, encode_leaf, FromType, GetTypeRegistration, Leaf, LeafError, Reflect, ReflectMut,
    ReflectRef, TypeData, TypeRegistration,
};
use crossbeam_channel::Sender;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Clone, Event)]
pub enum AssetLifetimeEvent {
    Dropped(AssetId),
}

impl AssetLifetimeEvent {
    pub fn id(&self) -> AssetId {
        match self {
            AssetLifetimeEvent::Dropped(id) => *id,
        }
    }
}

pub struct StrongAssetHandle {
    pub(crate) id: AssetId,
    pub(crate) lifetime_sender: Sender<AssetLifetimeEvent>,
}

impl Drop for StrongAssetHandle {
    fn drop(&mut self) {
        let _ = self
            .lifetime_sender
            .send(AssetLifetimeEvent::Dropped(self.id));
    }
}

impl Debug for StrongAssetHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StrongAssetHandle")
            .field("id", &self.id)
            .finish()
    }
}

/// A handle to an asset of type `A`.
///
/// `Strong` keeps the underlying asset alive (via the `Arc<StrongAssetHandle>`,
/// whose `Drop` impl reports the asset as no longer referenced); `Weak` is
/// just an `AssetId` reference that does not keep anything alive and does not
/// resolve to a loaded asset on its own. This is unrelated to
/// `std::sync::Weak<StrongAssetHandle>` used internally by
/// `AssetHandleProvider` for handle-reuse/dedup (which is about not keeping
/// the asset alive from the provider's cache, not about this enum's variant).
///
/// `AssetHandle` serializes to (and deserializes from) just its bare
/// `AssetId`: deserializing always produces a `Weak` handle, since
/// deserialization has no `AssetServer` to resolve a `Strong` handle against.
pub enum AssetHandle<A: Asset> {
    Strong(Arc<StrongAssetHandle>, PhantomData<A>),
    Weak(AssetId, PhantomData<A>),
}

impl<A: Asset> AssetHandle<A> {
    pub(crate) fn strong(handle: Arc<StrongAssetHandle>) -> Self {
        AssetHandle::Strong(handle, PhantomData)
    }

    pub fn weak(id: AssetId) -> Self {
        AssetHandle::Weak(id, PhantomData)
    }

    pub fn id(&self) -> AssetId {
        match self {
            AssetHandle::Strong(handle, _) => handle.id,
            AssetHandle::Weak(id, _) => *id,
        }
    }
}

impl<A: Asset> Clone for AssetHandle<A> {
    fn clone(&self) -> Self {
        match self {
            AssetHandle::Strong(handle, _) => AssetHandle::Strong(handle.clone(), PhantomData),
            AssetHandle::Weak(id, _) => AssetHandle::Weak(*id, PhantomData),
        }
    }
}

impl<A: Asset> Debug for AssetHandle<A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AssetHandle::Strong(_, _) => write!(f, "AssetHandle::Strong({:?})", self.id()),
            AssetHandle::Weak(id, _) => write!(f, "AssetHandle::Weak({id:?})"),
        }
    }
}

impl<A: Asset> Serialize for AssetHandle<A> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.id().serialize(serializer)
    }
}

impl<'de, A: Asset> Deserialize<'de> for AssetHandle<A> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let id = AssetId::deserialize(deserializer)?;
        Ok(AssetHandle::weak(id))
    }
}

impl<A: Asset> Default for AssetHandle<A> {
    /// The empty handle: weak, naming no asset.
    fn default() -> Self {
        Self::weak(AssetId::nil())
    }
}

impl<A: Asset> PartialEq for AssetHandle<A> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Strong(l0, l1), Self::Strong(r0, r1)) => l0.id == r0.id && l1 == r1,
            (Self::Weak(l0, l1), Self::Weak(r0, r1)) => l0 == r0 && l1 == r1,
            _ => false,
        }
    }
}

impl<A: Asset> Leaf for AssetHandle<A> {
    fn encode(&self) -> Result<Vec<u8>, LeafError> {
        encode_leaf(&self.id())
    }

    fn decode(&mut self, bytes: &[u8]) -> Result<(), LeafError> {
        let mut id = AssetId::nil();
        decode_leaf(&mut id, bytes)?;
        if !id.is_nil() {
            log::warn!(
                "{} was read without its scene type data, so it stays weak and loads nothing; \
                 register it with App::register_type",
                std::any::type_name::<Self>()
            );
        }
        *self = Self::weak(id);
        Ok(())
    }
}

impl<A: Asset> Reflect for AssetHandle<A> {
    fn reflect_ref(&self) -> ReflectRef<'_> {
        ReflectRef::Leaf(self)
    }

    fn reflect_mut(&mut self) -> ReflectMut<'_> {
        ReflectMut::Leaf(self)
    }
}

impl<A: LoadableAsset> GetTypeRegistration for AssetHandle<A> {
    fn get_type_registration() -> TypeRegistration {
        let mut registration = TypeRegistration::of::<Self>();
        registration.insert(<ReflectSceneLeaf as FromType<Self>>::from_type());
        registration.insert(<ReflectAssetHandle as FromType<Self>>::from_type());
        registration
    }
}

/// Why [`ReflectAssetHandle::set`] left a value unchanged.
#[derive(Debug, PartialEq, Eq)]
pub enum SetHandleError {
    NotThisHandle,
    AssetNotRegistered,
}

/// An `AssetHandle<A>` described without naming `A`.
#[derive(Clone, Copy)]
pub struct ReflectAssetHandle {
    kind: &'static str,
    id: fn(&dyn Reflect) -> Option<AssetId>,
    set: fn(&mut dyn Reflect, &AssetServer, AssetId) -> Result<(), SetHandleError>,
}

impl TypeData for ReflectAssetHandle {}

impl ReflectAssetHandle {
    /// The handled asset's kind, as [`Asset::name`] gives it.
    pub fn kind(&self) -> &'static str {
        self.kind
    }

    /// The id `value` holds, or `None` if it is not this handle type.
    pub fn id(&self, value: &dyn Reflect) -> Option<AssetId> {
        (self.id)(value)
    }

    /// Writes into `value` a handle to `id` loaded through `server`.
    pub fn set(
        &self,
        value: &mut dyn Reflect,
        server: &AssetServer,
        id: AssetId,
    ) -> Result<(), SetHandleError> {
        (self.set)(value, server, id)
    }
}

impl<A: LoadableAsset> FromType<AssetHandle<A>> for ReflectAssetHandle {
    fn from_type() -> Self {
        Self {
            kind: A::name(),
            id: |value| {
                (value as &dyn std::any::Any)
                    .downcast_ref::<AssetHandle<A>>()
                    .map(AssetHandle::id)
            },
            set: |value, server, id| {
                let handle = (value as &mut dyn std::any::Any)
                    .downcast_mut::<AssetHandle<A>>()
                    .ok_or(SetHandleError::NotThisHandle)?;
                if !server.is_registered::<A>() {
                    return Err(SetHandleError::AssetNotRegistered);
                }
                *handle = server.load(id);
                Ok(())
            },
        }
    }
}

impl<A: LoadableAsset> FromType<AssetHandle<A>> for ReflectSceneLeaf {
    fn from_type() -> Self {
        Self {
            spawn: |leaf, bytes, spawn| {
                let handle = (leaf as &mut dyn std::any::Any)
                    .downcast_mut::<AssetHandle<A>>()
                    .ok_or_else(|| LeafError("not this asset's handle".into()))?;
                let mut id = AssetId::nil();
                decode_leaf(&mut id, bytes)?;
                *handle = match spawn.world.get_resource::<AssetServer>() {
                    Some(server) if !id.is_nil() => server.load::<A>(id),
                    _ => AssetHandle::weak(id),
                };
                Ok(())
            },
            capture: |leaf, capture| {
                let handle = (leaf as &dyn std::any::Any)
                    .downcast_ref::<AssetHandle<A>>()
                    .ok_or_else(|| LeafError("not this asset's handle".into()))?;
                capture.reference(handle.id());
                encode_leaf(&handle.id())
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn final_strong_reference_emits_one_uuid_drop_event() {
        let (sender, receiver) = crossbeam_channel::unbounded();
        let id = AssetId::new();
        let handle = Arc::new(StrongAssetHandle {
            id,
            lifetime_sender: sender,
        });
        let clone = handle.clone();
        drop(handle);
        assert!(receiver.is_empty());

        drop(clone);
        let AssetLifetimeEvent::Dropped(dropped_id) = receiver.try_recv().unwrap();
        assert_eq!(dropped_id, id);
        assert!(receiver.is_empty());
    }
}
