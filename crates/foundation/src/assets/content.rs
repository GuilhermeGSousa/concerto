//! Framing for game-ready content assets: `magic | u32 header_len |
//! bincode(ContentAssetHeader) | payload (verbatim)`. The header is read
//! without touching the payload, so a future asset registry can index a
//! whole content tree by scanning headers alone.
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};

use super::{Asset, AssetId};

pub use concerto_asset_format::{
    read_content_asset, read_content_asset_header, write_content_asset, ContentAssetHeader,
    ImportProvenance, CONTENT_ASSET_MAGIC, CONTENT_FORMAT_VERSION,
};

/// Where `AssetRegistry` lives, relative to the same root content-asset
/// addresses resolve against (a project root at import/save time, the
/// runtime `ContentAssetRoot` at load time).
pub const REGISTRY_FILE_NAME: &str = "content/.registry.toml";

/// Writes `value` as a content asset at `project_root/address`, creating
/// parent directories as needed, and upserts the asset registry so a
/// UUID-based (`AssetServer::load`) requests can find it later.
///
/// The asset's id is *minted* the first time an address is written and
/// reused every time after, by reading the header already on disk — so a
/// re-save keeps the identity that existing references name. `address` is
/// the project-relative path (`"content/hero/body.gasset"`); `project_root`
/// is the source tree an editor saves into, which is deliberately *not* the
/// exe-relative runtime root — a save must land in the tree under version
/// control, not beside the binary where the next build overwrites it.
pub fn save_content_asset<A: Asset>(
    value: &A,
    project_root: &Path,
    address: &str,
) -> anyhow::Result<()> {
    let mut registry = AssetRegistry::load(project_root)?;
    save_content_asset_into(value, project_root, address, &mut registry)?;
    registry.save(project_root)
}

/// Writes `value` at `project_root/address` as [`save_content_asset`] does, but records it in
/// `registry` instead of the registry file, and returns its id.
pub fn save_content_asset_into<A: Asset>(
    value: &A,
    project_root: &Path,
    address: &str,
    registry: &mut AssetRegistry,
) -> anyhow::Result<AssetId> {
    let path = project_root.join(address);
    let asset_id = mint_or_reuse_id(&path)?;
    let header = ContentAssetHeader {
        format_version: CONTENT_FORMAT_VERSION,
        asset_id,
        references: value.referenced_sub_assets(),
        kind: A::name().to_string(),
        provenance: None,
    };
    let payload = bincode::serialize(value).context("failed to serialize content asset payload")?;
    let bytes = write_content_asset(&header, &payload)?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create '{}'", parent.display()))?;
    }
    std::fs::write(&path, bytes)
        .with_context(|| format!("failed to write content asset '{}'", path.display()))?;

    registry.insert(asset_id, address);
    Ok(asset_id)
}

/// The id to write at `path`: the one already in the file's header if a
/// content asset is there, otherwise a freshly minted one. Identity is
/// assigned once and then belongs to the asset, not to its location.
pub fn mint_or_reuse_id(path: &Path) -> anyhow::Result<AssetId> {
    if path.exists() {
        return Ok(read_content_asset_header(path)?.asset_id);
    }
    Ok(AssetId::new())
}

/// A serialized `[assets]` table in `.registry.toml`: `AssetId::simple_hex()`
/// keys to content-tree address values.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct RegistryFile {
    #[serde(default)]
    assets: BTreeMap<String, String>,
}

/// Maps AssetIds to their content-tree addresses, with a reverse index for
/// importing and authoring tools. The runtime retains only the forward index.
/// `import` rebuilds it from content headers;
/// `save_content_asset` upserts individual entries.
#[derive(Debug, Clone, Default)]
pub struct AssetRegistry {
    entries: BTreeMap<AssetId, String>,
    id_by_address: HashMap<String, AssetId>,
}

impl AssetRegistry {
    /// Transfer the forward index to the runtime, discarding the tooling index.
    pub(crate) fn into_entries(self) -> BTreeMap<AssetId, String> {
        self.entries
    }

    pub fn new() -> Self {
        Self::default()
    }

