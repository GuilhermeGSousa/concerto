use std::{
    any::TypeId,
    collections::{HashMap, HashSet},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, RwLock, Weak,
    },
};

use anyhow::Context;
use concerto_ecs::{
    query::change_detection::DetectChanges,
    resource::{Res, Resource},
    world,
};
use concerto_tasks::load_pool::LoadTaskPool;
use crossbeam_channel::{Receiver, Sender};

use crate::{
    assets::{handle::StrongAssetHandle, LoadableAsset},
    tasks::{task_pool::TaskPool, Task},
};

use super::{
    asset_container::AssetContainer,
    asset_store::AssetStore,
    handle::{AssetHandle, AssetLifetimeEvent},
    registry::{AssetEntry, AssetRegistry},
    Asset, AssetId, ContentAssetRoot,
};

struct LoadedAsset {
    pub(crate) id: AssetId,
    pub(crate) value: Box<dyn AssetContainer>,
}

impl LoadedAsset {
    pub fn new<A: Asset + 'static>(id: AssetId, value: A) -> Self {
        LoadedAsset {
            id,
            value: Box::new(value),
        }
    }
}

enum AssetLoadEvent {
    Loaded { asset: LoadedAsset },
    LoadFailed { id: AssetId },
}

impl AssetLoadEvent {
    fn id(&self) -> AssetId {
        match self {
            AssetLoadEvent::Loaded { asset } => asset.id,
            AssetLoadEvent::LoadFailed { id } => *id,
        }
    }
}

pub struct AssetLoadContext {
    asset_server: AssetServer,
    asset_id: AssetId,
}

impl AssetLoadContext {
    pub fn asset_server(&self) -> &AssetServer {
        &self.asset_server
    }

    pub fn asset_id(&self) -> AssetId {
        self.asset_id
    }

    pub(crate) fn new(asset_server: AssetServer, asset_id: AssetId) -> Self {
        Self {
            asset_server,
            asset_id,
        }
    }
}

pub(crate) struct AssetInfo {
    handle: Weak<StrongAssetHandle>,
}

struct PendingLoad {
    _task: Task<()>,
    cancelled: Arc<AtomicBool>,
}

pub(crate) struct AssetServerData {
    pending_tasks: RwLock<HashMap<AssetId, PendingLoad>>,
    loaded_assets: RwLock<HashSet<AssetId>>,
    handle_provider: AssetHandleProvider,
    asset_load_event_sender: Sender<AssetLoadEvent>,
    asset_load_event_receiver: Receiver<AssetLoadEvent>,
    content: RwLock<ContentState>,
}

struct ContentState {
    root: ContentAssetRoot,
    registry: Arc<AssetRegistry>,
}

#[derive(Resource, Clone)]
pub struct AssetServer {
    data: Arc<AssetServerData>,
}

impl AssetServer {
    pub fn new() -> Self {
        Self::with_content_root(ContentAssetRoot::default_for_platform())
    }

    /// Creates a server with the supplied initial content root.
    pub fn with_content_root(root: ContentAssetRoot) -> Self {
        let (asset_load_event_sender, asset_load_event_receiver) = crossbeam_channel::unbounded();
        let server_data = AssetServerData {
            pending_tasks: RwLock::new(HashMap::new()),
            loaded_assets: RwLock::new(HashSet::new()),
            handle_provider: AssetHandleProvider::new(),
            asset_load_event_sender,
            asset_load_event_receiver,
            content: RwLock::new(ContentState {
                root,
                registry: Arc::default(),
            }),
        };

        Self {
            data: Arc::new(server_data),
        }
    }

    /// Configures the editor project root and its complete registry snapshot.
    pub fn publish_project_content(
        &self,
        project_root: &Path,
        registry: AssetRegistry,
    ) -> anyhow::Result<()> {
        let root = project_root.canonicalize().with_context(|| {
            format!(
                "failed to resolve project root '{}'",
                project_root.display()
            )
        })?;
        self.publish_content_source(ContentAssetRoot::Directory(root), registry);
        Ok(())
    }

    /// Atomically replaces the root and UUID registry used by subsequent
    /// loads. This is also available for non-native content providers.
    pub fn publish_content_source(&self, root: ContentAssetRoot, registry: AssetRegistry) {
        // `load` takes these locks in the same order. Holding the pending lock
        // closes the gap where a request could otherwise attach to the old
        // snapshot while it is being replaced.
        let mut pending = self.data.pending_tasks.write().unwrap();
        {
            let mut content = self.data.content.write().unwrap();
            content.root = root;
            content.registry = Arc::new(registry);
        };
        pending.clear();
    }

