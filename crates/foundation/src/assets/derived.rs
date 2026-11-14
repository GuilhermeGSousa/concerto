use std::{
    any::Any,
    collections::{BTreeMap, HashMap},
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{Arc, RwLock},
};

use anyhow::Context;

#[cfg(not(target_arch = "wasm32"))]
use super::{
    asset_server::AssetLoadContext,
    content::{write_cooked_content_asset, CookedSection},
};
use super::{
    asset_server::AssetServer,
    content::{
        asset_record, decode_content_asset, decode_content_payload, read_content_asset_parts,
        ContentAssetHeader, DerivedSection,
    },
    registry::{AssetEntry, AssetRegistry},
    utils::try_read_relative,
    Asset, AssetId, ContentAssetRoot, LoadableAsset,
};

/// An asset whose runtime part is a blob derived from other content assets, cached by a key it defines.
pub trait DerivedData: LoadableAsset {
    /// Bumped when `derive`'s output changes for the same inputs; part of the cache path.
    const DERIVED_VERSION: u32;

    /// Whether this asset has a blob to derive; one that doesn't loads as it is and is not cooked.
    fn is_derived(&self) -> bool {
        true
    }

    /// Whether the content asset with this header and payload derives; decodes it and asks [`is_derived`](Self::is_derived) unless overridden.
    fn file_is_derived(header: &ContentAssetHeader, payload: &[u8]) -> anyhow::Result<bool> {
        Ok(
            decode_content_payload::<Self>(&header.kind, header.kind_version, payload)?
                .is_derived(),
        )
    }

    /// The content assets the blob is built from.
    fn derived_inputs(&self) -> Vec<AssetId>;

    /// Further inputs named by the first inputs, such as the meshes a scene places.
    fn further_inputs(&self, _inputs: &DerivedInputs) -> anyhow::Result<Vec<AssetId>> {
        Ok(Vec::new())
    }

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

type FileIsDerived = fn(&ContentAssetHeader, &[u8]) -> anyhow::Result<bool>;

static DERIVED_KINDS: RwLock<BTreeMap<&'static str, FileIsDerived>> = RwLock::new(BTreeMap::new());

pub(crate) fn note_derived_kind<A: DerivedData>() {
    DERIVED_KINDS
        .write()
        .unwrap()
        .insert(A::name(), A::file_is_derived);
}

pub(crate) fn record_is_derived(kind: &str, record: &[u8]) -> anyhow::Result<bool> {
    let Some(check) = DERIVED_KINDS.read().unwrap().get(kind).copied() else {
        return Ok(false);
    };
    let parts = read_content_asset_parts(record)?;
    check(&parts.header, parts.payload)
}

/// The assets a derivation reads, keyed by id, with each file read once from the content root and kept.
#[derive(Debug, Default)]
pub struct DerivedInputs {
    files: HashMap<AssetId, Arc<Vec<u8>>>,
    records: HashMap<AssetId, Arc<Vec<u8>>>,
}

impl DerivedInputs {
    /// Reads each of `ids` as a load would, failing on an id that is missing or names an asset that itself derives.
    pub async fn read(
        root: &ContentAssetRoot,
        registry: &AssetRegistry,
        ids: &[AssetId],
    ) -> anyhow::Result<Self> {
        let mut inputs = Self::default();
        inputs.read_more(root, registry, ids).await?;
        Ok(inputs)
    }

    /// Reads each of `ids` not read yet, under the same rules as [`read`](Self::read).
    pub async fn read_more(
        &mut self,
        root: &ContentAssetRoot,
        registry: &AssetRegistry,
        ids: &[AssetId],
    ) -> anyhow::Result<()> {
        for &id in ids {
            if self.records.contains_key(&id) {
                continue;
            }
            let entry = registry
                .get(id)
                .with_context(|| format!("derived input {id:?} is not in the content registry"))?;
            let record = self.read_input(root, entry).await?;
            self.records.insert(id, record);
        }
        Ok(())
    }

