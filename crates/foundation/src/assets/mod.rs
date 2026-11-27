use std::path::{Path, PathBuf};

pub use concerto_foundation_macros::Asset;

pub use crate::asset_id;
#[doc(hidden)]
pub use concerto_foundation_macros::asset_id_bytes as __asset_id_bytes;

/// Reads a content asset's persistent UUID at compile time.
///
/// Paths are relative to the calling package's manifest. Register the content
/// directory with `concerto_asset_build::track_assets` in that package's build script.
/// Only the header is read; the expansion is a constant UUID, not asset bytes.
/// A missing file expands to `AssetId::nil()` with a warning.
///
/// `"content/hero/scene.gasset#mesh/0"` names an asset embedded in a file.
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
pub mod derived;
pub mod handle;
pub mod registry;
pub mod utils;

/// Where the runtime finds content asset files.
///
/// Only the root differs per platform. Every address is a full path relative
/// to it, such as `"content/hero/scene.gasset"`: the `content/` segment is
/// part of the address, not injected by the root.
#[derive(Debug, Clone)]
pub enum ContentAssetRoot {
    /// Native: the directory containing the executable.
    Directory(PathBuf),
    /// wasm: the URL of the page's directory, without a trailing slash.
    UrlBase(String),
}

impl ContentAssetRoot {
    /// The content root of the running program on this platform.
    ///
    /// Natively it is [`content_root_for_exe`] of the running executable: an
    /// executable-specific content directory when it holds `content/`,
    /// otherwise the directory containing the executable. Cargo workspace
    /// binaries can therefore coexist in one target directory by placing
    /// content under `<exe-dir>/<exe-name>-content/`, while packaged
    /// applications place `content/` directly beside the executable.
    ///
    /// On wasm it is the page's base URL, so games hosted under a sub-path
    /// find their content.
    pub fn default_for_platform() -> Self {
        cfg_if::cfg_if! {
            if #[cfg(target_arch = "wasm32")] {
                ContentAssetRoot::UrlBase(web_content_base())
            } else {
                let root = std::env::current_exe()
                    .map(|exe| content_root_for_exe(&exe))
                    .unwrap_or_else(|_| PathBuf::from("."));
                ContentAssetRoot::Directory(root)
            }
        }
    }
}

/// The native content root for the executable at `exe`: `<exe-dir>/<exe>-content/` when it holds a `content/` directory, otherwise `<exe-dir>`.
pub fn content_root_for_exe(exe: &Path) -> PathBuf {
    let Some(exe_dir) = exe.parent() else {
        return PathBuf::from(".");
    };
    exe.file_stem()
        .map(|name| exe_dir.join(format!("{}-content", name.to_string_lossy())))
        .filter(|root| root.join("content").is_dir())
        .unwrap_or_else(|| exe_dir.to_path_buf())
}

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

/// A unit of loadable engine content.
///
/// The `Serialize + DeserializeOwned` supertrait means a serialized, on-disk
/// asset is just an `Asset`, so `ImportContext::emit` needs no separate DTO
/// trait. Reach for a distinct DTO type only when the live asset holds data
/// that cannot serialize, such as GPU descriptor handles or `&'static`
/// references; never merely for an [`AssetHandle`](handle::AssetHandle)
/// field, which serializes to its bare [`AssetId`].
pub trait Asset: Send + Sync + 'static + serde::Serialize + serde::de::DeserializeOwned {
    /// The version of this type's payload encoding, stored in each content asset's header.
    const FORMAT_VERSION: u32 = 1;

    fn name() -> &'static str;

    /// Decodes a payload written at `version`, older than [`FORMAT_VERSION`](Self::FORMAT_VERSION).
    ///
    /// # Errors
    ///
    /// Returns an error when the payload cannot be migrated. The default
    /// implementation rejects every version.
    fn migrate(version: u32, _payload: &[u8]) -> anyhow::Result<Self> {
        anyhow::bail!(
            "{} version {version} has no migration to version {}; re-import its source, or re-create it if it was authored in the editor",
            Self::name(),
            Self::FORMAT_VERSION
        )
    }

    /// The ids of every sub-asset this one references.
    ///
    /// The import tool's reference-integrity pass reads it. Empty for leaf assets.
    fn referenced_sub_assets(&self) -> Vec<AssetId> {
        Vec::new()
    }
}

/// Data an asset's file keeps in a section of its own.
///
/// It serves tools and a derive that misses its cache. Loads never read it,
/// and a cook drops it.
pub trait UncookedData:
    Send + Sync + 'static + serde::Serialize + serde::de::DeserializeOwned
{
    /// The asset type whose files carry this data.
    type Asset: Asset;

    /// The version of this type's encoding, stored with each section.
    const VERSION: u32 = 1;

    /// Decodes a section written at `version`, older than [`VERSION`](Self::VERSION).
    ///
    /// # Errors
    ///
    /// Returns an error when the section cannot be migrated. The default
    /// implementation rejects every version.
    fn migrate(version: u32, _bytes: &[u8]) -> anyhow::Result<Self> {
        anyhow::bail!(
            "the uncooked data of {} version {version} has no migration to version {}; re-import its source, or re-create it if it was authored in the editor",
            <Self::Asset as Asset>::name(),
            Self::VERSION
        )
    }
}

/// An asset the editor can create from nothing.
pub trait CreatableAsset: Asset {
    fn new_asset() -> Self;
}

pub trait LoadableAsset: Asset {
    /// Finishes runtime initialization once the payload is deserialized.
    ///
    /// A load calls it after decoding the payload, and
    /// [`AssetServer::add`](asset_server::AssetServer::add) calls it inline.
    ///
    /// # Errors
    ///
    /// Returns an error when the asset cannot be initialized. A load then
    /// fails, and `add` logs the error and leaves the handle unresolved.
    fn on_load(&mut self, _context: &asset_server::AssetLoadContext) -> anyhow::Result<()> {
        Ok(())
    }
}

/// An asset that can be placed in a scene as an entity.
pub trait PlaceableAsset: LoadableAsset {
    /// Adds to `entity` what makes it show or use the asset behind `handle`.
    fn place(
        asset: &Self,
        handle: handle::AssetHandle<Self>,
        entity: concerto_ecs::Entity,
        cmd: &mut concerto_ecs::CommandQueue,
    );
}

#[cfg(test)]
mod content_root_tests {
    use super::{content_root_for_exe, directory_of, AssetId};

    #[test]
    fn content_root_for_exe_picks_the_isolated_directory_by_its_content_folder() {
        let dir =
            std::env::temp_dir().join(format!("content-root-{}", AssetId::new().simple_hex()));
        let exe = dir.join("game");
        std::fs::create_dir_all(dir.join("game-content")).unwrap();
        assert_eq!(content_root_for_exe(&exe), dir);

        std::fs::create_dir_all(dir.join("game-content/content")).unwrap();
        assert_eq!(content_root_for_exe(&exe), dir.join("game-content"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

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
    }
}