    /// Replaces the registry subsequent loads resolve through, keeping the root and pending loads.
    pub fn publish_registry(&self, registry: AssetRegistry) {
        self.data.content.write().unwrap().registry = Arc::new(registry);
    }

    /// The registry loads resolve through, empty until one is published.
    pub fn registry(&self) -> Option<Arc<AssetRegistry>> {
        Some(Arc::clone(&self.data.content.read().unwrap().registry))
    }

    /// Whether an asset requested with `load` or `add` has yet to reach the world; a failed load counts until its failure is handled.
    pub fn is_loading(&self) -> bool {
        !self.data.pending_tasks.read().unwrap().is_empty()
            || !self.data.asset_load_event_receiver.is_empty()
    }

    /// Cancels the loads pending at the call; none of them delivers its asset afterwards.
    pub fn cancel_pending(&self) {
        let mut pending = self.data.pending_tasks.write().unwrap();
        for load in pending.values() {
            load.cancelled.store(true, Ordering::SeqCst);
        }
        let kept: Vec<_> = self
            .data
            .asset_load_event_receiver
            .try_iter()
            .filter(|event| !pending.contains_key(&event.id()))
            .collect();
        for event in kept {
            let _ = self.data.asset_load_event_sender.send(event);
        }
        pending.clear();
    }

    pub fn register_asset<A: Asset>(&mut self, asset: &AssetStore<A>) {
        self.data
            .handle_provider
            .register_asset::<A>(asset.clone_drop_sender());
    }

    /// Whether `A` was registered, without which `load` and `add` panic.
    pub fn is_registered<A: Asset>(&self) -> bool {
        self.data.handle_provider.is_registered::<A>()
    }

    pub fn add<A: Asset>(&self, asset: A) -> AssetHandle<A> {
        let id = AssetId::new();

        let sender = self.data.asset_load_event_sender.clone();
        let _ = sender.send(AssetLoadEvent::Loaded {
            asset: LoadedAsset::new(id, asset),
        });
        self.data.handle_provider.request_handle(id)
    }

    /// Loads a stored asset using its persistent UUID, obtained from `asset_id!`
    /// or a serialized asset reference. Repeated requests share handles and loads.
    pub fn load<A: LoadableAsset + 'static>(&self, id: AssetId) -> AssetHandle<A> {
        // Hold this lock through task insertion so concurrent requests cannot
        // spawn duplicate loads for the same persistent UUID.
        let mut pending = self.data.pending_tasks.write().unwrap();
        if !pending.contains_key(&id) && !self.data.loaded_assets.read().unwrap().contains(&id) {
            let cancelled = Arc::new(AtomicBool::new(false));
            let task = self.request_load::<A>(id, Arc::clone(&cancelled));
            pending.insert(
                id,
                PendingLoad {
                    _task: task,
                    cancelled,
                },
            );
        }
        self.data.handle_provider.request_handle(id)
    }

    pub(crate) fn process_handle_drop(&mut self, id: &AssetId) {
        self.data.loaded_assets.write().unwrap().remove(id);
    }

    #[cfg(test)]
    fn resolve_by_id(&self, id: AssetId) -> Option<String> {
        self.resolve_asset(id).map(|(_, entry)| entry.address)
    }

    fn resolve_asset(&self, id: AssetId) -> Option<(ContentAssetRoot, AssetEntry)> {
        let content = self.data.content.read().unwrap();
        let entry = content.registry.get(id)?.clone();
        Some((content.root.clone(), entry))
    }

    /// Resolves the UUID through the registry before reading the cooked asset.
    /// A registry miss fails the load without attempting file I/O.
    fn request_load<A: LoadableAsset>(&self, id: AssetId, cancelled: Arc<AtomicBool>) -> Task<()> {
        let data = Arc::clone(&self.data);
        let deliver = move |event| {
            let _pending = data.pending_tasks.read().unwrap();
            if !cancelled.load(Ordering::SeqCst) {
                let _ = data.asset_load_event_sender.send(event);
            }
        };

        let server = self.clone();
        // No profiling scope around the async body: a scope guard must not be
        // held across .await (tasks can migrate between worker threads).
        // Load costs show up on the named "asset-load-N" threads instead.
        LoadTaskPool::get_or_init(|| TaskPool::with_name("asset-load")).spawn(async move {
            let (content_root, entry) = match server.resolve_asset(id) {
                Some(resolved) => resolved,
                None => {
                    log::error!(
                        "no content asset registered for AssetId {id:?} (type {})",
                        std::any::type_name::<A>()
                    );
                    deliver(AssetLoadEvent::LoadFailed { id });
                    return;
                }
            };
            if !entry.is::<A>() {
                log::error!(
                    "content asset '{}' holds a {} but a {} was requested",
                    entry.address,
                    entry.kind,
                    A::name()
                );
                deliver(AssetLoadEvent::LoadFailed { id });
                return;
            }
            let address = entry.address;
            let asset = async {
                let bytes = crate::assets::utils::load_content_asset_bytes(
                    &content_root,
                    &address,
                    A::name(),
                )
                .await
                .with_context(|| format!("failed to read {} asset", A::name()))?;
                let mut asset: A = bincode::deserialize(&bytes)
                    .with_context(|| format!("failed to deserialize {} asset", A::name()))?;
                let context = AssetLoadContext::new(server, id);
                asset.on_load(&context)?;
                anyhow::Ok(asset)
            }
            .await;
            match asset {
                Ok(asset) => {
                    deliver(AssetLoadEvent::Loaded {
                        asset: LoadedAsset::new(id, asset),
                    });
                }
                Err(error) => {
                    log::error!(
                        "Failed to load asset '{}' (type {}): {:#}",
                        address,
                        std::any::type_name::<A>(),
                        error
                    );
                    deliver(AssetLoadEvent::LoadFailed { id });
                }
            }
        })
    }
}

