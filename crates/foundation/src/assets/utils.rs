use anyhow::Context;
use cfg_if::cfg_if;

use super::{
    content::{decode_content_payload, read_content_asset_parts},
    derived::CookedBlob,
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

    let decoded = read_content_asset_parts(&raw).and_then(|parts| {
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
    });
    decoded.with_context(|| format!("malformed content asset '{address}'"))
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