    /// The whole file of a loose input `id`, or the bytes a loose file of a host or an embedded one would have.
    pub fn bytes(&self, id: AssetId) -> Option<&[u8]> {
        self.records.get(&id).map(|record| record.as_slice())
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

    async fn read_input(
        &mut self,
        root: &ContentAssetRoot,
        entry: &AssetEntry,
    ) -> anyhow::Result<Arc<Vec<u8>>> {
        let id = entry.id;
        let file = match self.files.get(&entry.file()) {
            Some(file) => Arc::clone(file),
            None => {
                let bytes = try_read_relative(root, &entry.address)
                    .await?
                    .with_context(|| {
                        format!(
                            "derived input {id:?} has no content asset at '{}'",
                            entry.address
                        )
                    })?;
                let file = Arc::new(bytes);
                self.files.insert(entry.file(), Arc::clone(&file));
                file
            }
        };
        let location = entry.location();
        let name = entry.embedded_in.as_ref().map(|held| held.name.as_str());
        let hosts = name.is_none()
            && read_content_asset_parts(&file).is_ok_and(|parts| !parts.header.embedded.is_empty());
        let record =
            if name.is_none() && !hosts {
                file
            } else {
                Arc::new(asset_record(&file, name).with_context(|| {
                    format!("failed to read derived input {id:?} at '{location}'")
                })?)
            };
        let derives = record_is_derived(&entry.kind, &record)
            .with_context(|| format!("failed to read derived input {id:?} at '{location}'"))?;
        if derives {
            anyhow::bail!(
                "derived input {id:?} at '{location}' is a {} that is itself derived: derived assets can't derive from derived assets",
                entry.kind
            );
        }
        Ok(record)
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

    /// The cache in `<project_root>/derived-cache`, created on the first `put`.
    pub fn in_project(project_root: &Path) -> Self {
        Self::at(project_root.join("derived-cache"))
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

#[cfg(not(target_arch = "wasm32"))]
pub(crate) type CookFuture = Pin<Box<dyn Future<Output = anyhow::Result<Option<Vec<u8>>>> + Send>>;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) type CookHook = fn(id: AssetId, file: Vec<u8>, server: AssetServer) -> CookFuture;

/// The addresses a cook wrote, in the order it wrote them.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CookReport {
    /// The content addresses written under the output directory.
    pub cooked: Vec<String>,
}

pub(crate) struct CookedBlob {
    pub(crate) section: DerivedSection,
    pub(crate) blob: Vec<u8>,
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) struct DerivedBlob {
    pub(crate) key: u64,
    pub(crate) blob: Vec<u8>,
}

pub(crate) type DerivedHook = fn(
    asset: Box<dyn Any + Send>,
    cooked: Option<CookedBlob>,
    server: AssetServer,
) -> DerivedFuture;

pub(crate) async fn derive_and_apply<A: DerivedData>(
    asset: &mut A,
    server: &AssetServer,
) -> anyhow::Result<DerivedBlob> {
    let (root, registry) = server.content_source();
    let mut inputs = DerivedInputs::read(&root, &registry, &asset.derived_inputs()).await?;
    let further = asset.further_inputs(&inputs)?;
    inputs.read_more(&root, &registry, &further).await?;
    let key = asset.derived_key(&inputs)?;
    let cache = server.derived_cache();
    if let Some(blob) = cache.get(A::name(), A::DERIVED_VERSION, key) {
        match asset.apply_derived(&blob) {
            Ok(()) => return Ok(DerivedBlob { key, blob }),
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
    Ok(DerivedBlob { key, blob })
}

fn apply_cooked<A: DerivedData>(asset: &mut A, cooked: CookedBlob) -> bool {
    let section = cooked.section;
    if section.derived_version != A::DERIVED_VERSION {
        log::warn!(
            "deriving {} again: its cooked blob is derived version {}, this build's is {}",
            A::name(),
            section.derived_version,
            A::DERIVED_VERSION
        );
        return false;
    }
    match asset.apply_derived(&cooked.blob) {
        Ok(()) => {
            log::debug!("applied the cooked {} blob {:016x}", A::name(), section.key);
            true
        }
        Err(error) => {
            log::warn!(
                "deriving {} again: its cooked blob {:016x} was refused: {error:#}",
                A::name(),
                section.key
            );
            false
        }
    }
}

pub(crate) fn derived_hook<A: DerivedData>(
    asset: Box<dyn Any + Send>,
    cooked: Option<CookedBlob>,
    server: AssetServer,
) -> DerivedFuture {
    Box::pin(async move {
        let mut asset = *asset
            .downcast::<A>()
            .map_err(|_| anyhow::anyhow!("the derived hook for {} got another type", A::name()))?;
        if !asset.is_derived() {
            if cooked.is_some() {
                log::warn!(
                    "ignoring the derived section of a {} that doesn't derive",
                    A::name()
                );
            }
            return Ok(Box::new(asset) as Box<dyn Any + Send>);
        }
        if let Some(cooked) = cooked {
            if apply_cooked(&mut asset, cooked) {
                return Ok(Box::new(asset) as Box<dyn Any + Send>);
            }
        }
        derive_and_apply(&mut asset, &server).await?;
        Ok(Box::new(asset) as Box<dyn Any + Send>)
    })
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn cook_hook<A: DerivedData>(
    id: AssetId,
    file: Vec<u8>,
    server: AssetServer,
) -> CookFuture {
    Box::pin(async move {
        let parts = read_content_asset_parts(&file)?;
        if !A::file_is_derived(&parts.header, parts.payload)? {
            return Ok(None);
        }
        let mut asset = decode_content_payload::<A>(
            &parts.header.kind,
            parts.header.kind_version,
            parts.payload,
        )?;
        asset.on_load(&AssetLoadContext::new(server.clone(), id))?;
        let derived = derive_and_apply(&mut asset, &server).await?;
        write_cooked_content_asset(
            &parts.header,
            parts.payload,
            CookedSection {
                derived_version: A::DERIVED_VERSION,
                key: derived.key,
                blob: &derived.blob,
            },
        )
        .map(Some)
    })
}
