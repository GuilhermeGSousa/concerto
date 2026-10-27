//! Shared persistent asset identity and binary framing, independent of the engine.
use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct AssetId(Uuid);

/// Fixed namespace for deriving AssetIds from asset paths, so the same
/// path string always hashes to the same UUID (v5) regardless of process
/// or machine. Generated once via `uuid::Uuid::new_v4()` and hard-coded —
/// it must never change once assets have been addressed with it.
const ASSET_PATH_NAMESPACE: Uuid = Uuid::from_bytes([
    0x6d, 0x1a, 0x9a, 0x3e, 0x2f, 0x0b, 0x4a, 0x77, 0x8e, 0x92, 0x1a, 0x64, 0xaf, 0x03, 0x5c, 0x11,
]);

impl AssetId {
    /// Constructs an identity from its UUID bytes, including in constant expressions.
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(Uuid::from_bytes(bytes))
    }

    /// The UUID bytes embedded by compile-time asset references.
    pub const fn to_bytes(self) -> [u8; 16] {
        *self.0.as_bytes()
    }

    pub fn new() -> Self {
        AssetId(Uuid::new_v4())
    }

    /// The id that names no asset.
    pub const fn nil() -> Self {
        AssetId(Uuid::nil())
    }

    pub fn is_nil(&self) -> bool {
        self.0.is_nil()
    }

    /// Derives a deterministic UUID for legacy content and callers that need
    /// a namespaced path hash. Imported asset identity is minted and persisted
    /// separately; runtime content loading must resolve it through the registry.
    pub fn from_path(path: &str) -> Self {
        AssetId(Uuid::new_v5(&ASSET_PATH_NAMESPACE, path.as_bytes()))
    }

    /// The AssetId's underlying UUID as 32 lowercase hex digits, no hyphens —
    /// used as the `AssetRegistry` key (a text-friendly, collision-free pure
    /// function of the ID).
    pub fn simple_hex(&self) -> String {
        self.0.simple().to_string()
    }

    /// Inverse of `simple_hex()` — parses an AssetId back from its
    /// 32-lowercase-hex-digit form. Used by `AssetRegistry` to reconstruct
    /// ids read back from registry TOML.
    pub fn from_simple_hex(hex: &str) -> Result<Self, uuid::Error> {
        Uuid::parse_str(hex).map(AssetId)
    }
}

impl Default for AssetId {
    fn default() -> Self {
        Self::new()
    }
}

/// Leading bytes of every content asset file.
pub const CONTENT_ASSET_MAGIC: [u8; 4] = *b"GRDY";

/// Bumped whenever `ContentAssetHeader`'s on-disk shape changes
/// incompatibly. `read_content_asset` rejects a mismatch outright.
pub const CONTENT_FORMAT_VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentAssetHeader {
    pub format_version: u32,
    /// Identity: a UUID minted when the asset is first written and carried
    /// here from then on, so it survives the file being renamed or moved.
    /// Deliberately not derived from the address — only the asset registry
    /// connects an address back to the id.
    pub asset_id: AssetId,
    /// Outbound references, so a registry scan never reads payloads.
    pub references: Vec<AssetId>,
    /// Authoritative type tag; must equal the loading type's `Asset::name()`.
    pub kind: String,
    /// Where this content asset came from, if `import` produced it from a
    /// DCC source rather than an editor saving it directly.
    pub provenance: Option<ImportProvenance>,
}

/// The offline source (and sub-asset within it) that `import` produced a
/// content asset from — lets a future editor show "re-import" provenance
/// without re-deriving it from the address string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportProvenance {
    /// Project-relative source path, or an absolute path for external sources.
    /// Older assets can retain the path spelling used by the original importer.
    pub source: String,
    /// The sub-asset name within that source, e.g. `"mesh/0"`.
    pub sub_asset: String,
}

pub fn write_content_asset(header: &ContentAssetHeader, payload: &[u8]) -> anyhow::Result<Vec<u8>> {
    let header_bytes =
        bincode::serialize(header).context("failed to serialize content asset header")?;

    let header_len =
        u32::try_from(header_bytes.len()).context("content asset header exceeds 4 GiB")?;

    let mut out = Vec::with_capacity(8 + header_bytes.len() + payload.len());
    out.extend_from_slice(&CONTENT_ASSET_MAGIC);
    out.extend_from_slice(&header_len.to_le_bytes());
    out.extend_from_slice(&header_bytes);
    out.extend_from_slice(payload);
    Ok(out)
}

