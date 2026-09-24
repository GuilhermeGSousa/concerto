use std::path::{Path, PathBuf};

use concerto_core::assets::{Asset, AssetId};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DependencyEntry {
    pub path: PathBuf,
    pub content_hash: u64,
}

/// Hashes a file's contents for [`ImportContext::track_dependency`]. Kept
/// as dead plumbing: importers still record dependencies, but nothing
/// consumes them until a future incremental-import feature reads them back.
pub fn hash_file_contents(path: &Path) -> Result<u64, ImportError> {
    use std::hash::{DefaultHasher, Hash, Hasher};

    let bytes = std::fs::read(path).map_err(|err| ImportError::SourceUnreadable {
        source_path: path.to_path_buf(),
        message: err.to_string(),
    })?;

    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    Ok(hasher.finish())
}

#[derive(Debug, Clone)]
pub struct EmittedSubAsset {
    pub name: String,
    pub asset_id: AssetId,
    pub type_name: &'static str,
    pub bytes: Vec<u8>,
    pub references: Vec<AssetId>,
}

#[derive(Debug, Clone)]
pub enum ImportError {
    SourceUnreadable {
        source_path: PathBuf,
        message: String,
    },
    MalformedSource {
        source_path: PathBuf,
        message: String,
    },
    MissingRequiredData {
        source_path: PathBuf,
        message: String,
    },
    SerializationFailed {
        sub_asset_name: String,
        message: String,
    },
}

#[derive(Debug, Clone)]
pub struct ImportOutputs {
    pub sub_assets: Vec<EmittedSubAsset>,
    pub dependencies: Vec<DependencyEntry>,
}

/// Maps a sub-asset name to the `AssetId` that references to it should use.
/// The default addresses sub-assets against the source file
/// (`"<source>#<sub>"`); the `import` tool substitutes content-tree paths.
pub type SubAssetIdResolver = Box<dyn Fn(&str) -> AssetId + Send + Sync>;

pub struct ImportContext {
    relative_source: PathBuf,
    sub_assets: Vec<EmittedSubAsset>,
    dependencies: Vec<DependencyEntry>,
    sub_asset_id_resolver: Option<SubAssetIdResolver>,
}

impl ImportContext {
    pub fn new(relative_source: PathBuf) -> Self {
        Self {
            relative_source,
            sub_assets: Vec::new(),
            dependencies: Vec::new(),
            sub_asset_id_resolver: None,
        }
    }

    /// Like [`ImportContext::new`], but with a caller-supplied
    /// [`SubAssetIdResolver`] that decides the id of every same-file
    /// cross-reference (used by the `import` tool to redirect them to
    /// content-tree paths).
    pub fn with_sub_asset_id_resolver(
        relative_source: PathBuf,
        resolver: SubAssetIdResolver,
    ) -> Self {
        Self {
            relative_source,
            sub_assets: Vec::new(),
            dependencies: Vec::new(),
            sub_asset_id_resolver: Some(resolver),
        }
    }

    /// Computes the stable AssetId a sub-asset name resolves to *within this
    /// source file*, without needing to know the final content-tree
    /// address. Importers use this to build same-file cross-references
    /// (e.g. a material's texture, a scene node's mesh) as real
    /// `AssetHandle::weak(id)` values on the structs they emit.
    ///
    /// When a [`SubAssetIdResolver`] was supplied via
    /// [`ImportContext::with_sub_asset_id_resolver`], it decides the id
    /// instead of the default `"<source>#<sub>"` addressing.
    pub fn sub_asset_id(&self, name: &str) -> AssetId {
        match &self.sub_asset_id_resolver {
            Some(resolve) => resolve(name),
            None => AssetId::from_path(&format!("{}#{}", self.relative_source.display(), name)),
        }
    }

    pub fn emit<T: Asset>(&mut self, name: &str, value: &T) -> Result<(), ImportError> {
        let bytes = bincode::serialize(value).map_err(|err| ImportError::SerializationFailed {
            sub_asset_name: name.to_string(),
            message: err.to_string(),
        })?;

        self.sub_assets.push(EmittedSubAsset {
            name: name.to_string(),
            asset_id: self.sub_asset_id(name),
            type_name: T::name(),
            bytes,
            references: value.referenced_sub_assets(),
        });

        Ok(())
    }

    pub fn track_dependency(&mut self, path: PathBuf, content_hash: u64) {
        self.dependencies
            .push(DependencyEntry { path, content_hash });
    }

    pub fn into_parts(self) -> ImportOutputs {
        ImportOutputs {
            sub_assets: self.sub_assets,
            dependencies: self.dependencies,
        }
    }
}