/// Hands the server the world's [`AssetRegistry`] whenever it changes.
pub fn sync_asset_registry(registry: Option<Res<AssetRegistry>>, server: Res<AssetServer>) {
    if let Some(registry) = registry.filter(|registry| registry.is_changed()) {
        server.publish_registry(registry.clone());
    }
}

impl Default for AssetServer {
    fn default() -> Self {
        Self::new()
    }
}

// TODO: This shouldn't need to be public
pub fn handle_asset_load_events(world: &mut world::World) {
    let server = world.remove_resource::<AssetServer>().unwrap();

    server
        .data
        .asset_load_event_receiver
        .try_iter()
        .for_each(|event| match event {
            AssetLoadEvent::Loaded {
                asset: loaded_asset,
            } => {
                server
                    .data
                    .pending_tasks
                    .write()
                    .unwrap()
                    .remove(&loaded_asset.id);
                server
                    .data
                    .loaded_assets
                    .write()
                    .unwrap()
                    .insert(loaded_asset.id);
                loaded_asset.value.insert(loaded_asset.id, world);
            }
            AssetLoadEvent::LoadFailed { id } => {
                server.data.pending_tasks.write().unwrap().remove(&id);
                server.data.loaded_assets.write().unwrap().remove(&id);
            }
        });
    world.insert_resource(server);
}

struct AssetHandleProvider {
    asset_handles: RwLock<HashMap<AssetId, AssetInfo>>,
    asset_lifetime_send_map: RwLock<HashMap<TypeId, Sender<AssetLifetimeEvent>>>,
}

impl AssetHandleProvider {
    pub fn new() -> Self {
        Self {
            asset_handles: RwLock::new(HashMap::new()),
            asset_lifetime_send_map: RwLock::new(HashMap::new()),
        }
    }

    pub fn register_asset<A: Asset>(&self, lifetime_sender: Sender<AssetLifetimeEvent>) {
        let type_id = TypeId::of::<A>();
        self.asset_lifetime_send_map
            .write()
            .unwrap()
            .insert(type_id, lifetime_sender);
    }

    fn is_registered<A: Asset>(&self) -> bool {
        self.asset_lifetime_send_map
            .read()
            .unwrap()
            .contains_key(&TypeId::of::<A>())
    }