pub fn read_content_asset(bytes: &[u8]) -> anyhow::Result<(ContentAssetHeader, &[u8])> {
    if bytes.len() < 8 || bytes[..4] != CONTENT_ASSET_MAGIC {
        bail!("not a content asset: missing the GRDY magic prefix");
    }

    let header_len = u32::from_le_bytes(
        bytes[4..8]
            .try_into()
            .expect("slice of exactly 4 bytes is always a [u8; 4]"),
    ) as usize;
    let header_end = 8usize
        .checked_add(header_len)
        .context("content asset header length overflows")?;
    if bytes.len() < header_end {
        bail!(
            "content asset truncated: header claims {header_len} bytes, only {} available",
            bytes.len() - 8
        );
    }

    let header: ContentAssetHeader = bincode::deserialize(&bytes[8..header_end])
        .context("failed to deserialize content asset header")?;
    if header.format_version != CONTENT_FORMAT_VERSION {
        bail!(
            "unsupported content asset format version {} (this build expects {CONTENT_FORMAT_VERSION})",
            header.format_version
        );
    }
    Ok((header, &bytes[header_end..]))
}

/// Reads just the header of the content asset at `path`, never the payload.
///
/// A content tree holds whole textures and meshes — tens of megabytes each —
/// so indexing one by reading every file whole is not viable. This reads the
/// 8-byte magic-and-length prefix, then exactly `header_len` more bytes.
pub fn read_content_asset_header(path: &Path) -> anyhow::Result<ContentAssetHeader> {
    use std::io::Read;

    let mut file = std::fs::File::open(path)
        .with_context(|| format!("failed to open '{}'", path.display()))?;

    let mut prefix = [0u8; 8];
    file.read_exact(&mut prefix)
        .with_context(|| format!("'{}' is too short to be a content asset", path.display()))?;
    if prefix[..4] != CONTENT_ASSET_MAGIC {
        bail!(
            "not a content asset: '{}' is missing the GRDY magic prefix",
            path.display()
        );
    }

    let header_len = u32::from_le_bytes(
        prefix[4..8]
            .try_into()
            .expect("slice of exactly 4 bytes is always a [u8; 4]"),
    ) as usize;

    let mut header_bytes = vec![0u8; header_len];
    file.read_exact(&mut header_bytes).with_context(|| {
        format!(
            "content asset '{}' truncated: header claims {header_len} bytes",
            path.display()
        )
    })?;

    let header: ContentAssetHeader = bincode::deserialize(&header_bytes)
        .with_context(|| format!("failed to deserialize header of '{}'", path.display()))?;
    if header.format_version != CONTENT_FORMAT_VERSION {
        bail!(
            "unsupported content asset format version {} in '{}' (this build expects {CONTENT_FORMAT_VERSION})",
            header.format_version,
            path.display()
        );
    }
    Ok(header)
}

/// One content asset: what a scan reads from its header, or a registry line holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedEntry {
    pub id: AssetId,
    /// The path relative to the scan's base, `/`-separated (`"content/hero/body.gasset"`).
    pub address: String,
    pub kind: String,
    /// Where an imported asset came from; `None` for an authored asset and for a registry line.
    pub provenance: Option<ImportProvenance>,
}

