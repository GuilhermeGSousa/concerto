use std::path::PathBuf;

pub use concerto_core_macros::Asset;

pub use crate::asset_id;
#[doc(hidden)]
pub use concerto_core_macros::asset_id_bytes as __asset_id_bytes;

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
    /// wasm: the page origin, e.g. `"http://host"`.
    UrlBase(String),
}

impl ContentAssetRoot {
    /// Native: an executable-specific content directory when one exists,
    /// otherwise the directory containing the executable. Cargo workspace
    /// binaries can therefore coexist in one target directory by placing
    /// content under `<exe-dir>/<exe-name>-content/`; packaged applications
    /// can place `content/` directly beside the executable. wasm uses the page
    /// origin, matching Trunk's `copy-dir` target.
    pub fn default_for_platform() -> Self {
        cfg_if::cfg_if! {
            if #[cfg(target_arch = "wasm32")] {
                let origin = web_sys::window()
                    .and_then(|window| window.location().origin().ok())
                    .unwrap_or_default();
                ContentAssetRoot::UrlBase(origin)
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
