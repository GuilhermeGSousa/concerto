#[cfg(not(target_arch = "wasm32"))]
use std::collections::BTreeMap;
use std::{
    any::TypeId,
    collections::{HashMap, HashSet},
    future::Future,
    path::Path,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, RwLock, Weak,
    },
};

use anyhow::Context;
use concerto_ecs::{
    events::event_channel::EventChannel,
    query::change_detection::DetectChanges,
    reflect::AppTypeRegistry,
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
    content::{
        decode_content_payload, decode_uncooked, AssetRecord, ContentFile, DerivedSection,
        EmbeddedAsset, UncookedBytes,
    },
    derived::{
        derived_hook, note_derived_kind, CookedBlob, DerivedData, DerivedDataCache, DerivedHook,
        DerivedLoad,
    },
    handle::{AssetHandle, AssetLifetimeEvent, AssetLoadFailed},
    registry::{AssetEntry, AssetRegistry},
    utils::try_read_relative,
    Asset, AssetId, ContentAssetRoot, UncookedData,
};
#[cfg(not(target_arch = "wasm32"))]
use super::{
    content::{asset_record, read_content_asset_header, read_uncooked_section},
    derived::{cook_hook, record_is_derived, CookHook, CookReport},
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
    type_registry: AppTypeRegistry,
}

impl AssetLoadContext {
    pub fn asset_server(&self) -> &AssetServer {
        &self.asset_server
    }

    pub fn asset_id(&self) -> AssetId {
        self.asset_id
    }

    /// The type registry shared with the world; take one `read()` guard and register no type while holding it.
    pub fn type_registry(&self) -> &AppTypeRegistry {
        &self.type_registry
    }

    pub(crate) fn new(asset_server: AssetServer, asset_id: AssetId) -> Self {
        let type_registry = asset_server.data.type_registry.read().unwrap().clone();
        Self {
            asset_server,
            asset_id,
            type_registry,
        }
    }
}

pub(crate) struct AssetInfo {
    handle: Weak<StrongAssetHandle>,
}

#[cfg(not(target_arch = "wasm32"))]
struct CookFile<'a> {
    root: &'a ContentAssetRoot,
    dir: &'a Path,
    address: &'a str,
    entries: &'a [&'a AssetEntry],
    hooks: &'a HashMap<&'static str, CookHook>,
}

struct RecordLoad<'a> {
    id: AssetId,
    location: &'a str,
    record: AssetRecord<'a>,
    server: AssetServer,
}

#[cfg(not(target_arch = "wasm32"))]
type RecordFuture<'a> = Pin<Box<dyn Future<Output = anyhow::Result<LoadedAsset>> + Send + 'a>>;
#[cfg(target_arch = "wasm32")]
type RecordFuture<'a> = Pin<Box<dyn Future<Output = anyhow::Result<LoadedAsset>> + 'a>>;

type RecordLoader = for<'a> fn(RecordLoad<'a>) -> RecordFuture<'a>;

