//! Framing for game-ready content assets: `magic | u32 header_len |
//! bincode(u32 format version, ContentAssetHeader) | payload (verbatim)`. The header is read
//! without touching the payload, so a future asset registry can index a
//! whole content tree by scanning headers alone.
use std::path::Path;

use anyhow::Context;

use super::registry::{AssetEntry, AssetRegistry};
use super::{Asset, AssetId};

pub use concerto_asset_format::{
    read_content_asset, read_content_asset_header, write_content_asset, ContentAssetHeader,
    ImportProvenance, CONTENT_ASSET_MAGIC, CONTENT_FORMAT_VERSION,
};

/// Decodes a content asset as `A`, checking its kind and type version and migrating an older version.
pub fn decode_content_asset<A: Asset>(bytes: &[u8]) -> anyhow::Result<(ContentAssetHeader, A)> {
    let (header, payload) = read_content_asset(bytes)?;
    if header.kind != A::name() {
        anyhow::bail!(
            "content asset holds a {} but a {} was requested",
            header.kind,
            A::name()
        );
    }
    let version = header.kind_version;
    let current = A::FORMAT_VERSION;
    let value = match version.cmp(&current) {
        std::cmp::Ordering::Equal => bincode::deserialize(payload)
            .with_context(|| format!("failed to deserialize {} payload", A::name()))?,
        std::cmp::Ordering::Less => A::migrate(version, payload).with_context(|| {
            format!(
                "{name} payload is version {version}; this build's {name} is version {current}",
                name = A::name()
            )
        })?,
        std::cmp::Ordering::Greater => anyhow::bail!(
            "{} payload is version {version}, newer than this build's version {current}",
            A::name()
        ),
    };
    Ok((header, value))
}

/// Writes `value` as a content asset at `project_root/address`, records it in `registry`, and returns its id, reusing the one already in the file's header.
pub fn save_content_asset_into<A: Asset>(
    value: &A,
    project_root: &Path,
    address: &str,
    registry: &mut AssetRegistry,
) -> anyhow::Result<AssetId> {
    let path = project_root.join(address);
    let asset_id = mint_or_reuse_id(&path)?;
    let header = ContentAssetHeader {
        asset_id,
        references: value.referenced_sub_assets(),
        kind: A::name().to_string(),
        kind_version: A::FORMAT_VERSION,
        provenance: None,
    };
    let kind = header.kind.clone();
    let payload = bincode::serialize(value).context("failed to serialize content asset payload")?;
    let bytes = write_content_asset(&header, &payload)?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create '{}'", parent.display()))?;
    }
    std::fs::write(&path, bytes)
        .with_context(|| format!("failed to write content asset '{}'", path.display()))?;

    registry.insert(AssetEntry {
        id: asset_id,
        address: address.to_owned(),
        kind,
        kind_version: Some(A::FORMAT_VERSION),
        provenance: None,
    });
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
