use std::path::PathBuf;

pub use concerto_foundation_macros::Asset;

pub use crate::asset_id;
#[doc(hidden)]
pub use concerto_foundation_macros::asset_id_bytes as __asset_id_bytes;

/// Read a content asset's persistent UUID at compile time.
///
/// Paths are relative to the calling package's manifest. Register the content
/// directory with `concerto_asset_build::track_assets` in that package's build script.
/// Only the header is read; the expansion is a constant UUID, not asset bytes.
///
/// ```ignore
/// const HERO: AssetId = asset_id!("content/hero/scene.gasset");
/// let handle = server.load::<Scene>(HERO);
/// ```
#[macro_export]
macro_rules! asset_id {
    ($path:literal $(,)?) => {
        $crate::assets::AssetId::from_bytes($crate::assets::__asset_id_bytes!($path))
    };
}

pub mod asset_container;
pub mod asset_server;
pub mod asset_store;
pub mod content;
pub mod handle;
pub mod utils;

/// Where the runtime finds content asset files. Only the root differs per
/// platform; every address is a full path relative to it (e.g.
/// `"content/hero/scene.gasset"` — the `content/` segment is part of the
/// address, not injected by the root).
#[derive(Debug, Clone)]
pub enum ContentAssetRoot {
    /// Native: the directory containing the executable.
    Directory(PathBuf),
    /// wasm: the URL of the directory the page is served from, without a
    /// trailing slash, e.g. `"https://host/games/mine"`.
    UrlBase(String),
}

impl ContentAssetRoot {
    /// Native: an executable-specific content directory when one exists,
    /// otherwise the directory containing the executable. Cargo workspace
    /// binaries can therefore coexist in one target directory by placing
    /// content under `<exe-dir>/<exe-name>-content/`; packaged applications
    /// can place `content/` directly beside the executable. wasm uses the
    /// page's base URL (its directory, or a `<base>` element), so a game
    /// hosted under a sub-path — as itch.io and GitHub Pages do — finds the
    /// `content/` directory published next to its page.
    pub fn default_for_platform() -> Self {
        cfg_if::cfg_if! {
            if #[cfg(target_arch = "wasm32")] {
                ContentAssetRoot::UrlBase(web_content_base())
            } else {
                let root = std::env::current_exe()
                    .ok()
                    .and_then(|exe| {
                        let exe_dir = exe.parent()?.to_path_buf();
                        let isolated_root = exe.file_stem()
                            .map(|name| exe_dir.join(format!("{}-content", name.to_string_lossy())));
                        Some(isolated_root
                            .filter(|root| root.join(content::REGISTRY_FILE_NAME).is_file())
                            .unwrap_or(exe_dir))
                    })
                    .unwrap_or_else(|| PathBuf::from("."));
                ContentAssetRoot::Directory(root)
            }
        }
    }
}

/// The directory URL content is served from on the web: the document's base
/// URI (which honours `<base href>`) up to its last `/`.
#[cfg(target_arch = "wasm32")]
fn web_content_base() -> String {
    let document_base = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.base_uri().ok().flatten());
    match document_base {
        Some(uri) => directory_of(&uri),
        None => web_sys::window()
            .and_then(|window| window.location().origin().ok())
            .unwrap_or_default(),
    }
}

/// `https://host/a/b/index.html?x` → `https://host/a/b`.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
fn directory_of(uri: &str) -> String {
    let without_query = uri.split(['?', '#']).next().unwrap_or(uri);
    match without_query.rfind('/') {
        Some(slash) if slash > without_query.find("//").map_or(0, |i| i + 1) => {
            without_query[..slash].to_string()
        }
        _ => without_query.trim_end_matches('/').to_string(),
    }
}

impl Default for ContentAssetRoot {
    fn default() -> Self {
        Self::default_for_platform()
    }
}

pub use concerto_asset_format::AssetId;

/// A unit of loadable engine content. The `Serialize + DeserializeOwned`
/// supertrait means a serialized, on-disk asset is just an `Asset` —
/// `ImportContext::emit` needs no separate DTO trait. Reach for a distinct
/// DTO type only when the live asset holds data that genuinely cannot
/// serialize (GPU descriptor handles, `&'static` refs) — never merely for an
/// `AssetHandle<T>` field, which serializes to its bare `AssetId`.
pub trait Asset: Send + Sync + 'static + serde::Serialize + serde::de::DeserializeOwned {
    fn name() -> &'static str;

    /// AssetIds of every sub-asset this one references — the import tool's
    /// reference-integrity pass. Empty for leaf assets.
    fn referenced_sub_assets(&self) -> Vec<AssetId> {
        Vec::new()
    }
}

pub trait LoadableAsset: Asset {
    /// Finishes runtime initialization after the cooked payload has been
    /// deserialized. Most assets need no additional work.
    fn on_load(&mut self, _context: &asset_server::AssetLoadContext) -> anyhow::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod content_root_tests {
    use super::directory_of;

    #[test]
    fn directory_of_strips_the_page_and_query() {
        assert_eq!(
            directory_of("https://html-classic.itch.zone/html/123/index.html?v=1"),
            "https://html-classic.itch.zone/html/123"
        );
        assert_eq!(
            directory_of("https://host/games/mine/"),
            "https://host/games/mine"
        );
        assert_eq!(
            directory_of("http://localhost:8080/"),
            "http://localhost:8080"
        );
        assert_eq!(
            directory_of("http://localhost:8080"),
            "http://localhost:8080"
        );
    }
}
