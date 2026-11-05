use std::{
    any::Any,
    collections::{BTreeSet, HashMap},
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::RwLock,
};

use anyhow::Context;

use super::{
    asset_server::AssetServer, content::decode_content_asset, registry::AssetRegistry,
    utils::try_read_relative, Asset, AssetId, ContentAssetRoot, LoadableAsset,
};

/// An asset whose runtime part is a blob derived from other content assets, cached by a key it defines.
pub trait DerivedData: LoadableAsset {
    /// Bumped when `derive`'s output changes for the same inputs; part of the cache path.
    const DERIVED_VERSION: u32;

    /// The content assets the blob is built from.
    fn derived_inputs(&self) -> Vec<AssetId>;

    /// The cache key for the blob these inputs give; equal keys mean an equal blob.
    fn derived_key(&self, inputs: &DerivedInputs) -> anyhow::Result<u64>;

    /// Builds the blob, calling `progress(finished, total)` as it goes.
    fn derive(
        &self,
        inputs: &DerivedInputs,
        progress: &mut dyn FnMut(usize, usize),
    ) -> anyhow::Result<Vec<u8>>;

    /// Takes the blob into the asset's runtime part.
    fn apply_derived(&mut self, blob: &[u8]) -> anyhow::Result<()>;
}

static DERIVED_KINDS: RwLock<BTreeSet<&'static str>> = RwLock::new(BTreeSet::new());

pub(crate) fn note_derived_kinds<A: DerivedData>() {
    let mut kinds = DERIVED_KINDS.write().unwrap();
    kinds.insert(A::name());
    kinds.extend(A::FORMER_NAMES.iter().copied());
}

fn is_derived_kind(kind: &str) -> bool {
    DERIVED_KINDS.read().unwrap().contains(kind)
}

/// The files a derivation reads, each read whole from the content root and keyed by id.
#[derive(Debug, Default)]
pub struct DerivedInputs {
    files: HashMap<AssetId, Vec<u8>>,
}

impl DerivedInputs {
    /// Reads each of `ids` as a load would, failing on an id that is missing or names a derived asset.
    pub async fn read(
        root: &ContentAssetRoot,
        registry: &AssetRegistry,
        ids: &[AssetId],
    ) -> anyhow::Result<Self> {
        let mut files = HashMap::new();
        for &id in ids {
            if files.contains_key(&id) {
                continue;
            }
            let entry = registry
                .get(id)
                .with_context(|| format!("derived input {id:?} is not in the content registry"))?;
            if is_derived_kind(&entry.kind) {
                anyhow::bail!(
                    "derived input {id:?} at '{}' is a {}: derived assets can't derive from derived assets",
                    entry.address,
                    entry.kind
                );
            }
            let bytes = try_read_relative(root, &entry.address)
                .await?
                .with_context(|| {
                    format!(
                        "derived input {id:?} has no content asset at '{}'",
                        entry.address
                    )
                })?;
            files.insert(id, bytes);
        }
        Ok(Self { files })
    }

    /// The whole file of input `id`.
    pub fn bytes(&self, id: AssetId) -> Option<&[u8]> {
        self.files.get(&id).map(Vec::as_slice)
    }

    /// Decodes input `id` as a `T`, checking its kind and migrating it as a load does, without `on_load`.
    pub fn decode<T: Asset>(&self, id: AssetId) -> anyhow::Result<T> {
        let bytes = self
            .bytes(id)
            .with_context(|| format!("derived input {id:?} was not read"))?;
        let (_, value) = decode_content_asset::<T>(bytes)
            .with_context(|| format!("malformed derived input {id:?}"))?;
        Ok(value)
    }
}

/// Where derived blobs are kept, one file per type, version and key; or nowhere.
#[derive(Debug, Clone, Default)]
pub struct DerivedDataCache {
    root: Option<PathBuf>,
}

impl DerivedDataCache {
    /// A cache in `root`, created on the first `put`.
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self {
            root: Some(root.into()),
        }
    }

    /// No cache: every lookup misses and every `put` is dropped.
    pub fn none() -> Self {
        Self::default()
    }

    /// The cache directory, if there is one.
    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    fn entry(&self, type_name: &str, version: u32, key: u64) -> Option<(PathBuf, String)> {
        let dir = self
            .root
            .as_ref()?
            .join(type_name)
            .join(format!("v{version}"));
        Some((dir, format!("{key:016x}.blob")))
    }

    /// The blob cached for `type_name` at `version` under `key`; a read error is a miss.
    pub fn get(&self, type_name: &str, version: u32, key: u64) -> Option<Vec<u8>> {
        let (dir, name) = self.entry(type_name, version, key)?;
        let path = dir.join(name);
        match std::fs::read(&path) {
            Ok(blob) => Some(blob),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                log::warn!("failed to read derived data '{}': {error}", path.display());
                None
            }
        }
    }

    /// Stores `blob` for `type_name` at `version` under `key`, through a uniquely named temporary file; a failure is logged.
    pub fn put(&self, type_name: &str, version: u32, key: u64, blob: &[u8]) {
        let Some((dir, name)) = self.entry(type_name, version, key) else {
            return;
        };
        let path = dir.join(&name);
        let temp = dir.join(format!("{name}.{}.tmp", &AssetId::new().simple_hex()[..16]));
        let written = std::fs::create_dir_all(&dir)
            .and_then(|()| std::fs::write(&temp, blob))
            .and_then(|()| std::fs::rename(&temp, &path));
        if let Err(error) = written {
            let _ = std::fs::remove_file(&temp);
            log::warn!("failed to write derived data '{}': {error}", path.display());
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) type DerivedFuture =
    Pin<Box<dyn Future<Output = anyhow::Result<Box<dyn Any + Send>>> + Send>>;
#[cfg(target_arch = "wasm32")]
pub(crate) type DerivedFuture = Pin<Box<dyn Future<Output = anyhow::Result<Box<dyn Any + Send>>>>>;

pub(crate) type DerivedHook = fn(asset: Box<dyn Any + Send>, server: AssetServer) -> DerivedFuture;

pub(crate) fn derived_hook<A: DerivedData>(
    asset: Box<dyn Any + Send>,
    server: AssetServer,
) -> DerivedFuture {
    Box::pin(async move {
        let mut asset = *asset
            .downcast::<A>()
            .map_err(|_| anyhow::anyhow!("the derived hook for {} got another type", A::name()))?;
        let (root, registry) = server.content_source();
        let inputs = DerivedInputs::read(&root, &registry, &asset.derived_inputs()).await?;
        let key = asset.derived_key(&inputs)?;
        let cache = server.derived_cache();
        if let Some(blob) = cache.get(A::name(), A::DERIVED_VERSION, key) {
            match asset.apply_derived(&blob) {
                Ok(()) => return Ok(Box::new(asset) as Box<dyn Any + Send>),
                Err(error) => log::warn!(
                    "discarding the cached {} blob {key:016x}: {error:#}",
                    A::name()
                ),
            }
        }
        let blob = asset
            .derive(&inputs, &mut |_, _| {})
            .with_context(|| format!("failed to derive {}", A::name()))?;
        cache.put(A::name(), A::DERIVED_VERSION, key, &blob);
        asset
            .apply_derived(&blob)
            .with_context(|| format!("failed to apply the derived {}", A::name()))?;
        Ok(Box::new(asset) as Box<dyn Any + Send>)
    })
}
