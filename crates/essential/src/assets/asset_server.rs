use std::{
    any::TypeId,
    collections::{BTreeMap, HashMap, HashSet},
    path::Path,
    sync::{Arc, RwLock, Weak},
};

use anyhow::Context;
use crossbeam_channel::{Receiver, Sender};
use ecs::{resource::Resource, world};
use tasks::load_pool::LoadTaskPool;

use crate::{
    assets::{handle::StrongAssetHandle, LoadableAsset},
    tasks::{task_pool::TaskPool, Task},
};

use super::{
    asset_container::AssetContainer,
    asset_store::AssetStore,
    handle::{AssetHandle, AssetLifetimeEvent},
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

pub(crate) struct AssetServerData {
    pending_tasks: RwLock<HashMap<AssetId, Task<()>>>,
    loaded_assets: RwLock<HashSet<AssetId>>,
    handle_provider: AssetHandleProvider,
    asset_load_event_sender: Sender<AssetLoadEvent>,
    asset_load_event_receiver: Receiver<AssetLoadEvent>,
    content: RwLock<ContentState>,
}

struct ContentState {
    root: ContentAssetRoot,
    registry: Option<Arc<BTreeMap<AssetId, String>>>,
    initialization: Option<Task<()>>,
    error: Option<String>,
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
                registry: None,
                initialization: None,
                error: None,
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
        registry: crate::assets::content::AssetRegistry,
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
    pub fn publish_content_source(
        &self,
        root: ContentAssetRoot,
        registry: crate::assets::content::AssetRegistry,
    ) {
        // `load` takes these locks in the same order. Holding the pending lock
        // closes the gap where a request could otherwise attach to the old
        // snapshot while it is being replaced.
        let mut pending = self.data.pending_tasks.write().unwrap();
        {
            let mut content = self.data.content.write().unwrap();
            content.root = root;
            content.registry = Some(Arc::new(registry.into_entries()));
            content.initialization = None;
            content.error = None;
        };
        pending.clear();
    }

    /// Eagerly loads the UUID-to-path registry.
    /// The asset-manager plugin calls this through its readiness lifecycle;
    /// standalone callers may await it to report errors before requesting loads.
    /// Otherwise, load tasks initialize the registry lazily.
    pub async fn initialize(&self) -> anyhow::Result<()> {
        let root = {
            let content = self.data.content.read().unwrap();
            if content.registry.is_some() {
                return Ok(());
            }
            content.root.clone()
        };
        self.initialize_root(root).await
    }

    async fn initialize_root(&self, root: ContentAssetRoot) -> anyhow::Result<()> {
        let result = crate::assets::utils::load_registry(&root)
            .await
            .with_context(|| format!("failed to initialize asset registry at {root:?}"));
        let mut content = self.data.content.write().unwrap();
        match result {
            Ok(registry) => {
                content.registry = Some(Arc::new(registry.into_entries()));
                content.error = None;
                Ok(())
            }
            Err(error) => {
                content.error = Some(format!("{error:#}"));
                Err(error)
            }
        }
    }

    /// Starts registry initialization on first poll, then reports readiness or
    /// the initialization error. Does not block the native or browser runner.
    pub fn poll_initialize(&self) -> anyhow::Result<bool> {
        let mut content = self.data.content.write().unwrap();
        if content.registry.is_some() {
            return Ok(true);
        }
        if let Some(error) = &content.error {
            anyhow::bail!("{error}");
        }
        if content.initialization.is_none() {
            let root = content.root.clone();
            let server = self.clone();
            content.initialization = Some(
                LoadTaskPool::get_or_init(|| TaskPool::with_name("asset-load")).spawn(async move {
                    // initialize_root publishes errors for the next poll.
                    let _ = server.initialize_root(root).await;
                }),
            );
        }
        Ok(false)
    }

    pub fn register_asset<A: Asset>(&mut self, asset: &AssetStore<A>) {
        self.data
            .handle_provider
            .register_asset::<A>(asset.clone_drop_sender());
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
    /// Registry initialization happens lazily if it has not already completed.
    pub fn load<A: LoadableAsset + 'static>(&self, id: AssetId) -> AssetHandle<A> {
        // Hold this lock through task insertion so concurrent requests cannot
        // spawn duplicate loads for the same persistent UUID.
        let mut pending = self.data.pending_tasks.write().unwrap();
        if !pending.contains_key(&id) && !self.data.loaded_assets.read().unwrap().contains(&id) {
            pending.insert(id, self.request_load::<A>(id));
        }
        self.data.handle_provider.request_handle(id)
    }

    pub(crate) fn process_handle_drop(&mut self, id: &AssetId) {
        self.data.loaded_assets.write().unwrap().remove(id);
    }

    #[cfg(test)]
    async fn resolve_by_id(&self, id: AssetId) -> Option<String> {
        self.resolve_asset(id).await.map(|(_, address)| address)
    }

    async fn resolve_asset(&self, id: AssetId) -> Option<(ContentAssetRoot, String)> {
        if let Err(error) = self.initialize().await {
            log::error!("{error:#}");
            return None;
        }
        let content = self.data.content.read().unwrap();
        let address = content.registry.as_ref()?.get(&id)?.clone();
        Some((content.root.clone(), address))
    }

    /// Resolves the UUID through the registry before reading the cooked asset.
    /// A registry miss fails the load without attempting file I/O.
    fn request_load<A: LoadableAsset>(&self, id: AssetId) -> Task<()> {
        let sender = self.data.asset_load_event_sender.clone();

        let server = self.clone();
        // No profiling scope around the async body: a scope guard must not be
        // held across .await (tasks can migrate between worker threads).
        // Load costs show up on the named "asset-load-N" threads instead.
        LoadTaskPool::get_or_init(|| TaskPool::with_name("asset-load")).spawn(async move {
            let (content_root, address) = match server.resolve_asset(id).await {
                Some(resolved) => resolved,
                None => {
                    log::error!(
                        "no content asset registered for AssetId {id:?} (type {})",
                        std::any::type_name::<A>()
                    );
                    sender.send(AssetLoadEvent::LoadFailed { id }).unwrap();
                    return;
                }
            };
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
                    sender
                        .send(AssetLoadEvent::Loaded {
                            asset: LoadedAsset::new(id, asset),
                        })
                        .unwrap();
                }
                Err(error) => {
                    log::error!(
                        "Failed to load asset '{}' (type {}): {:#}",
                        address,
                        std::any::type_name::<A>(),
                        error
                    );
                    sender.send(AssetLoadEvent::LoadFailed { id }).unwrap();
                }
            }
        })
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
    use crate::assets::{
        asset_store::AssetStore,
        content::{read_content_asset_header, save_content_asset, AssetRegistry},
    };
    use ecs::world::World;

    fn temp_root(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("asset-server-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("content")).unwrap();
        dir
    }

    /// A minimal real asset: written to disk by `save_content_asset` and read
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
            .expect("load spawns a task for an id that isn't loaded yet");
        pollster::block_on(task);
    }

    #[test]
    fn resolve_by_id_finds_a_registered_asset() {
        let dir = temp_root("resolve-hit");
        let id = AssetId::from_path("content/hero/scene.gasset");
        let mut registry = AssetRegistry::new();
        registry.insert(id, "content/hero/scene.gasset");
        registry.save(&dir).expect("save registry");

        let server = AssetServer::with_content_root(ContentAssetRoot::Directory(dir.clone()));
        let resolved = pollster::block_on(server.resolve_by_id(id));

        assert_eq!(resolved.as_deref(), Some("content/hero/scene.gasset"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resolve_by_id_returns_none_for_an_unregistered_id() {
        let dir = temp_root("resolve-miss");
        let server = AssetServer::with_content_root(ContentAssetRoot::Directory(dir.clone()));
        let resolved = pollster::block_on(server.resolve_by_id(AssetId::new()));
        assert_eq!(resolved, None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resolve_by_id_caches_the_registry_after_first_load() {
        let dir = temp_root("resolve-cache");
        let id = AssetId::from_path("content/hero/scene.gasset");
        let mut registry = AssetRegistry::new();
        registry.insert(id, "content/hero/scene.gasset");
        registry.save(&dir).expect("save registry");

        let server = AssetServer::with_content_root(ContentAssetRoot::Directory(dir.clone()));
        let first = pollster::block_on(server.resolve_by_id(id));
        assert_eq!(first.as_deref(), Some("content/hero/scene.gasset"));

        // Removing the on-disk registry must not affect a cached lookup.
        std::fs::remove_file(dir.join("content/.registry.toml")).unwrap();
        let second = pollster::block_on(server.resolve_by_id(id));
        assert_eq!(
            second.as_deref(),
            Some("content/hero/scene.gasset"),
            "the registry is cached after first use, so a since-deleted file must not affect the second lookup"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_reads_a_registered_content_asset() {
        let dir = temp_root("load-by-id-hit");
        let address = "content/fixture/value.gasset";
        // Writes both the .gasset file and its registry entry, exactly as
        // `import` and an editor save do.
        save_content_asset(
            &FixtureAsset {
                value: 7,
                initialized: false,
            },
            &dir,
            address,
        )
        .expect("save content asset");
        let id = read_content_asset_header(&dir.join(address))
            .expect("read header")
            .asset_id;

        let mut world = World::new();
        let store = AssetStore::<FixtureAsset>::new();
        let mut server = AssetServer::with_content_root(ContentAssetRoot::Directory(dir.clone()));
        server.register_asset::<FixtureAsset>(&store);
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
    fn fixture_server(dir: &std::path::Path) -> (AssetServer, World) {
        let mut server =
            AssetServer::with_content_root(ContentAssetRoot::Directory(dir.to_owned()));
        let store = AssetStore::<FixtureAsset>::new();
        server.register_asset(&store);
        pollster::block_on(server.initialize()).unwrap();
        let mut world = World::new();
        world.insert_resource(store);
        world.insert_resource(server.clone());
        (server, world)
    }

    #[test]
    fn uuid_requests_share_pending_and_loaded_assets() {
        let dir = temp_root("shared-uuid");
        let address = "content/fixture/value.gasset";
        save_content_asset(
            &FixtureAsset {
                value: 42,
                initialized: false,
            },
            &dir,
            address,
        )
        .unwrap();
        let id = read_content_asset_header(&dir.join(address))
            .unwrap()
            .asset_id;
        assert_ne!(id, AssetId::from_path(address));
        let (server, mut world) = fixture_server(&dir);
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
        let address = "content/fixture/value.gasset";
        save_content_asset(
            &FixtureAsset {
                value: 99,
                initialized: false,
            },
            &dir,
            address,
        )
        .unwrap();
        let id = read_content_asset_header(&dir.join(address))
            .unwrap()
            .asset_id;
        let serialized = {
            let (server, mut world) = fixture_server(&dir);
            let handle = server.load::<FixtureAsset>(id);
            drive_pending_load(&server, handle.id());
            handle_asset_load_events(&mut world);
            bincode::serialize(&handle).unwrap()
        };
        let weak: AssetHandle<FixtureAsset> = bincode::deserialize(&serialized).unwrap();
        let (server, mut world) = fixture_server(&dir);
        let handle = server.load::<FixtureAsset>(weak.id());
        drive_pending_load(&server, handle.id());
        handle_asset_load_events(&mut world);
        let store = world.get_resource::<AssetStore<FixtureAsset>>().unwrap();
        assert_eq!(store.get(&handle).unwrap().value, 99);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn initialization_error_is_reported_and_can_be_retried() {
        let dir = temp_root("init-error");
        std::fs::write(dir.join("content/.registry.toml"), "invalid = [").unwrap();
        let server = AssetServer::with_content_root(ContentAssetRoot::Directory(dir.clone()));
        assert!(pollster::block_on(server.initialize()).is_err());
        assert!(server.poll_initialize().is_err());
        AssetRegistry::new().save(&dir).unwrap();
        pollster::block_on(server.initialize()).unwrap();
        assert!(server.poll_initialize().unwrap());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