    pub fn request_handle<A: Asset>(&self, id: AssetId) -> AssetHandle<A> {
        let lifetime_sender = self
            .asset_lifetime_send_map
            .read()
            .unwrap()
            .get(&TypeId::of::<A>())
            .unwrap_or_else(|| {
                panic!(
                    "Asset lifetime sender not found for {}, make sure to register it",
                    A::name()
                )
            })
            .clone();

        let mut binding = self.asset_handles.write().unwrap();

        let info = binding.entry(id).or_insert_with(|| AssetInfo {
            handle: Weak::new(),
        });

        if let Some(strong_handle) = info.handle.upgrade() {
            AssetHandle::strong(strong_handle)
        } else {
            let handle = Arc::new(StrongAssetHandle {
                id,
                lifetime_sender,
            });

            info.handle = Arc::downgrade(&handle);

            AssetHandle::strong(handle)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::handle::ReflectAssetHandle;
    use crate::assets::{
        asset_store::AssetStore,
        content::save_content_asset_into,
        registry::{AssetEntry, AssetRegistry},
    };
    use concerto_ecs::{component::Component, reflect::ReflectSceneComponent, world::World};

    fn temp_root(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("asset-server-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("content")).unwrap();
        dir
    }

    /// A minimal real asset: written to disk by `save_content_asset_into` and read
    /// back through the cooked-asset path, so UUID loading is exercised end to
    /// end rather than against a mock.
    #[derive(serde::Serialize, serde::Deserialize)]
    struct FixtureAsset {
        value: u32,
        #[serde(skip)]
        initialized: bool,
    }

    impl Asset for FixtureAsset {
        fn name() -> &'static str {
            "FixtureAsset"
        }
    }

    impl LoadableAsset for FixtureAsset {
        fn on_load(&mut self, _context: &AssetLoadContext) -> anyhow::Result<()> {
            self.initialized = true;
            Ok(())
        }
    }

    fn scene_entry(id: AssetId) -> AssetEntry {
        AssetEntry {
            id,
            address: "content/hero/scene.gasset".into(),
            kind: "Scene".into(),
            provenance: None,
        }
    }

    /// Runs the load task spawned for `id` to completion.
    ///
    /// Taking the real `Task` out of `pending_tasks` and awaiting it makes the
    /// wait deterministic — the task is a `Future`, so there is no sleeping or
    /// polling — while still running the genuine `LoadTaskPool` task, registry
    /// read and deserialization.
    fn drive_pending_load(server: &AssetServer, id: AssetId) {
        let task = server
            .data
            .pending_tasks
            .write()
            .unwrap()
            .remove(&id)
            .expect("load spawns a task for an id that isn't loaded yet")
            ._task;
        pollster::block_on(task);
    }

    #[test]
    fn resolve_by_id_finds_a_registered_asset() {
        let dir = temp_root("resolve-hit");
        let id = AssetId::from_path("content/hero/scene.gasset");
        let mut registry = AssetRegistry::new();
        registry.insert(scene_entry(id));

        let server = AssetServer::with_content_root(ContentAssetRoot::Directory(dir.clone()));
        server.publish_registry(registry);
        let resolved = server.resolve_by_id(id);

        assert_eq!(resolved.as_deref(), Some("content/hero/scene.gasset"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resolve_by_id_returns_none_for_an_unregistered_id() {
        let dir = temp_root("resolve-miss");
        let server = AssetServer::with_content_root(ContentAssetRoot::Directory(dir.clone()));
        let resolved = server.resolve_by_id(AssetId::new());
        assert_eq!(resolved, None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_reads_a_registered_content_asset() {
        let dir = temp_root("load-by-id-hit");
        let address = "content/fixture/value.gasset";
        let mut registry = AssetRegistry::new();
        let id = save_content_asset_into(
            &FixtureAsset {
                value: 7,
                initialized: false,
            },
            &dir,
            address,
            &mut registry,
        )
        .expect("save content asset");

        let mut world = World::new();
        let store = AssetStore::<FixtureAsset>::new();
        let mut server = AssetServer::with_content_root(ContentAssetRoot::Directory(dir.clone()));
        server.register_asset::<FixtureAsset>(&store);
        server.publish_registry(registry);
        world.insert_resource(store);
        world.insert_resource(server.clone());

        let handle = server.load::<FixtureAsset>(id);
        drive_pending_load(&server, id);
        handle_asset_load_events(&mut world);

        assert!(
            server.data.loaded_assets.read().unwrap().contains(&id),
            "a registered id must resolve through the registry and finish loading"
        );
        let store = world
            .get_resource::<AssetStore<FixtureAsset>>()
            .expect("asset store");
        assert_eq!(store.get(&handle).map(|asset| asset.value), Some(7));
        assert!(store.get(&handle).unwrap().initialized);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[derive(Component, concerto_reflect::Reflect, Default)]
    #[reflect(SceneComponent)]
    struct Holder {
        handle: AssetHandle<FixtureAsset>,
        empty: AssetHandle<FixtureAsset>,
    }

    #[test]
    fn a_scene_handle_spawns_strong_through_the_server_and_capture_records_its_id() {
        use concerto_ecs::reflect::SceneCapture;

        let dir = temp_root("scene-handle");
        let store = AssetStore::<FixtureAsset>::new();
        let mut server = AssetServer::with_content_root(ContentAssetRoot::Directory(dir.clone()));
        server.register_asset::<FixtureAsset>(&store);
        let mut world = World::new();
        world.insert_resource(server);
        world.register_type::<Holder>();
        let id = AssetId::new();
        let stored = concerto_reflect::to_value(&Holder {
            handle: AssetHandle::weak(id),
            empty: AssetHandle::default(),
        })
        .unwrap();
        let entity = world.spawn(());

        assert!(world.apply_scene_component("Holder", &stored, entity, &[entity]));

        let holder = world.get_component_for_entity::<Holder>(entity).unwrap();
        assert!(matches!(holder.handle, AssetHandle::Strong(..)));
        assert_eq!(holder.handle.id(), id);
        assert!(matches!(holder.empty, AssetHandle::Weak(..)));
        assert!(holder.empty.id().is_nil());

        let mut capture = SceneCapture::new(HashMap::from([(entity, 0)]));
        let captured = world
            .component_types(entity)
            .next()
            .unwrap()
            .capture(&world, entity, &mut capture)
            .unwrap();
        assert_eq!(captured, stored);
        assert_eq!(capture.referenced_assets(), [id]);

        std::fs::remove_dir_all(&dir).ok();
    }

    fn handle_type_data(world: &World) -> ReflectAssetHandle {
        world
            .get_resource::<concerto_ecs::reflect::AppTypeRegistry>()
            .unwrap()
            .get_type_data::<ReflectAssetHandle>(std::any::TypeId::of::<AssetHandle<FixtureAsset>>())
            .unwrap()
    }

    #[test]
    fn a_handle_registered_through_its_component_describes_itself_without_naming_its_asset() {
        let mut world = World::new();
        world.register_type::<Holder>();
        let handle = handle_type_data(&world);
        let id = AssetId::new();

        assert_eq!(handle.kind(), "FixtureAsset");
        assert_eq!(handle.id(&AssetHandle::<FixtureAsset>::weak(id)), Some(id));
        assert_eq!(handle.id(&7u32), None);
    }

    #[test]
    fn a_described_handle_loads_only_through_a_server_that_registered_its_asset() {
        let mut world = World::new();
        world.register_type::<Holder>();
        let handle = handle_type_data(&world);
        let dir = temp_root("described-handle");
        let id = AssetId::new();
        let mut value = AssetHandle::<FixtureAsset>::default();

        let bare = AssetServer::with_content_root(ContentAssetRoot::Directory(dir.clone()));
        assert!(handle.set(&mut value, &bare, id).is_err());
        assert!(value.id().is_nil());

        let store = AssetStore::<FixtureAsset>::new();
        let mut server = AssetServer::with_content_root(ContentAssetRoot::Directory(dir.clone()));
        server.register_asset::<FixtureAsset>(&store);
        assert!(handle.set(&mut 7u32, &server, id).is_err());
        handle.set(&mut value, &server, id).unwrap();
        assert!(matches!(value, AssetHandle::Strong(..)));
        assert_eq!(value.id(), id);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_handle_written_through_live_leaves_loads_through_the_server() {
        use crate::assets::handle::LiveLeaves;
        use concerto_ecs::reflect::AppTypeRegistry;
        use std::any::TypeId;

        let dir = temp_root("live-handle");
        let store = AssetStore::<FixtureAsset>::new();
        let mut server = AssetServer::with_content_root(ContentAssetRoot::Directory(dir.clone()));
        server.register_asset::<FixtureAsset>(&store);
        let mut world = World::new();
        world.insert_resource(server.clone());
        world.register_type::<AssetHandle<FixtureAsset>>();
        let id = AssetId::new();
        let mut hook = LiveLeaves::new(&world);
        let stored = concerto_reflect::json_to_value_with(
            &serde_json::Value::String(id.simple_hex()),
            TypeId::of::<AssetHandle<FixtureAsset>>(),
            &world.get_resource::<AppTypeRegistry>().unwrap().read(),
            &mut hook,
        )
        .unwrap();

        let mut handle = AssetHandle::<FixtureAsset>::default();
        let warnings = concerto_reflect::apply_value_with(&mut handle, &stored, &mut hook);

        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(matches!(handle, AssetHandle::Strong(..)));
        assert_eq!(handle.id(), id);
        assert!(server.data.pending_tasks.read().unwrap().contains_key(&id));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn live_leaves_keep_entity_references_live_and_handles_weak_without_a_server() {
        use crate::assets::handle::LiveLeaves;
        use concerto_ecs::{component::scene::SceneEntityRef, reflect::AppTypeRegistry};
        use std::any::TypeId;

        let mut world = World::new();
        world.register_type::<AssetHandle<FixtureAsset>>();
        let entity = world.spawn(());
        let id = AssetId::new();
        let mut hook = LiveLeaves::new(&world);
        let registry = world.get_resource::<AppTypeRegistry>().unwrap().read();
        let reference = concerto_reflect::json_to_value_with(
            &serde_json::Value::String(entity.to_id_string()),
            TypeId::of::<SceneEntityRef>(),
            &registry,
            &mut hook,
        )
        .unwrap();
        let asset = concerto_reflect::json_to_value_with(
            &serde_json::Value::String(id.simple_hex()),
            TypeId::of::<AssetHandle<FixtureAsset>>(),
            &registry,
            &mut hook,
        )
        .unwrap();

        let mut written = SceneEntityRef::default();
        let mut handle = AssetHandle::<FixtureAsset>::default();
        assert!(concerto_reflect::apply_value_with(&mut written, &reference, &mut hook).is_empty());
        assert!(concerto_reflect::apply_value_with(&mut handle, &asset, &mut hook).is_empty());

        assert_eq!(written, SceneEntityRef::Entity(entity));
        assert!(matches!(handle, AssetHandle::Weak(..)));
        assert_eq!(handle.id(), id);
    }

    #[test]
    fn load_fails_for_an_unregistered_id() {
        let dir = temp_root("load-by-id-miss");
        let store = AssetStore::<FixtureAsset>::new();
        let mut server = AssetServer::with_content_root(ContentAssetRoot::Directory(dir.clone()));
        server.register_asset::<FixtureAsset>(&store);

        let id = AssetId::new();
        let _handle = server.load::<FixtureAsset>(id);
        drive_pending_load(&server, id);

        match server.data.asset_load_event_receiver.try_recv() {
            Ok(AssetLoadEvent::LoadFailed { id: failed, .. }) => assert_eq!(failed, id),
            Ok(AssetLoadEvent::Loaded { .. }) => {
                panic!("an unregistered id must not produce an asset")
            }
            Err(error) => panic!("expected a LoadFailed event, got {error}"),
        }
        assert!(!server.data.loaded_assets.read().unwrap().contains(&id));

        std::fs::remove_dir_all(&dir).ok();
    }
    fn fixture_server(dir: &std::path::Path, registry: AssetRegistry) -> (AssetServer, World) {
        let mut server =
            AssetServer::with_content_root(ContentAssetRoot::Directory(dir.to_owned()));
        let store = AssetStore::<FixtureAsset>::new();
        server.register_asset(&store);
        server.publish_registry(registry);
        let mut world = World::new();
        world.insert_resource(store);
        world.insert_resource(server.clone());
        (server, world)
    }

    #[test]
    fn uuid_requests_share_pending_and_loaded_assets() {
        let dir = temp_root("shared-uuid");
        let (id, registry) = saved_fixture(&dir, 42);
        assert_ne!(id, AssetId::from_path("content/fixture/value.gasset"));
        let (server, mut world) = fixture_server(&dir, registry);
        let first = server.load::<FixtureAsset>(id);
        let second = server.load::<FixtureAsset>(id);
        assert_eq!(first.id(), id);
        assert_eq!(second.id(), id);
        match (&first, &second) {
            (AssetHandle::Strong(a, _), AssetHandle::Strong(b, _)) => assert!(Arc::ptr_eq(a, b)),
            _ => panic!("load returns strong handles"),
        }
        assert_eq!(server.data.pending_tasks.read().unwrap().len(), 1);
        drive_pending_load(&server, id);
        handle_asset_load_events(&mut world);
        let third = server.load::<FixtureAsset>(id);
        assert_eq!(third.id(), id);
        assert!(server.data.pending_tasks.read().unwrap().is_empty());
        let store = world.get_resource::<AssetStore<FixtureAsset>>().unwrap();
        assert_eq!(store.into_iter().count(), 1);
        assert_eq!(store.get(&first).unwrap().value, 42);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn serialized_uuid_handle_resolves_on_a_fresh_server() {
        let dir = temp_root("serialized-uuid");
        let (id, registry) = saved_fixture(&dir, 99);
        let serialized = {
            let (server, mut world) = fixture_server(&dir, registry.clone());
            let handle = server.load::<FixtureAsset>(id);
            drive_pending_load(&server, handle.id());
            handle_asset_load_events(&mut world);
            bincode::serialize(&handle).unwrap()
        };
        let weak: AssetHandle<FixtureAsset> = bincode::deserialize(&serialized).unwrap();
        let (server, mut world) = fixture_server(&dir, registry);
        let handle = server.load::<FixtureAsset>(weak.id());
        drive_pending_load(&server, handle.id());
        handle_asset_load_events(&mut world);
        let store = world.get_resource::<AssetStore<FixtureAsset>>().unwrap();
        assert_eq!(store.get(&handle).unwrap().value, 99);
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn saved_fixture(dir: &std::path::Path, value: u32) -> (AssetId, AssetRegistry) {
        let mut registry = AssetRegistry::new();
        let id = save_content_asset_into(
            &FixtureAsset {
                value,
                initialized: false,
            },
            dir,
            "content/fixture/value.gasset",
            &mut registry,
        )
        .unwrap();
        (id, registry)
    }

    #[test]
    fn load_fails_without_reading_when_the_registry_names_another_kind() {
        let dir = temp_root("kind-mismatch");
        let (id, mut registry) = saved_fixture(&dir, 5);
        let (server, mut world) = fixture_server(&dir, registry.clone());
        let entry = registry.get(id).unwrap().clone();
        registry.insert(AssetEntry {
            kind: "Mesh".into(),
            ..entry
        });
        server.publish_registry(registry);

        let handle = server.load::<FixtureAsset>(id);
        drive_pending_load(&server, id);
        handle_asset_load_events(&mut world);

        let store = world.get_resource::<AssetStore<FixtureAsset>>().unwrap();
        assert!(
            store.get(&handle).is_none(),
            "the registry says the id holds a Mesh, so a FixtureAsset load must fail"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_server_resolves_through_the_registry_resource_once_it_changes() {
        use concerto_ecs::{IntoSystem, System};

        let dir = temp_root("sync-registry");
        let (server, mut world) = fixture_server(&dir, AssetRegistry::new());
        let id = AssetId::new();
        let mut system = sync_asset_registry.into_system();
        system.initialize(&mut world);
        world.insert_resource(AssetRegistry::new());
        system.run_and_apply((), &mut world);
        assert_eq!(server.resolve_by_id(id), None);

        world
            .get_resource_mut::<AssetRegistry>()
            .unwrap()
            .insert(scene_entry(id));
        system.run_and_apply((), &mut world);

        assert_eq!(
            server.resolve_by_id(id).as_deref(),
            Some("content/hero/scene.gasset")
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !done() {
            assert!(
                std::time::Instant::now() < deadline,
                "{what} never happened"
            );
            std::thread::yield_now();
        }
    }

    #[test]
    fn a_load_that_finished_but_was_not_yet_handled_is_not_inserted_once_cancelled() {
        let dir = temp_root("cancel-finished");
        let (id, registry) = saved_fixture(&dir, 5);
        let (server, mut world) = fixture_server(&dir, registry);
        let handle = server.load::<FixtureAsset>(id);
        wait_until("the load", || {
            !server.data.asset_load_event_receiver.is_empty()
        });

        server.cancel_pending();
        handle_asset_load_events(&mut world);

        let store = world.get_resource::<AssetStore<FixtureAsset>>().unwrap();
        assert!(store.get(&handle).is_none());
        assert!(server.data.pending_tasks.read().unwrap().is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_load_is_in_flight_until_its_asset_reaches_the_world() {
        let dir = temp_root("in-flight-load");
        let (id, registry) = saved_fixture(&dir, 6);
        let (server, mut world) = fixture_server(&dir, registry);
        assert!(!server.is_loading());

        let handle = server.load::<FixtureAsset>(id);
        assert!(server.is_loading());
        wait_until("the load", || {
            !server.data.asset_load_event_receiver.is_empty()
        });
        assert!(server.is_loading());

        handle_asset_load_events(&mut world);
        assert!(!server.is_loading());
        let store = world.get_resource::<AssetStore<FixtureAsset>>().unwrap();
        assert_eq!(store.get(&handle).map(|asset| asset.value), Some(6));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_failed_load_is_in_flight_until_its_failure_is_handled() {
        let dir = temp_root("in-flight-failure");
        let (server, mut world) = fixture_server(&dir, AssetRegistry::new());

        let _handle = server.load::<FixtureAsset>(AssetId::new());
        wait_until("the failure", || {
            !server.data.asset_load_event_receiver.is_empty()
        });
        assert!(server.is_loading());

        handle_asset_load_events(&mut world);
        assert!(!server.is_loading());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_added_asset_is_in_flight_until_it_reaches_the_world() {
        let dir = temp_root("in-flight-added");
        let (server, mut world) = fixture_server(&dir, AssetRegistry::new());

        let _handle = server.add(FixtureAsset {
            value: 9,
            initialized: false,
        });
        assert!(server.is_loading());

        handle_asset_load_events(&mut world);
        assert!(!server.is_loading());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn cancelled_loads_are_no_longer_in_flight() {
        let dir = temp_root("in-flight-cancelled");
        let (id, registry) = saved_fixture(&dir, 4);
        let (server, _world) = fixture_server(&dir, registry);
        let _handle = server.load::<FixtureAsset>(id);

        server.cancel_pending();

        assert!(!server.is_loading());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[derive(serde::Serialize, serde::Deserialize)]
    struct GatedAsset(u32);

    impl Asset for GatedAsset {
        fn name() -> &'static str {
            "GatedAsset"
        }
    }

    type Gate = (Sender<()>, Receiver<()>);
    static ENTERED: std::sync::OnceLock<Gate> = std::sync::OnceLock::new();
    static RELEASED: std::sync::OnceLock<Gate> = std::sync::OnceLock::new();

    fn gate(cell: &'static std::sync::OnceLock<Gate>) -> &'static Gate {
        cell.get_or_init(crossbeam_channel::unbounded)
    }

    impl LoadableAsset for GatedAsset {
        fn on_load(&mut self, _context: &AssetLoadContext) -> anyhow::Result<()> {
            gate(&ENTERED).0.send(()).unwrap();
            gate(&RELEASED).1.recv().unwrap();
            Ok(())
        }
    }

    #[test]
    fn a_load_cancelled_while_it_runs_never_inserts_its_asset() {
        let dir = temp_root("cancel-running");
        let mut registry = AssetRegistry::new();
        let id = save_content_asset_into(
            &GatedAsset(3),
            &dir,
            "content/fixture/gated.gasset",
            &mut registry,
        )
        .unwrap();
        let mut server = AssetServer::with_content_root(ContentAssetRoot::Directory(dir.clone()));
        let store = AssetStore::<GatedAsset>::new();
        server.register_asset(&store);
        server.publish_registry(registry);
        let mut world = World::new();
        world.insert_resource(store);
        world.insert_resource(server.clone());
        let handle = server.load::<GatedAsset>(id);
        let running = Arc::clone(&server.data.pending_tasks.read().unwrap()[&id].cancelled);
        gate(&ENTERED)
            .1
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();

        server.cancel_pending();
        gate(&RELEASED).0.send(()).unwrap();
        wait_until("the end of the load", || Arc::strong_count(&running) == 1);
        handle_asset_load_events(&mut world);

        let store = world.get_resource::<AssetStore<GatedAsset>>().unwrap();
        assert!(store.get(&handle).is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn cancelling_keeps_an_added_asset() {
        let dir = temp_root("cancel-added");
        let (server, mut world) = fixture_server(&dir, AssetRegistry::new());
        let handle = server.add(FixtureAsset {
            value: 8,
            initialized: false,
        });

        server.cancel_pending();
        handle_asset_load_events(&mut world);

        let store = world.get_resource::<AssetStore<FixtureAsset>>().unwrap();
        assert_eq!(store.get(&handle).map(|asset| asset.value), Some(8));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_server_knows_only_the_asset_types_registered_with_it() {
        let mut server = AssetServer::new();
        assert!(!server.is_registered::<FixtureAsset>());

        server.register_asset(&AssetStore::<FixtureAsset>::new());

        assert!(server.is_registered::<FixtureAsset>());
        assert!(!server.is_registered::<GatedAsset>());
    }
}
