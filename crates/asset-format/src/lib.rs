//! Shared persistent asset identity and binary framing, independent of the engine.
use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};
use std::path::Path;
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
    /// ids read back from `.registry.toml`.
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
