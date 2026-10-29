use std::{
    any::{Any, TypeId},
    collections::HashMap,
    fmt::Debug,
    marker::PhantomData,
    sync::Arc,
};

use super::{asset_server::AssetServer, Asset, AssetId, LoadableAsset};
use concerto_ecs::{
    component::scene::LiveEntityRefs,
    reflect::{AppTypeRegistry, ReflectSceneLeaf},
    Event, World,
};
use concerto_reflect::{
    decode_leaf, encode_leaf, FromType, GetTypeRegistration, JsonLeaf, Leaf, LeafError, LeafHook,
    Reflect, ReflectJson, ReflectMut, ReflectRef, TypeData, TypeRegistration, TypeRegistry,
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
        registration.insert(<ReflectJson as FromType<Self>>::from_type());
        registration.insert(<ReflectAssetHandle as FromType<Self>>::from_type());
        registration
    }
}

impl<A: Asset> JsonLeaf for AssetHandle<A> {
    /// The asset's id as 32 hex digits; reading one back gives a weak handle.
    fn to_json(&self) -> serde_json::Value {
        serde_json::Value::String(self.id().simple_hex())
    }

    fn from_json(value: &serde_json::Value) -> Result<Self, String> {
        value
            .as_str()
            .and_then(|id| AssetId::from_simple_hex(id).ok())
            .map(Self::weak)
            .ok_or_else(|| "expected an asset id of 32 hex digits".to_owned())
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

/// Writes values into a live world: entity references as [`LiveEntityRefs`] stores them, and asset handles loaded through the world's [`AssetServer`] as a scene spawns them.
pub struct LiveLeaves {
    server: Option<AssetServer>,
    handles: HashMap<TypeId, ReflectAssetHandle>,
}

impl LiveLeaves {
    /// Takes the handle types registered in `world` and its asset server, if it has one.
    pub fn new(world: &World) -> Self {
        let server = world.get_resource::<AssetServer>();
        match world.get_resource::<AppTypeRegistry>() {
            Some(registry) => Self::from_parts(&registry.read(), server),
            None => Self {
                server: server.cloned(),
                handles: HashMap::new(),
            },
        }
    }

    /// Takes the handle types registered in `registry` and the asset server to load them through, for a caller that reads a world's resources without holding the world.
    pub fn from_parts(registry: &TypeRegistry, server: Option<&AssetServer>) -> Self {
        let handles = registry
            .iter()
            .filter_map(|registration| {
                Some((
                    registration.type_id(),
                    *registration.data::<ReflectAssetHandle>()?,
                ))
            })
            .collect();
        Self {
            server: server.cloned(),
            handles,
        }
    }
}

impl LeafHook for LiveLeaves {
    fn encode(&mut self, leaf: &dyn Reflect) -> Option<Result<Vec<u8>, LeafError>> {
        LiveEntityRefs.encode(leaf)
    }

    fn decode(&mut self, leaf: &mut dyn Reflect, bytes: &[u8]) -> Option<Result<(), LeafError>> {
        if let Some(decoded) = LiveEntityRefs.decode(leaf, bytes) {
            return Some(decoded);
        }
        let handle = self.handles.get(&(&*leaf as &dyn Any).type_id())?;
        let server = self.server.as_ref()?;
        let mut id = AssetId::nil();
        if let Err(error) = decode_leaf(&mut id, bytes) {
            return Some(Err(error));
        }
        if id.is_nil() {
            return None;
        }
        Some(handle.set(leaf, server, id).map_err(|error| match error {
            SetHandleError::NotThisHandle => LeafError("not this asset's handle".into()),
            SetHandleError::AssetNotRegistered => LeafError(format!(
                "{} assets are not registered with the asset server",
                handle.kind()
            )),
        }))
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

    #[derive(serde::Serialize, serde::Deserialize)]
    struct Note;

    impl Asset for Note {
        fn name() -> &'static str {
            "Note"
        }
    }

    impl LoadableAsset for Note {}

    fn json_codec() -> concerto_reflect::ReflectJson {
        let mut registry = concerto_reflect::TypeRegistry::default();
        registry.register::<AssetHandle<Note>>();
        *registry
            .get_type_data::<concerto_reflect::ReflectJson>(
                std::any::TypeId::of::<AssetHandle<Note>>(),
            )
            .unwrap()
    }

    fn handle_from_json(value: &serde_json::Value) -> Result<AssetId, String> {
        let decoded = (json_codec().from_json)(value)?;
        let handle = (decoded.as_ref() as &dyn std::any::Any)
            .downcast_ref::<AssetHandle<Note>>()
            .unwrap();
        assert!(matches!(handle, AssetHandle::Weak(..)));
        Ok(handle.id())
    }

    #[test]
    fn a_handle_is_its_asset_id_as_32_hex_digits_in_json() {
        let id = AssetId::new();
        let handle = AssetHandle::<Note>::weak(id);
        let json = serde_json::Value::String(id.simple_hex());

        assert_eq!((json_codec().to_json)(&handle), json);
        assert_eq!(handle_from_json(&json), Ok(id));
    }

    #[test]
    fn a_handle_does_not_decode_from_anything_but_an_asset_id() {
        for value in [serde_json::json!("not an id"), serde_json::json!(7)] {
            assert!(handle_from_json(&value).is_err(), "{value} must not decode");
        }
    }

    fn handle_registry() -> concerto_reflect::TypeRegistry {
        let mut registry = concerto_reflect::TypeRegistry::default();
        registry.register::<AssetHandle<Note>>();
        registry
    }

    #[test]
    fn a_handle_written_back_from_its_json_names_the_same_asset() {
        let registry = handle_registry();
        let id = AssetId::new();
        let json = concerto_reflect::to_json(&AssetHandle::<Note>::weak(id), &registry);

        let stored = concerto_reflect::json_to_value(
            &json,
            std::any::TypeId::of::<AssetHandle<Note>>(),
            &registry,
        )
        .unwrap();
        let mut loaded = AssetHandle::<Note>::default();
        assert!(concerto_reflect::apply_value(&mut loaded, &stored).is_empty());
        assert_eq!(loaded.id(), id);
    }

    #[test]
    fn a_handle_written_from_anything_but_an_asset_id_says_what_it_expected() {
        assert_eq!(
            concerto_reflect::json_to_value(
                &serde_json::json!(7),
                std::any::TypeId::of::<AssetHandle<Note>>(),
                &handle_registry(),
            ),
            Err("expected an asset id of 32 hex digits".to_owned())
        );
    }
}
