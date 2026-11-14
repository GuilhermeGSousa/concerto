use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;

use concerto_asset_format::{
    asset_location, parse_registry, scan_content, sub_asset_file_stem, write_registry, ScannedEntry,
};
use concerto_ecs::resource::Resource;

use super::content::ImportProvenance;
use super::{Asset, AssetId};

/// One content asset on disk: its id, where it is, and what kind of asset it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetEntry {
    pub id: AssetId,
    /// The project-relative path, `/`-separated (`"content/hero/body.gasset"`).
    pub address: String,
    /// The `Asset::name()` of the type stored in the file.
    pub kind: String,
    /// The header's `kind_version`; `None` for every entry parsed from registry text.
    pub kind_version: Option<u32>,
    /// Where an imported asset came from; `None` for an authored asset, and for
    /// every entry parsed from registry text rather than from content headers.
    pub provenance: Option<ImportProvenance>,
    /// The asset's name inside the file at `address`; `None` for the host or a loose asset.
    pub embedded_as: Option<String>,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct SortKey<'a> {
    folder: &'a str,
    name: Cow<'a, str>,
    kind: &'a str,
    address: &'a str,
    embedded_as: Option<&'a str>,
}

impl AssetEntry {
    /// `address`, or `address#name` for an embedded asset.
    pub fn location(&self) -> String {
        asset_location(&self.address, self.embedded_as.as_deref())
    }

    /// The file name without its extension, or for an embedded asset the name its loose file would have.
    pub fn name(&self) -> Cow<'_, str> {
        if let Some(name) = &self.embedded_as {
            return Cow::Owned(sub_asset_file_stem(name, "gasset"));
        }
        Cow::Borrowed(
            Path::new(&self.address)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or(&self.address),
        )
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

    fn sort_key(&self) -> SortKey<'_> {
        SortKey {
            folder: self.folder(),
            name: self.name(),
            kind: &self.kind,
            address: &self.address,
            embedded_as: self.embedded_as.as_deref(),
        }
    }
}

impl From<ScannedEntry> for AssetEntry {
    fn from(entry: ScannedEntry) -> Self {
        Self {
            id: entry.id,
            address: entry.address,
            kind: entry.kind,
            kind_version: entry.kind_version,
            provenance: entry.provenance,
            embedded_as: entry.embedded_as,
        }
    }
}

/// Every content asset of a project or a shipped game: id, address, and kind.
#[derive(Debug, Clone, Default, Resource)]
pub struct AssetRegistry {
    entries: HashMap<AssetId, AssetEntry>,
    id_by_location: HashMap<String, AssetId>,
    order: Vec<AssetId>,
}

impl AssetRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn parse(text: &str) -> anyhow::Result<Self> {
        Ok(Self::from_entries(parse_registry(text)?))
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
        Ok(Self::from_entries(scan_content(
            project_root,
            &project_root.join(content_root),
            extension,
        )?))
    }

    fn from_entries(entries: Vec<ScannedEntry>) -> Self {
        let mut registry = Self::default();
        for entry in entries {
            registry.insert(entry.into());
        }
        registry
    }

    /// The registry's text: one `id = { address, kind }` line per asset, with `embedded_as` for an embedded one, ordered by id.
    pub fn to_toml(&self) -> String {
        let entries: Vec<ScannedEntry> = self
            .entries
            .values()
            .map(|entry| ScannedEntry {
                id: entry.id,
                address: entry.address.clone(),
                kind: entry.kind.clone(),
                kind_version: None,
                provenance: None,
                embedded_as: entry.embedded_as.clone(),
            })
            .collect();
        write_registry(&entries)
    }

    pub fn get(&self, id: AssetId) -> Option<&AssetEntry> {
        self.entries.get(&id)
    }

    /// The id of the host or loose asset at `address`. The inverse of looking up an entry's address.
    pub fn id_for_address(&self, address: &str) -> Option<AssetId> {
        self.id_for_location(address)
            .filter(|id| self.entries[id].embedded_as.is_none())
    }

    /// The asset at a location as [`AssetEntry::location`] writes it.
    pub fn id_for_location(&self, location: &str) -> Option<AssetId> {
        self.id_by_location.get(location).copied()
    }

    /// Every entry stored in the file at `address`, host first.
    pub fn in_file<'a>(&'a self, address: &'a str) -> impl Iterator<Item = &'a AssetEntry> {
        let host = self.id_for_address(address).map(|id| &self.entries[&id]);
        let embedded = self
            .iter()
            .filter(move |entry| entry.address == address && entry.embedded_as.is_some());
        host.into_iter().chain(embedded)
    }

    /// Every entry, ordered by folder, then name, then kind, then address.
    pub fn iter(&self) -> impl Iterator<Item = &AssetEntry> {
        self.order.iter().map(|id| &self.entries[id])
    }

    /// Every entry holding an `A`, in [`AssetRegistry::iter`] order.
    pub fn of_type<A: Asset>(&self) -> impl Iterator<Item = &AssetEntry> {
        self.of_kind(A::name())
    }

    /// The entries of the kind an [`Asset::name`] gives, for a caller that does not know the type.
    pub fn of_kind<'a>(&'a self, kind: &'a str) -> impl Iterator<Item = &'a AssetEntry> {
        self.iter().filter(move |entry| entry.kind == kind)
    }

    /// Adds `entry`, replacing any entry with the same id or at the same location.
    pub fn insert(&mut self, entry: AssetEntry) {
        self.remove(entry.id);
        let location = entry.location();
        if let Some(held) = self.id_for_location(&location) {
            self.remove(held);
        }
        let at = self
            .order
            .partition_point(|id| self.entries[id].sort_key() < entry.sort_key());
        self.order.insert(at, entry.id);
        self.id_by_location.insert(location, entry.id);
        self.entries.insert(entry.id, entry);
    }

    pub fn remove(&mut self, id: AssetId) -> Option<AssetEntry> {
        let entry = self.entries.remove(&id)?;
        self.id_by_location.remove(&entry.location());
        self.order.retain(|other| *other != id);
        Some(entry)
    }
}
