//! Framing for game-ready content assets: `magic | u32 header_len |
//! bincode(ContentAssetHeader) | payload (verbatim)`. The header is read
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
        format_version: CONTENT_FORMAT_VERSION,
        asset_id,
        references: value.referenced_sub_assets(),
        kind: A::name().to_string(),
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