    /// Loads `<project_root>/content/.registry.toml`, or an empty registry
    /// if it does not exist yet (a content tree with nothing imported or
    /// saved into it has no registry file).
    pub fn load(project_root: &Path) -> anyhow::Result<Self> {
        let path = project_root.join(REGISTRY_FILE_NAME);
        match std::fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(err).with_context(|| format!("failed to read '{}'", path.display())),
        }
    }

    pub fn parse(text: &str) -> anyhow::Result<Self> {
        let file: RegistryFile = toml::from_str(text).context("failed to parse asset registry")?;
        let mut registry = Self::default();
        for (hex, address) in file.assets {
            let id = AssetId::from_simple_hex(&hex)
                .map_err(|err| anyhow::anyhow!("invalid asset id '{hex}' in registry: {err}"))?;
            if let Some(existing) = registry.id_by_address.get(&address) {
                bail!(
                    "asset registry maps two ids ({} and {}) to the same address '{address}'",
                    existing.simple_hex(),
                    id.simple_hex()
                );
            }
            registry.insert(id, address);
        }
        Ok(registry)
    }

    /// Builds a registry by scanning `<project_root>/<content_root>` for
    /// `*.<extension>` files and reading each one's header.
    ///
    /// This is the authoritative way to produce a registry: identity lives in
    /// the header, so a scan re-points an id at wherever its file actually is
    /// now — which is what lets a content asset be renamed or moved without
    /// breaking the references that name its id. An absent content tree is an
    /// empty registry, not an error.
    pub fn from_content_tree(
        project_root: &Path,
        content_root: &str,
        extension: &str,
    ) -> anyhow::Result<Self> {
        let root = project_root.join(content_root);
        let mut registry = Self::default();
        let mut source_of: HashMap<AssetId, String> = HashMap::new();

        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            let entries = match std::fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                Err(err) => {
                    return Err(err).with_context(|| format!("failed to read '{}'", dir.display()))
                }
            };
            for entry in entries {
                let path = entry
                    .with_context(|| format!("failed to read an entry of '{}'", dir.display()))?
                    .path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) != Some(extension) {
                    continue;
                }

                let header = read_content_asset_header(&path)?;
                let address = path
                    .strip_prefix(project_root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");

                if let Some(previous) = source_of.get(&header.asset_id) {
                    bail!(
                        "content tree is malformed: '{previous}' and '{address}' both carry asset id {}",
                        header.asset_id.simple_hex()
                    );
                }
                source_of.insert(header.asset_id, address.clone());
                registry.insert(header.asset_id, address);
            }
        }

        Ok(registry)
    }

    pub fn save(&self, project_root: &Path) -> anyhow::Result<()> {
        let path = project_root.join(REGISTRY_FILE_NAME);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create '{}'", parent.display()))?;
        }
        let assets = self
            .entries
            .iter()
            .map(|(id, address)| (id.simple_hex(), address.clone()))
            .collect();
        let text = toml::to_string_pretty(&RegistryFile { assets })
            .context("failed to serialize asset registry")?;
        std::fs::write(&path, text).with_context(|| format!("failed to write '{}'", path.display()))
    }

    pub fn get(&self, id: AssetId) -> Option<&str> {
        self.entries.get(&id).map(String::as_str)
    }

    pub fn insert(&mut self, id: AssetId, address: impl Into<String>) {
        let address = address.into();
        if let Some(previous) = self.entries.insert(id, address.clone()) {
            self.id_by_address.remove(&previous);
        }
        self.id_by_address.insert(address, id);
    }

    pub fn remove(&mut self, id: AssetId) -> Option<String> {
        let address = self.entries.remove(&id)?;
        self.id_by_address.remove(&address);
        Some(address)
    }

    /// The id of the asset at `address`, for importing and authoring. The inverse
    /// of [`AssetRegistry::get`].
    pub fn id_for_address(&self, address: &str) -> Option<AssetId> {
        self.id_by_address.get(address).copied()
    }

    pub fn iter(&self) -> impl Iterator<Item = (AssetId, &str)> {
        self.entries
            .iter()
            .map(|(id, address)| (*id, address.as_str()))
    }
}
