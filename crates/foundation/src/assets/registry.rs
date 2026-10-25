use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use anyhow::{bail, Context};
use concerto_ecs::resource::Resource;
use serde::{Deserialize, Serialize};

use super::content::{read_content_asset_header, ImportProvenance};
use super::{Asset, AssetId};

/// Where `AssetRegistry` lives, relative to the same root content-asset
/// addresses resolve against (a project root at import/save time, the
/// runtime `ContentAssetRoot` at load time).
pub const REGISTRY_FILE_NAME: &str = "content/.registry.toml";

/// One content asset on disk: its id, where it is, and what kind of asset it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetEntry {
    pub id: AssetId,
    /// The project-relative path, `/`-separated (`"content/hero/body.gasset"`).
    pub address: String,
    /// The `Asset::name()` of the type stored in the file.
    pub kind: String,
    /// Where an imported asset came from; `None` for an authored asset, and for
    /// every entry read from the registry file rather than from content headers.
    pub provenance: Option<ImportProvenance>,
}

impl AssetEntry {
    /// The file name without its extension.
    pub fn name(&self) -> &str {
        Path::new(&self.address)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or(&self.address)
    }

    /// The folder holding the file, empty for an address with no folder.
    pub fn folder(&self) -> &str {
        self.address
            .rsplit_once('/')
            .map_or("", |(folder, _)| folder)
    }

    /// Whether the file holds an `A`.
    pub fn is<A: Asset>(&self) -> bool {
        self.kind == A::name()
    }

    fn sort_key(&self) -> (&str, &str, &str, &str) {
        (self.folder(), self.name(), &self.kind, &self.address)
    }
}

#[derive(Serialize, Deserialize)]
struct RegistryRecord {
    address: String,
    kind: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum StoredRecord {
    Record(RegistryRecord),
    Legacy(String),
}

#[derive(Deserialize)]
struct RegistryFile {
    #[serde(default)]
    assets: BTreeMap<String, StoredRecord>,
}

/// Every content asset of a project or a shipped game: id, address, and kind.
#[derive(Debug, Clone, Default, Resource)]
pub struct AssetRegistry {
    entries: HashMap<AssetId, AssetEntry>,
    id_by_address: HashMap<String, AssetId>,
    order: Vec<AssetId>,
}

impl AssetRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Loads `<project_root>/content/.registry.toml`, or an empty registry
    /// if it does not exist yet (a content tree with nothing imported or
    /// saved into it has no registry file).
    pub fn load(project_root: &Path) -> anyhow::Result<Self> {
        let path = project_root.join(REGISTRY_FILE_NAME);
        match std::fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text).with_context(|| format!("in '{}'", path.display())),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(err).with_context(|| format!("failed to read '{}'", path.display())),
        }
    }

    pub fn parse(text: &str) -> anyhow::Result<Self> {
        let file: RegistryFile = toml::from_str(text).context("failed to parse asset registry")?;
        let mut registry = Self::default();
        for (hex, record) in file.assets {
            let id = AssetId::from_simple_hex(&hex)
                .map_err(|err| anyhow::anyhow!("invalid asset id '{hex}' in registry: {err}"))?;
            let record = match record {
                StoredRecord::Record(record) => record,
                StoredRecord::Legacy(address) => bail!(
                    "asset registry predates asset kinds ('{address}' has none); rebuild it by importing a source or saving any asset from the editor"
                ),
            };
            if let Some(existing) = registry.id_by_address.get(&record.address) {
                bail!(
                    "asset registry maps two ids ({} and {}) to the same address '{}'",
                    existing.simple_hex(),
                    id.simple_hex(),
                    record.address
                );
            }
            registry.insert(AssetEntry {
                id,
                address: record.address,
                kind: record.kind,
                provenance: None,
            });
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

                if let Some(previous) = registry.get(header.asset_id) {
                    bail!(
                        "content tree is malformed: '{}' and '{address}' both carry asset id {}",
                        previous.address,
                        header.asset_id.simple_hex()
                    );
                }
                registry.insert(AssetEntry {
                    id: header.asset_id,
                    address,
                    kind: header.kind,
                    provenance: header.provenance,
                });
            }
        }

        Ok(registry)
    }

    /// The registry file's text: one `id = { address, kind }` line per asset, ordered by id.
    pub fn to_toml(&self) -> String {
        let mut ids: Vec<_> = self.entries.keys().copied().collect();
        ids.sort();
        let mut text = String::from("[assets]\n");
        for id in ids {
            let entry = &self.entries[&id];
            text.push_str(&format!(
                "{} = {{ address = {}, kind = {} }}\n",
                id.simple_hex(),
                toml::Value::String(entry.address.clone()),
                toml::Value::String(entry.kind.clone()),
            ));
        }
        text
    }

    pub fn save(&self, project_root: &Path) -> anyhow::Result<()> {
        let path = project_root.join(REGISTRY_FILE_NAME);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create '{}'", parent.display()))?;
        }
        std::fs::write(&path, self.to_toml())
            .with_context(|| format!("failed to write '{}'", path.display()))
    }

    pub fn get(&self, id: AssetId) -> Option<&AssetEntry> {
        self.entries.get(&id)
    }

    /// The id of the asset at `address`. The inverse of looking up an entry's address.
    pub fn id_for_address(&self, address: &str) -> Option<AssetId> {
        self.id_by_address.get(address).copied()
    }

    /// Every entry, ordered by folder, then name, then kind, then address.
    pub fn iter(&self) -> impl Iterator<Item = &AssetEntry> {
        self.order.iter().map(|id| &self.entries[id])
    }

    /// Every entry holding an `A`, in [`AssetRegistry::iter`] order.
    pub fn of_type<A: Asset>(&self) -> impl Iterator<Item = &AssetEntry> {
        self.iter().filter(|entry| entry.is::<A>())
    }

    /// Adds `entry`, replacing any entry with the same id.
    pub fn insert(&mut self, entry: AssetEntry) {
        self.remove(entry.id);
        let at = self
            .order
            .partition_point(|id| self.entries[id].sort_key() < entry.sort_key());
        self.order.insert(at, entry.id);
        self.id_by_address.insert(entry.address.clone(), entry.id);
        self.entries.insert(entry.id, entry);
    }

    pub fn remove(&mut self, id: AssetId) -> Option<AssetEntry> {
        let entry = self.entries.remove(&id)?;
        self.id_by_address.remove(&entry.address);
        self.order.retain(|other| *other != id);
        Some(entry)
    }
}