fn normalise_lexically(path: &Path) -> PathBuf {
    use std::path::Component;

    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match out.components().next_back() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                Some(Component::RootDir | Component::Prefix(_)) => {}
                _ => out.push(".."),
            },
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Lists every `*.<extension>` file under `dir` by its header, addressed relative to `base`.
pub fn scan_content(base: &Path, dir: &Path, extension: &str) -> anyhow::Result<Vec<ScannedEntry>> {
    let base = normalise_lexically(base);
    let dir = normalise_lexically(dir);

    let mut entries = Vec::new();
    let mut index_by_id: HashMap<AssetId, usize> = HashMap::new();
    let mut visited: HashSet<PathBuf> = HashSet::new();
    let mut stack = vec![dir];
    while let Some(path) = stack.pop() {
        if path.is_dir() {
            let canonical = match path.canonicalize() {
                Ok(canonical) => canonical,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                Err(err) => {
                    return Err(err)
                        .with_context(|| format!("failed to resolve '{}'", path.display()))
                }
            };
            if !visited.insert(canonical) {
                continue;
            }
            let listing = match std::fs::read_dir(&path) {
                Ok(listing) => listing,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                Err(err) => {
                    return Err(err).with_context(|| format!("failed to read '{}'", path.display()))
                }
            };
            let mut children = Vec::new();
            for entry in listing {
                children.push(
                    entry
                        .with_context(|| {
                            format!("failed to read an entry of '{}'", path.display())
                        })?
                        .path(),
                );
            }
            children.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
            stack.extend(children.into_iter().rev());
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some(extension) {
            continue;
        }

        let header = read_content_asset_header(&path)?;
        let address = path
            .strip_prefix(&base)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");

        if let Some(&previous) = index_by_id.get(&header.asset_id) {
            let previous: &ScannedEntry = &entries[previous];
            bail!(
                "content tree is malformed: '{}' and '{address}' both carry asset id {}",
                previous.address,
                header.asset_id.simple_hex()
            );
        }
        index_by_id.insert(header.asset_id, entries.len());
        entries.push(ScannedEntry {
            id: header.asset_id,
            address,
            kind: header.kind,
            provenance: header.provenance,
        });
    }

    Ok(entries)
}

#[derive(Deserialize)]
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

/// The registry text: one `id = { address, kind }` line per entry, ordered by id.
pub fn write_registry(entries: &[ScannedEntry]) -> String {
    let mut ordered: Vec<&ScannedEntry> = entries.iter().collect();
    ordered.sort_by_key(|entry| entry.id);
    let mut text = String::from("[assets]\n");
    for entry in ordered {
        text.push_str(&format!(
            "{} = {{ address = {}, kind = {} }}\n",
            entry.id.simple_hex(),
            toml::Value::String(entry.address.clone()),
            toml::Value::String(entry.kind.clone()),
        ));
    }
    text
}

/// Reads registry text written by [`write_registry`], ordered by id.
pub fn parse_registry(text: &str) -> anyhow::Result<Vec<ScannedEntry>> {
    let file: RegistryFile = toml::from_str(text).context("failed to parse asset registry")?;
    let mut entries = Vec::with_capacity(file.assets.len());
    let mut id_by_address: HashMap<String, AssetId> = HashMap::new();
    for (hex, record) in file.assets {
        let id = AssetId::from_simple_hex(&hex)
            .map_err(|err| anyhow::anyhow!("invalid asset id '{hex}' in registry: {err}"))?;
        let record = match record {
            StoredRecord::Record(record) => record,
            StoredRecord::Legacy(address) => bail!(
                "asset registry predates asset kinds ('{address}' has none); rebuild the game so its build script writes a new one"
            ),
        };
        if let Some(existing) = id_by_address.insert(record.address.clone(), id) {
            bail!(
                "asset registry maps two ids ({} and {}) to the same address '{}'",
                existing.simple_hex(),
                id.simple_hex(),
                record.address
            );
        }
        entries.push(ScannedEntry {
            id,
            address: record.address,
            kind: record.kind,
            provenance: None,
        });
    }
    Ok(entries)
}

/// `content.toml` at a project root, read by the import, the editor's scan and `track_assets`, never at runtime.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ContentConfig {
    /// Uniform extension for every content asset, without the dot.
    pub extension: String,
    /// Content tree root, relative to the project root.
    pub root: String,
}

impl Default for ContentConfig {
    fn default() -> Self {
        Self {
            extension: "gasset".to_string(),
            root: "content".to_string(),
        }
    }
}

impl ContentConfig {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read '{}'", path.display()))?;
        toml::from_str(&text).with_context(|| format!("failed to parse '{}'", path.display()))
    }

    /// Loads `content.toml` from `dir` if present, else the defaults.
    pub fn load_or_default(dir: &Path) -> anyhow::Result<Self> {
        let path = dir.join("content.toml");
        if path.exists() {
            Self::load(&path)
        } else {
            Ok(Self::default())
        }
    }
}
