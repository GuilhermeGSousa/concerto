use std::collections::BTreeMap;
use std::path::PathBuf;

use concerto_foundation::assets::content::decode_content_asset;
use concerto_foundation::assets::{Asset, AssetId};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DependencyEntry {
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct EmittedSubAsset {
    pub name: String,
    pub asset_id: AssetId,
    pub type_name: &'static str,
    pub kind_version: u32,
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
    pub renames: Vec<(String, String)>,
}

/// Maps a sub-asset name and its hinted old name to the `AssetId` that references to it should use.
/// The default addresses sub-assets against the source file
/// (`"<source>#<sub>"`); the `import` tool substitutes content-tree paths.
pub type SubAssetIdResolver = Box<dyn Fn(&str, Option<&str>) -> AssetId + Send + Sync>;

/// Reads the standalone content record of the output a sub-asset name names, as the content holds it before this import.
pub type PreviousOutputResolver = Box<dyn Fn(&str) -> Option<Vec<u8>> + Send + Sync>;

pub struct ImportContext {
    relative_source: PathBuf,
    sub_assets: Vec<EmittedSubAsset>,
    dependencies: Vec<DependencyEntry>,
    sub_asset_id_resolver: Option<SubAssetIdResolver>,
    renames: BTreeMap<String, String>,
    previous_outputs: Option<PreviousOutputResolver>,
}

impl ImportContext {
    pub fn new(relative_source: PathBuf) -> Self {
        Self {
            relative_source,
            sub_assets: Vec::new(),
            dependencies: Vec::new(),
            sub_asset_id_resolver: None,
            renames: BTreeMap::new(),
            previous_outputs: None,
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
            renames: BTreeMap::new(),
            previous_outputs: None,
        }
    }

    /// Sets how [`ImportContext::previous`] reads the outputs the content already holds.
    pub fn with_previous_outputs(mut self, resolver: PreviousOutputResolver) -> Self {
        self.previous_outputs = Some(resolver);
        self
    }

    /// The output `name` as the content holds it before this import, or `None` when there is none or it does not decode at this build's version.
    pub fn previous<T: Asset>(&self, name: &str) -> Option<T> {
        let resolve = self.previous_outputs.as_ref()?;
        let record = resolve(name).or_else(|| resolve(self.renames.get(name)?))?;
        decode_content_asset::<T>(&record)
            .map(|(_, value)| value)
            .ok()
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
            Some(resolve) => resolve(name, self.renames.get(name).map(String::as_str)),
            None => AssetId::from_path(&format!("{}#{}", self.relative_source.display(), name)),
        }
    }

    /// Records that `new_name` replaces `old_name` so the sub-asset keeps its id; call it before the first `sub_asset_id` or `emit` for `new_name`.
    pub fn renamed_from(&mut self, new_name: &str, old_name: &str) {
        if new_name != old_name {
            self.renames
                .insert(new_name.to_string(), old_name.to_string());
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
            kind_version: T::FORMAT_VERSION,
            bytes,
            references: value.referenced_sub_assets(),
        });

        Ok(())
    }

    /// Records a file the source refers to, which an import from outside the project copies beside it when it can be read.
    pub fn track_dependency(&mut self, path: PathBuf) {
        self.dependencies.push(DependencyEntry { path });
    }

    pub fn into_parts(self) -> ImportOutputs {
        ImportOutputs {
            sub_assets: self.sub_assets,
            dependencies: self.dependencies,
            renames: self.renames.into_iter().collect(),
        }
    }
}