fn load_record<A: LoadableAsset>(load: RecordLoad<'_>) -> RecordFuture<'_> {
    Box::pin(async move {
        let RecordLoad {
            id,
            location,
            record,
            server,
        } = load;
        let mut asset =
            decode_content_payload::<A>(record.kind, record.kind_version, record.payload)
                .with_context(|| format!("failed to read {} asset", A::name()))?;
        asset.on_load(&AssetLoadContext::new(server.clone(), id))?;
        let hook = server
            .data
            .derived_hooks
            .read()
            .unwrap()
            .get(&TypeId::of::<A>())
            .copied();
        match hook {
            Some(hook) => {
                let cooked = record.cooked.map(|cooked| CookedBlob {
                    section: DerivedSection {
                        derived_version: cooked.derived_version,
                        key: cooked.key,
                        len: cooked.blob.len() as u64,
                    },
                    blob: cooked.blob.to_vec(),
                });
                let uncooked = record.uncooked_section();
                let load = DerivedLoad {
                    id,
                    asset: Box::new(asset),
                    cooked,
                    uncooked,
                };
                asset = *hook(load, server)
                    .await?
                    .downcast::<A>()
                    .map_err(|_| anyhow::anyhow!("the derived hook returned another type"))?;
            }
            None if record.cooked.is_some() => log::warn!(
                "ignoring the derived section of '{location}': {} is not a derived asset",
                A::name()
            ),
            None => {}
        }
        Ok(LoadedAsset::new(id, asset))
    })
}

struct LoadRequest {
    entry: AssetEntry,
    load: RecordLoader,
    cancelled: Arc<AtomicBool>,
}

struct FileLoad {
    _task: Task<()>,
    cancelled: Arc<AtomicBool>,
    requests: Vec<LoadRequest>,
}

#[derive(Default)]
struct PendingLoads {
    files: HashMap<AssetId, FileLoad>,
    assets: HashMap<AssetId, Arc<AtomicBool>>,
}

impl PendingLoads {
    fn cancel(&mut self) {
        for load in self.files.values() {
            load.cancelled.store(true, Ordering::SeqCst);
        }
        for cancelled in self.assets.values() {
            cancelled.store(true, Ordering::SeqCst);
        }
        self.files.clear();
        self.assets.clear();
    }

    fn cancel_asset(&mut self, id: AssetId) {
        if let Some(cancelled) = self.assets.remove(&id) {
            cancelled.store(true, Ordering::SeqCst);
        }
    }
}

pub(crate) struct AssetServerData {
    pending: RwLock<PendingLoads>,
    loaded_assets: RwLock<HashSet<AssetId>>,
    handle_provider: AssetHandleProvider,
    asset_load_event_sender: Sender<AssetLoadEvent>,
    asset_load_event_receiver: Receiver<AssetLoadEvent>,
    content: RwLock<ContentState>,
    type_registry: RwLock<AppTypeRegistry>,
    format_versions: RwLock<HashMap<&'static str, u32>>,
    derived_hooks: RwLock<HashMap<TypeId, DerivedHook>>,
    #[cfg(not(target_arch = "wasm32"))]
    cook_hooks: RwLock<HashMap<&'static str, CookHook>>,
    derived_cache: RwLock<DerivedDataCache>,
    named: RwLock<HashSet<TypeId>>,
    loaders: RwLock<HashMap<&'static str, RecordLoader>>,
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
            pending: RwLock::default(),
            loaded_assets: RwLock::new(HashSet::new()),
            handle_provider: AssetHandleProvider::new(),
            asset_load_event_sender,
            asset_load_event_receiver,
            content: RwLock::new(ContentState {
                root,
                registry: Arc::default(),
            }),
            type_registry: RwLock::new(AppTypeRegistry::default()),
            format_versions: RwLock::new(HashMap::new()),
            derived_hooks: RwLock::new(HashMap::new()),
            #[cfg(not(target_arch = "wasm32"))]
            cook_hooks: RwLock::new(HashMap::new()),
            derived_cache: RwLock::new(DerivedDataCache::none()),
            named: RwLock::new(HashSet::new()),
            loaders: RwLock::new(HashMap::new()),
        };

        Self {
            data: Arc::new(server_data),
        }
    }

    /// Replaces the type registry loads hand to `on_load`; every clone sees it, and a load uses the one the server holds when its `on_load` starts.
    pub fn set_type_registry(&self, registry: AppTypeRegistry) {
        *self.data.type_registry.write().unwrap() = registry;
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
        let mut pending = self.data.pending.write().unwrap();
        {
            let mut content = self.data.content.write().unwrap();
            content.root = root;
            content.registry = Arc::new(registry);
        };
        pending.cancel();
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
        !self.data.pending.read().unwrap().assets.is_empty()
            || !self.data.asset_load_event_receiver.is_empty()
    }

    /// Whether the asset `id` is in the world or on its way there; false for an id never loaded or added, and once its load fails or is cancelled or it unloads.
    pub fn is_loading_or_loaded(&self, id: AssetId) -> bool {
        self.data.pending.read().unwrap().assets.contains_key(&id)
            || self.data.loaded_assets.read().unwrap().contains(&id)
    }

    /// Cancels the loads pending at the call; none of them delivers its asset afterwards.
    pub fn cancel_pending(&self) {
        let mut pending = self.data.pending.write().unwrap();
        let kept: Vec<_> = self
            .data
            .asset_load_event_receiver
            .try_iter()
            .filter(|event| !pending.assets.contains_key(&event.id()))
            .collect();
        for event in kept {
            let _ = self.data.asset_load_event_sender.send(event);
        }
        pending.cancel();
    }

    pub fn register_asset<A: Asset>(&mut self, asset: &AssetStore<A>) {
        self.data
            .handle_provider
            .register_asset::<A>(asset.clone_drop_sender());
        self.data
            .format_versions
            .write()
            .unwrap()
            .insert(A::name(), A::FORMAT_VERSION);
    }

    /// Registers `A` as [`register_asset`](Self::register_asset) does, and derives its blob on every load.
    pub fn register_derived_asset<A: DerivedData>(&mut self, asset: &AssetStore<A>) {
        self.register_asset(asset);
        note_derived_kind::<A>();
        self.data
            .derived_hooks
            .write()
            .unwrap()
            .insert(TypeId::of::<A>(), derived_hook::<A>);
        #[cfg(not(target_arch = "wasm32"))]
        self.data
            .cook_hooks
            .write()
            .unwrap()
            .insert(A::name(), cook_hook::<A>);
    }

    /// Whether files of `kind` hold a derived type, whose assets that derive [`cook`](Self::cook) writes with their blob.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn can_cook(&self, kind: &str) -> bool {
        self.data.cook_hooks.read().unwrap().contains_key(kind)
    }

    /// Writes to `<out>/<address>` every file of the registry whose asset derives, with its derived blob, and every other file that has an uncooked section, all without their uncooked sections, in address order, stopping at the first error; the report lists only those written.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn cook(&self, out: &Path) -> anyhow::Result<CookReport> {
        let (root, registry) = self.content_source();
        let ContentAssetRoot::Directory(dir) = &root else {
            anyhow::bail!("a cook reads its content from a directory, and this server's content root is a URL");
        };
        let hooks = self.data.cook_hooks.read().unwrap().clone();
        let mut files: BTreeMap<&str, Vec<&AssetEntry>> = BTreeMap::new();
        for entry in registry.iter() {
            files.entry(entry.address.as_str()).or_default().push(entry);
        }
        let mut report = CookReport::default();
        for (address, entries) in files {
            let file = CookFile {
                root: &root,
                dir,
                address,
                entries: &entries,
                hooks: &hooks,
            };
            let written = self
                .cook_file(file, out)
                .with_context(|| format!("failed to cook '{address}'"))?;
            if written {
                report.cooked.push(address.to_owned());
            }
        }
        Ok(report)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn cook_file(&self, file: CookFile<'_>, out: &Path) -> anyhow::Result<bool> {
        let CookFile {
            root,
            dir,
            address,
            entries,
            hooks,
        } = file;
        let has_uncooked = read_content_asset_header(&dir.join(address))?.has_uncooked();
        let hooked = entries
            .iter()
            .any(|entry| hooks.contains_key(entry.kind.as_str()));
        if !has_uncooked && !hooked {
            return Ok(false);
        }
        let stripped = pollster::block_on(try_read_relative(root, address))?
            .with_context(|| format!("no content asset at '{address}'"))?;
        let mut cooked = None;
        for entry in entries {
            let Some(&hook) = hooks.get(entry.kind.as_str()) else {
                continue;
            };
            match &entry.embedded_in {
                Some(embedded) => {
                    let record = asset_record(&stripped, Some(&embedded.name))?;
                    if record_is_derived(&entry.kind, &record)? {
                        anyhow::bail!(
                            "{} is embedded in an import's file; embedded assets cannot be cooked yet",
                            entry.location()
                        );
                    }
                }
                None => {
                    cooked = pollster::block_on(hook(entry.id, stripped.clone(), self.clone()))
                        .with_context(|| {
                            format!(
                                "failed to cook the {} at '{}'",
                                entry.kind,
                                entry.location()
                            )
                        })?;
                }
            }
        }
        let bytes = match cooked {
            Some(cooked) => cooked,
            None if has_uncooked => stripped,
            None => return Ok(false),
        };
        let path = out.join(address);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create '{}'", parent.display()))?;
        }
        std::fs::write(&path, bytes)
            .with_context(|| format!("failed to write '{}'", path.display()))?;
        Ok(true)
    }

    /// The uncooked data of the asset `id`, read alone from its file through the published root and registry; `None` when its record has none, as in a cooked file.
    pub async fn load_uncooked<E: UncookedData>(&self, id: AssetId) -> anyhow::Result<Option<E>> {
        let Some(section) = self.uncooked_section(id).await? else {
            return Ok(None);
        };
        decode_uncooked::<E>(&section.kind, section.version, &section.bytes)
            .with_context(|| format!("malformed uncooked data of AssetId {id:?}"))
            .map(Some)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) async fn skipped_uncooked(
        &self,
        id: AssetId,
    ) -> anyhow::Result<Option<UncookedBytes>> {
        let (root, registry) = self.content_source();
        match root {
            ContentAssetRoot::Directory(_) if registry.get(id).is_some() => {
                self.uncooked_section(id).await
            }
            _ => Ok(None),
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) async fn skipped_uncooked(
        &self,
        _id: AssetId,
    ) -> anyhow::Result<Option<UncookedBytes>> {
        Ok(None)
    }

    async fn uncooked_section(&self, id: AssetId) -> anyhow::Result<Option<UncookedBytes>> {
        let (root, registry) = self.content_source();
        let entry = registry
            .get(id)
            .with_context(|| format!("no content asset registered for AssetId {id:?}"))?;
        let location = entry.location();
        match &root {
            #[cfg(not(target_arch = "wasm32"))]
            ContentAssetRoot::Directory(dir) => {
                let path = dir.join(&entry.address);
                let name = entry.embedded_in.as_ref().map(|held| held.name.clone());
                blocking::unblock(move || read_uncooked_section(&path, name.as_deref())).await
            }
            _ => {
                let file = try_read_relative(&root, &entry.address)
                    .await?
                    .with_context(|| format!("no content asset at '{}'", entry.address))?;
                let content = ContentFile::parse(&file)
                    .with_context(|| format!("malformed content asset '{location}'"))?;
                let record = match &entry.embedded_in {
                    None => content.host(),
                    Some(held) => content
                        .header()
                        .embedded
                        .iter()
                        .find(|asset| asset.asset_id == id)
                        .with_context(|| {
                            format!("it holds no embedded asset '{}' with this id", held.name)
                        })
                        .and_then(|asset| content.embedded(asset)),
                }
                .with_context(|| format!("malformed content asset '{location}'"))?;
                Ok(record.uncooked_section())
            }
        }
    }

    /// Replaces the cache derived loads read and write; every clone sees it.
    pub fn set_derived_cache(&self, cache: DerivedDataCache) {
        *self.data.derived_cache.write().unwrap() = cache;
    }

    /// The cache derived loads read and write, none until one is set.
    pub fn derived_cache(&self) -> DerivedDataCache {
        self.data.derived_cache.read().unwrap().clone()
    }

    /// The `FORMAT_VERSION` of the registered asset type named `kind`.
    pub fn format_version(&self, kind: &str) -> Option<u32> {
        self.data.format_versions.read().unwrap().get(kind).copied()
    }

    /// Whether `A` was registered, without which `load` and `add` panic.
    pub fn is_registered<A: Asset>(&self) -> bool {
        self.data.handle_provider.is_registered::<A>()
    }

    /// Lets a handle to `A` be set through this server as a weak one that names its asset and loads nothing, for an editor that edits references to a kind it does not load.
    pub fn name_asset<A: Asset>(&self) {
        self.data.named.write().unwrap().insert(TypeId::of::<A>());
    }

    /// Whether handles to `A` set through this server only name their asset.
    pub fn names_asset<A: Asset>(&self) -> bool {
        self.data.named.read().unwrap().contains(&TypeId::of::<A>())
    }

    /// Adds a built asset, running its `on_load` inline before it reaches the world; a failing `on_load` is logged and the returned handle never resolves.
    pub fn add<A: LoadableAsset>(&self, mut asset: A) -> AssetHandle<A> {
        let id = AssetId::new();
        let handle = self.data.handle_provider.request_handle(id);

        self.data.loaded_assets.write().unwrap().insert(id);
        let context = AssetLoadContext::new(self.clone(), id);
        if let Err(error) = asset.on_load(&context) {
            log::error!(
                "Failed to add asset (type {}): {:#}",
                std::any::type_name::<A>(),
                error
            );
            self.data.loaded_assets.write().unwrap().remove(&id);
            return handle;
        }

        let sender = self.data.asset_load_event_sender.clone();
        let _ = sender.send(AssetLoadEvent::Loaded {
            asset: LoadedAsset::new(id, asset),
        });
        handle
    }

    /// Puts a built asset in its store under `id`, running its `on_load` inline and dropping a pending load of that id, so `load(id)` returns it and reads no file; inserting again replaces it, and a failing `on_load` is logged and leaves what was there.
    pub fn insert_loaded<A: LoadableAsset>(&self, id: AssetId, mut asset: A) -> AssetHandle<A> {
        let handle = self.data.handle_provider.request_handle(id);
        let was_loaded = {
            self.data.pending.write().unwrap().cancel_asset(id);
            !self.data.loaded_assets.write().unwrap().insert(id)
        };
        let context = AssetLoadContext::new(self.clone(), id);
        if let Err(error) = asset.on_load(&context) {
            log::error!(
                "Failed to insert asset (type {}): {:#}",
                std::any::type_name::<A>(),
                error
            );
            if !was_loaded {
                self.data.loaded_assets.write().unwrap().remove(&id);
            }
            return handle;
        }
        let _ = self
            .data
            .asset_load_event_sender
            .send(AssetLoadEvent::Loaded {
                asset: LoadedAsset::new(id, asset),
            });
        handle
    }

    /// Loads a stored asset using its persistent UUID, obtained from `asset_id!`
    /// or a serialized asset reference. Repeated requests share handles and loads,
    /// and the assets requested from one file share one read of it.
    pub fn load<A: LoadableAsset + 'static>(&self, id: AssetId) -> AssetHandle<A> {
        let handle = self.data.handle_provider.request_handle(id);
        let mut pending = self.data.pending.write().unwrap();
        if pending.assets.contains_key(&id) || self.data.loaded_assets.read().unwrap().contains(&id)
        {
            return handle;
        }
        self.start_load::<A>(id, &mut pending);
        handle
    }

    /// Reads the asset `id` from its file again, though it is loaded or was inserted, and replaces it once the read lands; a read that fails leaves what was there.
    pub fn reload<A: LoadableAsset + 'static>(&self, id: AssetId) -> AssetHandle<A> {
        let handle = self.data.handle_provider.request_handle(id);
        let mut pending = self.data.pending.write().unwrap();
        pending.cancel_asset(id);
        self.start_load::<A>(id, &mut pending);
        handle
    }

    /// Reads every asset of the file `file` that is loaded or loading again, its host and the assets embedded in it, each replaced once the read lands; an asset of a kind this server never loaded is left as it is.
    pub fn reload_file(&self, file: AssetId) {
        let loaders = self.data.loaders.read().unwrap().clone();
        let (_, registry) = self.content_source();
        let mut pending = self.data.pending.write().unwrap();
        for entry in registry.iter().filter(|entry| entry.file() == file) {
            let id = entry.id;
            let loaded = pending.assets.contains_key(&id)
                || self.data.loaded_assets.read().unwrap().contains(&id);
            let Some(&loader) = loaders.get(entry.kind.as_str()) else {
                continue;
            };
            if loaded {
                pending.cancel_asset(id);
                self.start_load_with(id, &entry.kind, loader, &mut pending);
            }
        }
    }

    fn start_load<A: LoadableAsset + 'static>(&self, id: AssetId, pending: &mut PendingLoads) {
        self.data
            .loaders
            .write()
            .unwrap()
            .insert(A::name(), load_record::<A>);
        self.start_load_with(id, A::name(), load_record::<A>, pending);
    }

    fn start_load_with(
        &self,
        id: AssetId,
        kind: &str,
        load: RecordLoader,
        pending: &mut PendingLoads,
    ) {
        let request_cancelled = Arc::new(AtomicBool::new(false));
        pending.assets.insert(id, Arc::clone(&request_cancelled));
        let (root, registry) = self.content_source();
        let Some(entry) = registry.get(id).cloned() else {
            log::error!("no content asset registered for AssetId {id:?} (kind {kind})");
            self.fail(id);
            return;
        };
        if entry.kind != kind {
            log::error!(
                "content asset '{}' holds a {} but a {kind} was requested",
                entry.location(),
                entry.kind,
            );
            self.fail(id);
            return;
        }
        let file = entry.file();
        let address = entry.address.clone();
        let request = LoadRequest {
            entry,
            load,
            cancelled: request_cancelled,
        };
        match pending.files.get_mut(&file) {
            Some(load) => load.requests.push(request),
            None => {
                let cancelled = Arc::new(AtomicBool::new(false));
                let task = self.load_file(file, root, address, Arc::clone(&cancelled));
                pending.files.insert(
                    file,
                    FileLoad {
                        _task: task,
                        cancelled,
                        requests: vec![request],
                    },
                );
            }
        }
    }

    fn fail(&self, id: AssetId) {
        let _ = self
            .data
            .asset_load_event_sender
            .send(AssetLoadEvent::LoadFailed { id });
    }

    pub(crate) fn process_handle_drop(&mut self, id: &AssetId) -> bool {
        if self.data.handle_provider.has_live_handle(id) {
            return false;
        }
        self.data.loaded_assets.write().unwrap().remove(id);
        true
    }

    pub(crate) fn content_source(&self) -> (ContentAssetRoot, Arc<AssetRegistry>) {
        let content = self.data.content.read().unwrap();
        (content.root.clone(), Arc::clone(&content.registry))
    }

    fn take_requests(&self, file: AssetId, cancelled: &AtomicBool) -> Option<Vec<LoadRequest>> {
        let mut pending = self.data.pending.write().unwrap();
        if cancelled.load(Ordering::SeqCst) {
            return None;
        }
        let PendingLoads { files, assets } = &mut *pending;
        let mut requests = std::mem::take(&mut files.get_mut(&file)?.requests);
        if requests.is_empty() {
            files.remove(&file);
            return None;
        }
        requests.retain(|request| {
            if request.cancelled.load(Ordering::SeqCst) {
                return false;
            }
            let id = request.entry.id;
            let wanted = self.data.handle_provider.has_live_handle(&id);
            if !wanted {
                assets.remove(&id);
            }
            wanted
        });
        Some(requests)
    }

    fn deliver(&self, cancelled: &AtomicBool, request: &AtomicBool, event: AssetLoadEvent) {
        let _pending = self.data.pending.read().unwrap();
        if !cancelled.load(Ordering::SeqCst) && !request.load(Ordering::SeqCst) {
            let _ = self.data.asset_load_event_sender.send(event);
        }
    }

    fn load_file(
        &self,
        file: AssetId,
        root: ContentAssetRoot,
        address: String,
        cancelled: Arc<AtomicBool>,
    ) -> Task<()> {
        let server = self.clone();
        // No profiling scope around the async body: a scope guard must not be
        // held across .await (tasks can migrate between worker threads).
        // Load costs show up on the named "asset-load-N" threads instead.
        LoadTaskPool::get_or_init(|| TaskPool::with_name("asset-load")).spawn(async move {
            let bytes = match try_read_relative(&root, &address).await {
                Ok(Some(bytes)) => Ok(bytes),
                Ok(None) => Err(format!("no content asset at '{address}'")),
                Err(error) => Err(format!("{error:#}")),
            };
            let content = bytes.as_ref().map_err(String::clone).and_then(|bytes| {
                ContentFile::parse(bytes)
                    .map_err(|error| format!("malformed content asset '{address}': {error:#}"))
            });
            let embedded: HashMap<AssetId, &EmbeddedAsset> = content
                .iter()
                .flat_map(|content| &content.header().embedded)
                .map(|asset| (asset.asset_id, asset))
                .collect();
            while let Some(requests) = server.take_requests(file, &cancelled) {
                for LoadRequest {
                    entry,
                    load,
                    cancelled: request_cancelled,
                } in requests
                {
                    if cancelled.load(Ordering::SeqCst) {
                        return;
                    }
                    let id = entry.id;
                    let location = entry.location();
                    let loaded = async {
                        let content = content
                            .as_ref()
                            .map_err(|message| anyhow::anyhow!("{message}"))?;
                        let record = match &entry.embedded_in {
                            None => content.host(),
                            Some(held) => embedded
                                .get(&id)
                                .with_context(|| {
                                    format!(
                                        "it holds no embedded asset '{}' with this id",
                                        held.name
                                    )
                                })
                                .and_then(|asset| content.embedded(asset)),
                        }
                        .with_context(|| format!("malformed content asset '{location}'"))?;
                        load(RecordLoad {
                            id,
                            location: &location,
                            record,
                            server: server.clone(),
                        })
                        .await
                    }
                    .await;
                    let event = match loaded {
                        Ok(asset) => AssetLoadEvent::Loaded { asset },
                        Err(error) => {
                            log::error!(
                                "Failed to load asset '{}' (type {}): {:#}",
                                location,
                                entry.kind,
                                error
                            );
                            AssetLoadEvent::LoadFailed { id }
                        }
                    };
                    server.deliver(&cancelled, &request_cancelled, event);
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
                    .loaded_assets
                    .write()
                    .unwrap()
                    .insert(loaded_asset.id);
                server
                    .data
                    .pending
                    .write()
                    .unwrap()
                    .assets
                    .remove(&loaded_asset.id);
                loaded_asset.value.insert(loaded_asset.id, world);
            }
            AssetLoadEvent::LoadFailed { id } => {
                server.data.pending.write().unwrap().assets.remove(&id);
                server.data.loaded_assets.write().unwrap().remove(&id);
                if let Some(failed) = world.get_resource_mut::<EventChannel<AssetLoadFailed>>() {
                    failed.push_event(AssetLoadFailed { id });
                }
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

    fn has_live_handle(&self, id: &AssetId) -> bool {
        self.asset_handles
            .read()
            .unwrap()
            .get(id)
            .is_some_and(|info| info.handle.upgrade().is_some())
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
    use concerto_ecs::{
        component::Component, entity::EntityHashMap, events::event_cursor::EventCursor,
        reflect::ReflectSceneComponent, world::World,
    };
    use concerto_reflect::ReflectDefault;

    fn temp_root(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("asset-server-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("content")).unwrap();
        dir
    }

    #[derive(serde::Serialize, serde::Deserialize)]
    struct FixtureAsset {
        value: u32,
        #[serde(skip)]
        initialized: bool,
    }

    impl FixtureAsset {
        fn new(value: u32) -> Self {
            Self {
                value,
                initialized: false,
            }
        }
    }

    const REFUSED: u32 = u32::MAX;

    impl Asset for FixtureAsset {
        fn name() -> &'static str {
            "FixtureAsset"
        }
    }

    impl LoadableAsset for FixtureAsset {
        fn on_load(&mut self, _context: &AssetLoadContext) -> anyhow::Result<()> {
            anyhow::ensure!(self.value != REFUSED, "refused");
            self.initialized = true;
            Ok(())
        }
    }

    fn scene_entry(id: AssetId) -> AssetEntry {
        AssetEntry {
            embedded_in: None,
            id,
            address: "content/hero/scene.gasset".into(),
            kind: "Scene".into(),
            kind_version: None,
            provenance: None,
        }
    }

    struct Fixture {
        server: AssetServer,
        world: World,
    }

    impl Fixture {
        fn new<A: Asset>(dir: &std::path::Path, registry: AssetRegistry) -> Self {
            let mut server =
                AssetServer::with_content_root(ContentAssetRoot::Directory(dir.to_owned()));
            let store = AssetStore::<A>::new();
            server.register_asset(&store);
            server.publish_registry(registry);
            let mut world = World::new();
            world.insert_resource(store);
            world.insert_resource(server.clone());
            Self { server, world }
        }

        fn value(&self, handle: &AssetHandle<FixtureAsset>) -> Option<u32> {
            let store = self
                .world
                .get_resource::<AssetStore<FixtureAsset>>()
                .unwrap();
            store.get(handle).map(|asset| {
                assert!(asset.initialized, "on_load ran before the asset landed");
                asset.value
            })
        }
    }

    struct Saved {
        id: AssetId,
        registry: AssetRegistry,
    }

    fn save<A: Asset>(dir: &std::path::Path, value: &A, address: &str) -> Saved {
        let mut registry = AssetRegistry::new();
        let id = save_content_asset_into(value, dir, address, &mut registry).unwrap();
        Saved { id, registry }
    }

    fn saved_fixture(dir: &std::path::Path, value: u32) -> Saved {
        save(
            dir,
            &FixtureAsset::new(value),
            "content/fixture/value.gasset",
        )
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

    fn wait_for_the_load(server: &AssetServer) {
        wait_until("the load", || {
            !server.data.asset_load_event_receiver.is_empty()
        });
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

        let mut capture = SceneCapture::new(EntityHashMap::from_iter([(entity, 0)]));
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

    #[test]
    fn a_handles_type_data_describes_it_and_sets_it_only_through_a_server_that_registered_its_asset(
    ) {
        let mut world = World::new();
        world.register_type::<Holder>();
        let handle = world
            .get_resource::<concerto_ecs::reflect::AppTypeRegistry>()
            .unwrap()
            .get_type_data::<ReflectAssetHandle>(TypeId::of::<AssetHandle<FixtureAsset>>())
            .unwrap();
        let dir = temp_root("described-handle");
        let id = AssetId::new();

        assert_eq!(handle.kind(), "FixtureAsset");
        assert_eq!(handle.id(&AssetHandle::<FixtureAsset>::weak(id)), Some(id));
        assert_eq!(handle.id(&7u32), None);

        let mut value = AssetHandle::<FixtureAsset>::default();
        let bare = AssetServer::with_content_root(ContentAssetRoot::Directory(dir.clone()));
        assert!(!bare.is_registered::<FixtureAsset>());
        assert!(handle.set(&mut value, &bare, id).is_err());
        assert!(value.id().is_nil());

        let store = AssetStore::<FixtureAsset>::new();
        let mut server = AssetServer::with_content_root(ContentAssetRoot::Directory(dir.clone()));
        server.register_asset::<FixtureAsset>(&store);
        assert!(handle.set(&mut 7u32, &server, id).is_err());
        handle.set(&mut value, &server, id).unwrap();
        assert!(matches!(value, AssetHandle::Strong(..)));
        assert_eq!(value.id(), id);

        let naming = AssetServer::with_content_root(ContentAssetRoot::Directory(dir.clone()));
        naming.name_asset::<FixtureAsset>();
        let mut named = AssetHandle::<FixtureAsset>::default();
        handle.set(&mut named, &naming, id).unwrap();
        assert_eq!(named, AssetHandle::weak(id));
        assert!(!naming.is_loading_or_loaded(id));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn live_leaves_load_a_handle_through_the_server_or_keep_it_weak_without_one_and_keep_entity_references_live(
    ) {
        use crate::assets::handle::LiveLeaves;
        use concerto_ecs::{component::scene::SceneEntityRef, reflect::AppTypeRegistry};

        let dir = temp_root("live-handle");
        let store = AssetStore::<FixtureAsset>::new();
        let mut server = AssetServer::with_content_root(ContentAssetRoot::Directory(dir.clone()));
        server.register_asset::<FixtureAsset>(&store);
        for with_server in [true, false] {
            let mut world = World::new();
            if with_server {
                world.insert_resource(server.clone());
            }
            world.register_type::<AssetHandle<FixtureAsset>>();
            let entity = world.spawn(());
            let id = AssetId::new();
            let mut hook = LiveLeaves::new(&world);
            let registry = world.get_resource::<AppTypeRegistry>().unwrap().read();
            let asset = concerto_reflect::json_to_value_with(
                &serde_json::Value::String(id.simple_hex()),
                TypeId::of::<AssetHandle<FixtureAsset>>(),
                &registry,
                &mut hook,
            )
            .unwrap();
            let reference = concerto_reflect::json_to_value_with(
                &serde_json::Value::String(entity.to_id_string()),
                TypeId::of::<SceneEntityRef>(),
                &registry,
                &mut hook,
            )
            .unwrap();

            let mut handle = AssetHandle::<FixtureAsset>::default();
            let mut written = SceneEntityRef::default();
            assert!(concerto_reflect::apply_value_with(&mut handle, &asset, &mut hook).is_empty());
            assert!(
                concerto_reflect::apply_value_with(&mut written, &reference, &mut hook).is_empty()
            );

            assert_eq!(handle.id(), id);
            assert_eq!(matches!(handle, AssetHandle::Strong(..)), with_server);
            assert_eq!(server.is_loading_or_loaded(id), with_server);
            assert_eq!(written, SceneEntityRef::Entity(entity));
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn uuid_requests_share_pending_and_loaded_assets() {
        let dir = temp_root("shared-uuid");
        let saved = saved_fixture(&dir, 42);
        let id = saved.id;
        assert_ne!(id, AssetId::from_path("content/fixture/value.gasset"));
        let mut fixture = Fixture::new::<FixtureAsset>(&dir, saved.registry);
        let first = fixture.server.load::<FixtureAsset>(id);
        let second = fixture.server.load::<FixtureAsset>(id);
        assert_eq!(first.id(), id);
        assert_eq!(second.id(), id);
        match &first {
            AssetHandle::Strong(a, _) => match &second {
                AssetHandle::Strong(b, _) => assert!(Arc::ptr_eq(a, b)),
                AssetHandle::Weak(..) => panic!("load returns strong handles"),
            },
            AssetHandle::Weak(..) => panic!("load returns strong handles"),
        }
        wait_for_the_load(&fixture.server);
        handle_asset_load_events(&mut fixture.world);
        let third = fixture.server.load::<FixtureAsset>(id);
        assert_eq!(third.id(), id);
        assert!(!fixture.server.is_loading());
        let store = fixture
            .world
            .get_resource::<AssetStore<FixtureAsset>>()
            .unwrap();
        assert_eq!(store.into_iter().count(), 1);
        assert_eq!(fixture.value(&first), Some(42));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_inserted_asset_stands_in_for_its_file_and_a_second_insert_replaces_it() {
        let dir = temp_root("insert-loaded");
        let saved = saved_fixture(&dir, 1);
        let id = saved.id;
        let mut fixture = Fixture::new::<FixtureAsset>(&dir, saved.registry);
        let pending = fixture.server.load::<FixtureAsset>(id);

        let inserted = fixture.server.insert_loaded(id, FixtureAsset::new(2));
        let loaded = fixture.server.load::<FixtureAsset>(id);

        assert_eq!(inserted.id(), id);
        handle_asset_load_events(&mut fixture.world);
        assert!(!fixture.server.is_loading());
        assert_eq!(fixture.value(&loaded), Some(2));
        assert_eq!(fixture.value(&pending), Some(2));

        fixture.server.insert_loaded(id, FixtureAsset::new(3));
        fixture.server.insert_loaded(id, FixtureAsset::new(REFUSED));
        handle_asset_load_events(&mut fixture.world);

        assert_eq!(fixture.value(&loaded), Some(3));
        assert!(fixture.server.is_loading_or_loaded(id));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_server_resolves_through_the_registry_resource_once_it_changes() {
        use concerto_ecs::{IntoSystem, System};

        let dir = temp_root("sync-registry");
        let mut fixture = Fixture::new::<FixtureAsset>(&dir, AssetRegistry::new());
        let id = AssetId::new();
        let address = |server: &AssetServer| {
            server
                .registry()
                .and_then(|registry| registry.get(id).map(|entry| entry.address.clone()))
        };
        let mut system = sync_asset_registry.into_system();
        system.initialize(&mut fixture.world);
        fixture.world.insert_resource(AssetRegistry::new());
        system.run_and_apply((), &mut fixture.world);
        assert_eq!(address(&fixture.server), None);

        fixture
            .world
            .get_resource_mut::<AssetRegistry>()
            .unwrap()
            .insert(scene_entry(id));
        system.run_and_apply((), &mut fixture.world);

        assert_eq!(
            address(&fixture.server).as_deref(),
            Some("content/hero/scene.gasset")
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    struct Life {
        name: &'static str,
        request: fn(&AssetServer, AssetId) -> AssetHandle<FixtureAsset>,
        settles: bool,
        loading: [bool; 3],
        loading_or_loaded: [bool; 3],
        value: Option<u32>,
        failed: bool,
    }

    #[test]
    fn a_request_is_in_flight_until_its_asset_or_failure_reaches_the_world() {
        let dir = temp_root("life");
        let saved = saved_fixture(&dir, 6);
        let cases = [
            Life {
                name: "a load of a registered id",
                request: |server, id| server.load(id),
                settles: true,
                loading: [true, true, false],
                loading_or_loaded: [true, true, true],
                value: Some(6),
                failed: false,
            },
            Life {
                name: "a load of an unregistered id",
                request: |server, _| server.load(AssetId::new()),
                settles: true,
                loading: [true, true, false],
                loading_or_loaded: [true, true, false],
                value: None,
                failed: true,
            },
            Life {
                name: "an added asset",
                request: |server, _| server.add(FixtureAsset::new(5)),
                settles: false,
                loading: [true, true, false],
                loading_or_loaded: [true, true, true],
                value: Some(5),
                failed: false,
            },
        ];
        for case in cases {
            let mut fixture = Fixture::new::<FixtureAsset>(&dir, saved.registry.clone());
            fixture
                .world
                .insert_resource(EventChannel::<AssetLoadFailed>::new());
            assert!(!fixture.server.is_loading(), "{}", case.name);
            let handle = (case.request)(&fixture.server, saved.id);
            let mut loading = Vec::new();
            let mut loading_or_loaded = Vec::new();
            let mut observe = |server: &AssetServer| {
                loading.push(server.is_loading());
                loading_or_loaded.push(server.is_loading_or_loaded(handle.id()));
            };
            observe(&fixture.server);
            if case.settles {
                wait_for_the_load(&fixture.server);
            }
            observe(&fixture.server);
            handle_asset_load_events(&mut fixture.world);
            observe(&fixture.server);
            assert_eq!(loading, case.loading, "{}", case.name);
            assert_eq!(loading_or_loaded, case.loading_or_loaded, "{}", case.name);
            assert_eq!(fixture.value(&handle), case.value, "{}", case.name);
            let failed: Vec<AssetId> = fixture
                .world
                .get_resource::<EventChannel<AssetLoadFailed>>()
                .unwrap()
                .read_from(&mut EventCursor::default())
                .map(|event| event.id)
                .collect();
            let expected = if case.failed {
                vec![handle.id()]
            } else {
                vec![]
            };
            assert_eq!(failed, expected, "{}", case.name);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    struct Cancel {
        name: &'static str,
        request: fn(&AssetServer, AssetId) -> AssetHandle<FixtureAsset>,
        settles: bool,
        loading: bool,
        value: Option<u32>,
    }

    #[test]
    fn cancelling_drops_every_pending_load_and_keeps_added_assets() {
        let dir = temp_root("cancel");
        let saved = saved_fixture(&dir, 4);
        let cases = [
            Cancel {
                name: "a load that finished but was not yet handled",
                request: |server, id| server.load(id),
                settles: true,
                loading: false,
                value: None,
            },
            Cancel {
                name: "an added asset",
                request: |server, _| server.add(FixtureAsset::new(8)),
                settles: false,
                loading: true,
                value: Some(8),
            },
        ];
        for case in cases {
            let mut fixture = Fixture::new::<FixtureAsset>(&dir, saved.registry.clone());
            let handle = (case.request)(&fixture.server, saved.id);
            if case.settles {
                wait_for_the_load(&fixture.server);
            }

            fixture.server.cancel_pending();

            assert_eq!(fixture.server.is_loading(), case.loading, "{}", case.name);
            handle_asset_load_events(&mut fixture.world);
            assert_eq!(fixture.value(&handle), case.value, "{}", case.name);
            assert!(!fixture.server.is_loading(), "{}", case.name);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[derive(concerto_reflect::Reflect, Default)]
    #[reflect(Default)]
    struct Probed;

    #[derive(serde::Serialize, serde::Deserialize)]
    struct ProbeAsset {
        #[serde(skip)]
        saw_default: bool,
    }

    impl Asset for ProbeAsset {
        fn name() -> &'static str {
            "ProbeAsset"
        }
    }

    impl LoadableAsset for ProbeAsset {
        fn on_load(&mut self, context: &AssetLoadContext) -> anyhow::Result<()> {
            self.saw_default = context
                .type_registry()
                .get_type_data::<ReflectDefault>(TypeId::of::<Probed>())
                .is_some();
            Ok(())
        }
    }

    struct Shared {
        name: &'static str,
        share: fn(&AssetServer),
        sees: bool,
    }

    #[test]
    fn on_load_reads_type_data_from_the_registry_shared_with_the_server_when_it_loads() {
        let cases = [
            Shared {
                name: "a registry holding the type",
                share: |server| {
                    let registry = AppTypeRegistry::default();
                    registry.write().register::<Probed>();
                    server.set_type_registry(registry);
                },
                sees: true,
            },
            Shared {
                name: "a type registered after the registry was shared",
                share: |server| {
                    let registry = AppTypeRegistry::default();
                    server.set_type_registry(registry.clone());
                    registry.write().register::<Probed>();
                },
                sees: true,
            },
        ];
        let dir = temp_root("on-load-registry");
        let saved = save(
            &dir,
            &ProbeAsset { saw_default: false },
            "content/fixture/probe.gasset",
        );
        for case in cases {
            let mut fixture = Fixture::new::<ProbeAsset>(&dir, saved.registry.clone());
            (case.share)(&fixture.server);
            let handle = fixture.server.load::<ProbeAsset>(saved.id);
            wait_for_the_load(&fixture.server);
            handle_asset_load_events(&mut fixture.world);
            let store = fixture
                .world
                .get_resource::<AssetStore<ProbeAsset>>()
                .unwrap();
            assert_eq!(
                store.get(&handle).expect("probe loaded").saw_default,
                case.sees,
                "{}",
                case.name
            );
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[derive(serde::Serialize, serde::Deserialize)]
    struct GatedAsset(u32);

    impl Asset for GatedAsset {
        fn name() -> &'static str {
            "GatedAsset"
        }
    }

    struct Gate {
        sender: Sender<()>,
        receiver: Receiver<()>,
    }

    static ENTERED: std::sync::OnceLock<Gate> = std::sync::OnceLock::new();
    static RELEASED: std::sync::OnceLock<Gate> = std::sync::OnceLock::new();

    fn gate(cell: &'static std::sync::OnceLock<Gate>) -> &'static Gate {
        cell.get_or_init(|| {
            let (sender, receiver) = crossbeam_channel::unbounded();
            Gate { sender, receiver }
        })
    }

    impl LoadableAsset for GatedAsset {
        fn on_load(&mut self, _context: &AssetLoadContext) -> anyhow::Result<()> {
            gate(&ENTERED).sender.send(()).unwrap();
            gate(&RELEASED).receiver.recv().unwrap();
            Ok(())
        }
    }

    #[test]
    fn a_load_cancelled_while_it_runs_never_inserts_its_asset() {
        let dir = temp_root("cancel-running");
        let saved = save(&dir, &GatedAsset(3), "content/fixture/gated.gasset");
        let mut fixture = Fixture::new::<GatedAsset>(&dir, saved.registry);
        let handle = fixture.server.load::<GatedAsset>(saved.id);
        let running =
            Arc::clone(&fixture.server.data.pending.read().unwrap().files[&saved.id].cancelled);
        gate(&ENTERED)
            .receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();

        fixture.server.cancel_pending();
        gate(&RELEASED).sender.send(()).unwrap();
        wait_until("the end of the load", || Arc::strong_count(&running) == 1);
        handle_asset_load_events(&mut fixture.world);

        let store = fixture
            .world
            .get_resource::<AssetStore<GatedAsset>>()
            .unwrap();
        assert!(store.get(&handle).is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
