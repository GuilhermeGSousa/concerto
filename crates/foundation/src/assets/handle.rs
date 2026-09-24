use std::{fmt::Debug, marker::PhantomData, sync::Arc};

use super::{Asset, AssetId};
use concerto_ecs::Event;
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
