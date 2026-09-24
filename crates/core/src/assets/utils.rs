use anyhow::Context;
use cfg_if::cfg_if;

use super::content::AssetRegistry;
use super::ContentAssetRoot;

/// Reads an asset's payload from the content asset at `<root>/<address>`.
pub async fn load_content_asset_bytes(
    root: &ContentAssetRoot,
    address: &str,
    expected_kind: &str,
) -> anyhow::Result<Vec<u8>> {
    let raw = try_read_relative(root, address)
        .await?
        .with_context(|| format!("no content asset at '{address}'"))?;

    let (header, payload) = crate::assets::content::read_content_asset(&raw)
        .with_context(|| format!("malformed content asset '{address}'"))?;
    if header.kind != expected_kind {
        anyhow::bail!(
            "content asset '{address}' holds a {} but a {expected_kind} was requested",
            header.kind
        );
    }
    Ok(payload.to_vec())
}

/// Reads and parses `.registry.toml` from `root`, for `AssetServer`'s
/// UUID-to-address resolution. An absent registry (nothing has
/// ever been imported or saved into this root) is an empty registry, not
/// an error.
pub async fn load_registry(root: &ContentAssetRoot) -> anyhow::Result<AssetRegistry> {
    match try_read_relative(root, crate::assets::content::REGISTRY_FILE_NAME).await? {
        Some(bytes) => {
            let text =
                String::from_utf8(bytes).context("asset registry file is not valid UTF-8")?;
            AssetRegistry::parse(&text)
        }
        None => Ok(AssetRegistry::default()),
    }
}

/// Reads `<root>/<relative>`, returning `Ok(None)` when it simply is not
/// there (a missing file natively, a non-success status on wasm) so the
/// caller can turn absence into its own error message. Any other failure
/// is a real error.
async fn try_read_relative(
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
