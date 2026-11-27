use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;

pub use concerto_asset_format::EmbeddedIn;
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
    /// Where an imported asset came from.
    ///
    /// `None` for an authored asset, and for every entry parsed from registry
    /// text rather than from content headers.
    pub provenance: Option<ImportProvenance>,
    /// The host whose file at `address` holds the asset, and its name there.
    ///
    /// `None` for the host itself or a loose asset.
    pub embedded_in: Option<EmbeddedIn>,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct SortKey<'a> {
    folder: &'a str,
    name: Cow<'a, str>,
    kind: &'a str,
    address: &'a str,
    embedded_name: Option<&'a str>,
}

impl AssetEntry {
    /// `address`, or `address#name` for an embedded asset.
    pub fn location(&self) -> String {
        asset_location(&self.address, self.embedded_name())
    }

    /// The asset whose file holds this one: the host of an embedded asset, the asset itself otherwise.
    pub fn file(&self) -> AssetId {
        self.embedded_in.as_ref().map_or(self.id, |held| held.host)
    }

    fn embedded_name(&self) -> Option<&str> {
        self.embedded_in.as_ref().map(|held| held.name.as_str())
    }

    /// The file name without its extension, or for an embedded asset the name its loose file would have.
    pub fn name(&self) -> Cow<'_, str> {
        if let Some(name) = self.embedded_name() {
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
            embedded_name: self.embedded_name(),
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
            embedded_in: entry.embedded_in,
        }
    }
}

/// Every content asset of a project or a shipped game: id, address, and kind.
#[derive(Debug, Clone, Default, Resource)]
pub struct AssetRegistry {
    entries: HashMap<AssetId, AssetEntry>,
    id_by_address: HashMap<String, AssetId>,
    embedded_by_host: HashMap<AssetId, Vec<AssetId>>,
    order: Vec<AssetId>,
}

impl AssetRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads a registry from the text [`write_registry`] writes.
    ///
    /// # Errors
    ///
    /// Returns an error when the text is not a registry that [`parse_registry`] accepts: invalid
    /// TOML, an invalid id, a line without a kind, two ids at one location, or an asset embedded at
    /// an address that holds no host.
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        Ok(Self::from_entries(parse_registry(text)?))
    }

    /// Builds a registry by scanning `<project_root>/<content_root>` for
    /// `*.<extension>` files and reading each one's header.
    ///
    /// This is the authoritative way to produce a registry. Identity lives in
    /// the header, so a scan re-points an id at wherever its file is now,
    /// which lets a content asset be renamed or moved without breaking the
    /// references that name its id. An absent content tree is an empty
    /// registry, not an error.
    ///
    /// # Errors
    ///
    /// Returns an error when the tree cannot be scanned: a file cannot be read
    /// or its header does not parse.
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

    /// The registry's text.
    ///
    /// One `id = { address, kind }` line per asset, with `embedded_as` for an
    /// embedded one, ordered by id.
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
                embedded_in: entry.embedded_in.clone(),
            })
            .collect();
        write_registry(&entries)
    }

    pub fn get(&self, id: AssetId) -> Option<&AssetEntry> {
        self.entries.get(&id)
    }

    /// The id of the host or loose asset at `address`. The inverse of looking up an entry's address.
    pub fn id_for_address(&self, address: &str) -> Option<AssetId> {
        self.id_by_address.get(address).copied()
    }

    /// The asset at a location as [`AssetEntry::location`] writes it.
    pub fn id_for_location(&self, location: &str) -> Option<AssetId> {
        if let Some(id) = self.id_for_address(location) {
            return Some(id);
        }
        for (at, _) in location.match_indices('#') {
            if let Some(host) = self.id_for_address(&location[..at]) {
                return self.embedded_named(host, &location[at + 1..]);
            }
        }
        None
    }

    /// Every entry stored in the file of the host or loose asset `file`, that asset first.
    pub fn in_file(&self, file: AssetId) -> impl Iterator<Item = &AssetEntry> {
        let host = self
            .entries
            .get(&file)
            .filter(|entry| entry.embedded_in.is_none());
        let embedded = self
            .embedded_by_host
            .get(&file)
            .into_iter()
            .flatten()
            .map(|id| &self.entries[id]);
        host.into_iter().chain(embedded)
    }

    fn embedded_named(&self, host: AssetId, name: &str) -> Option<AssetId> {
        self.embedded_by_host
            .get(&host)?
            .iter()
            .copied()
            .find(|id| self.entries[id].embedded_name() == Some(name))
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

    /// Adds `entry`, replacing any entry with the same id, at the same address, or of the same name in the same host.
    pub fn insert(&mut self, entry: AssetEntry) {
        self.detach(entry.id);
        let held = match &entry.embedded_in {
            Some(embedded) => self.embedded_named(embedded.host, &embedded.name),
            None => self.id_for_address(&entry.address),
        };
        if let Some(held) = held {
            self.remove(held);
        }
        let at = self
            .order
            .partition_point(|id| self.entries[id].sort_key() < entry.sort_key());
        self.order.insert(at, entry.id);
        match &entry.embedded_in {
            Some(embedded) => {
                let held = self.embedded_by_host.entry(embedded.host).or_default();
                let at = held.partition_point(|id| self.entries[id].sort_key() < entry.sort_key());
                held.insert(at, entry.id);
            }
            None => {
                self.id_by_address.insert(entry.address.clone(), entry.id);
            }
        }
        self.entries.insert(entry.id, entry);
    }

    /// Removes the entry of `id`, and with a host every entry embedded in its file.
    pub fn remove(&mut self, id: AssetId) -> Option<AssetEntry> {
        let entry = self.detach(id)?;
        for embedded in self.embedded_by_host.remove(&id).unwrap_or_default() {
            self.entries.remove(&embedded);
            self.order.retain(|other| *other != embedded);
        }
        Some(entry)
    }

    fn detach(&mut self, id: AssetId) -> Option<AssetEntry> {
        let entry = self.entries.remove(&id)?;
        match &entry.embedded_in {
            Some(embedded) => {
                if let Some(held) = self.embedded_by_host.get_mut(&embedded.host) {
                    held.retain(|other| *other != id);
                    if held.is_empty() {
                        self.embedded_by_host.remove(&embedded.host);
                    }
                }
            }
            None => {
                self.id_by_address.remove(&entry.address);
            }
        }
        self.order.retain(|other| *other != id);
        Some(entry)
    }
}
