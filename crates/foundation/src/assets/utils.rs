use anyhow::Context;
use cfg_if::cfg_if;

use super::{
    content::{asset_record, decode_content_payload, read_content_asset_parts},
    derived::CookedBlob,
    file_reads::SharedRead,
    registry::AssetEntry,
    Asset, ContentAssetRoot,
};

/// Loads the content asset at `<root>/<address>` as `A`, through [`decode_content_asset`](crate::assets::content::decode_content_asset).
pub async fn load_content_asset<A: Asset>(
    root: &ContentAssetRoot,
    address: &str,
) -> anyhow::Result<A> {
    Ok(load_content_asset_parts::<A>(root, address).await?.asset)
}

pub(crate) struct LoadedContent<A> {
    pub(crate) asset: A,
    pub(crate) cooked: Option<CookedBlob>,
}

pub(crate) async fn load_content_asset_parts<A: Asset>(
    root: &ContentAssetRoot,
    address: &str,
) -> anyhow::Result<LoadedContent<A>> {
    let raw = try_read_relative(root, address)
        .await?
        .with_context(|| format!("no content asset at '{address}'"))?;
    decode_content_parts(&raw).with_context(|| format!("malformed content asset '{address}'"))
}

pub(crate) async fn load_entry_parts<A: Asset>(
    root: &ContentAssetRoot,
    entry: &AssetEntry,
    read: &SharedRead,
) -> anyhow::Result<LoadedContent<A>> {
    let file = read.bytes(root, &entry.address).await?;
    let decoded = match entry.embedded_as.as_deref() {
        None => decode_content_parts(&file),
        Some(name) => {
            asset_record(&file, Some(name)).and_then(|record| decode_content_parts(&record))
        }
    };
    decoded.with_context(|| format!("malformed content asset '{}'", entry.location()))
}

fn decode_content_parts<A: Asset>(raw: &[u8]) -> anyhow::Result<LoadedContent<A>> {
    let parts = read_content_asset_parts(raw)?;
    let asset = decode_content_payload::<A>(&parts.header, parts.payload)?;
    let cooked = parts
        .header
        .derived
        .zip(parts.derived_blob)
        .map(|(section, blob)| CookedBlob {
            section,
            blob: blob.to_vec(),
        });
    Ok(LoadedContent { asset, cooked })
}

/// Reads `<root>/<relative>`, returning `Ok(None)` when it simply is not
/// there (a missing file natively, a non-success status on wasm) so the
/// caller can turn absence into its own error message. Any other failure
/// is a real error.
pub(crate) async fn try_read_relative(
    root: &ContentAssetRoot,
    relative: &str,
) -> anyhow::Result<Option<Vec<u8>>> {
    match root {
        ContentAssetRoot::Directory(dir) => {
            let path = dir.join(relative);
            cfg_if! {
                if #[cfg(target_arch = "wasm32")] {
                    let _ = path;
                    Ok(None)
                } else {
                    match async_fs::read(&path).await {
                        Ok(bytes) => Ok(Some(bytes)),
                        Err(err)
                            if matches!(
                                err.kind(),
                                std::io::ErrorKind::NotFound
                                    | std::io::ErrorKind::IsADirectory
                                    | std::io::ErrorKind::NotADirectory
                                    | std::io::ErrorKind::InvalidInput
                            ) =>
                        {
                            Ok(None)
                        }
                        Err(err) => Err(anyhow::Error::new(err))
                            .with_context(|| format!("failed to read '{}'", path.display())),
                    }
                }
            }
        }
        ContentAssetRoot::UrlBase(base) => {
            cfg_if! {
                if #[cfg(target_arch = "wasm32")] {
                    let url = format!("{base}/{relative}");
                    let response = reqwest::get(&url)
                        .await
                        .with_context(|| format!("HTTP request for '{url}' failed"))?;
                    if !response.status().is_success() {
                        return Ok(None);
                    }
                    let bytes = response
                        .bytes()
                        .await
                        .with_context(|| format!("failed to read response body for '{url}'"))?;
                    Ok(Some(bytes.to_vec()))
                } else {
                    let _ = relative;
                    anyhow::bail!(
                        "ContentAssetRoot::UrlBase is only supported on wasm32 (base '{base}')"
                    )
                }
            }
        }
    }
}
