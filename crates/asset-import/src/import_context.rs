use std::collections::BTreeMap;
use std::path::PathBuf;

use concerto_foundation::assets::content::decode_content_asset;
use concerto_foundation::assets::{Asset, AssetId, UncookedData};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
/// A file a source refers to, such as a glTF's external texture.
pub struct DependencyEntry {
    /// The file's path, as the importer resolved it from the source's directory.
    pub path: PathBuf,
}

/// One output an import emitted, serialized and ready to be written as a content asset.
#[derive(Debug, Clone)]
pub struct EmittedSubAsset {
    /// The output's name within its source, such as `mesh/Body`.
    pub name: String,
    /// The output's id, from [`ImportContext::sub_asset_id`].
    pub asset_id: AssetId,
    /// The asset kind, as `Asset::name` gives it.
    pub type_name: &'static str,
    /// The `Asset::FORMAT_VERSION` the payload was written with.
    pub kind_version: u32,
    /// The bincode-serialized asset.
    pub bytes: Vec<u8>,
    /// The uncooked section the output is written with, if it has one.
    pub uncooked: Option<EmittedUncooked>,
    /// The ids of the assets this one refers to.
    pub references: Vec<AssetId>,
}

/// The uncooked section an importer emitted with an output: its bytes and the version of their encoding.
#[derive(Debug, Clone)]
pub struct EmittedUncooked {
    /// The `UncookedData::VERSION` the bytes were written with.
    pub version: u32,
    /// The bincode-serialized uncooked data.
    pub bytes: Vec<u8>,
}

/// Why an import failed.
#[derive(Debug, Clone)]
pub enum ImportError {
    /// The source, or a file it refers to, could not be read.
    SourceUnreadable {
        /// The file that could not be read.
        source_path: PathBuf,
        /// Why.
        message: String,
    },
    /// The source does not parse, or holds data the importer cannot use.
    MalformedSource {
        /// The source.
        source_path: PathBuf,
        /// What is wrong.
        message: String,
    },
    /// The source lacks data the import needs.
    MissingRequiredData {
        /// The source.
        source_path: PathBuf,
        /// What is missing.
        message: String,
    },
    /// An emitted output could not be serialized.
    SerializationFailed {
        /// The output's name.
        sub_asset_name: String,
        /// The serializer's message.
        message: String,
    },
}

/// Everything an import emitted, taken from its [`ImportContext`].
#[derive(Debug, Clone)]
pub struct ImportOutputs {
    /// The outputs, in the order they were emitted.
    pub sub_assets: Vec<EmittedSubAsset>,
    /// The files the source refers to.
    pub dependencies: Vec<DependencyEntry>,
    /// Each renamed output, as its new name then its old one, from
    /// [`ImportContext::renamed_from`].
    pub renames: Vec<(String, String)>,
}

/// Maps a sub-asset name and its hinted old name to the [`AssetId`] that references to it should use.
///
/// The default addresses sub-assets against the source file
/// (`"<source>#<sub>"`); the `import` tool substitutes content-tree paths.
pub type SubAssetIdResolver = Box<dyn Fn(&str, Option<&str>) -> AssetId + Send + Sync>;

/// Reads the standalone content record of the output a sub-asset name names, as the content holds it before this import.
pub type PreviousOutputResolver = Box<dyn Fn(&str) -> Option<Vec<u8>> + Send + Sync>;

/// Where an [`Importer`](crate::Importer) emits the outputs of one source.
///
/// It gives each output a stable id, which other outputs of the same source reference it by.
pub struct ImportContext {
    relative_source: PathBuf,
    sub_assets: Vec<EmittedSubAsset>,
    dependencies: Vec<DependencyEntry>,
    sub_asset_id_resolver: Option<SubAssetIdResolver>,
    renames: BTreeMap<String, String>,
    previous_outputs: Option<PreviousOutputResolver>,
}

impl ImportContext {
    /// A context for the source at `relative_source`, whose outputs are addressed as
    /// `"<relative_source>#<name>"`.
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

    /// The output `name` as the content holds it before this import.
    ///
    /// Returns `None` when there is no such output or it does not decode at this build's version.
    pub fn previous<T: Asset>(&self, name: &str) -> Option<T> {
        let resolve = self.previous_outputs.as_ref()?;
        let record = resolve(name).or_else(|| resolve(self.renames.get(name)?))?;
        decode_content_asset::<T>(&record)
            .map(|(_, value)| value)
            .ok()
    }

    /// The stable [`AssetId`] a sub-asset name resolves to within this source file.
    ///
    /// It needs no final content-tree address. Importers use it to build
    /// same-file cross-references, such as a material's texture or a scene
    /// node's mesh, as real `AssetHandle::weak(id)` values on the structs they emit.
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

    /// Records that `new_name` replaces `old_name`, so the sub-asset keeps its id.
    ///
    /// Call it before the first [`Self::sub_asset_id`] or [`Self::emit`] for `new_name`.
    pub fn renamed_from(&mut self, new_name: &str, old_name: &str) {
        if new_name != old_name {
            self.renames
                .insert(new_name.to_string(), old_name.to_string());
        }
    }

    /// Emits `value` as the output `name`, with no uncooked section.
    ///
    /// # Errors
    ///
    /// Returns [`ImportError::SerializationFailed`] when `value` does not serialize.
    pub fn emit<T: Asset>(&mut self, name: &str, value: &T) -> Result<(), ImportError> {
        self.push(name, value, None)
    }

    /// Emits `value` as the output `name`, with `uncooked` as its uncooked section.
    ///
    /// # Errors
    ///
    /// Returns [`ImportError::SerializationFailed`] when `value` or `uncooked` does not serialize.
    pub fn emit_with<E: UncookedData>(
        &mut self,
        name: &str,
        value: &E::Asset,
        uncooked: &E,
    ) -> Result<(), ImportError> {
        let section = EmittedUncooked {
            version: E::VERSION,
            bytes: serialize(name, uncooked)?,
        };
        self.push(name, value, Some(section))
    }

    fn push<T: Asset>(
        &mut self,
        name: &str,
        value: &T,
        uncooked: Option<EmittedUncooked>,
    ) -> Result<(), ImportError> {
        let bytes = serialize(name, value)?;
        self.sub_assets.push(EmittedSubAsset {
            name: name.to_string(),
            asset_id: self.sub_asset_id(name),
            type_name: T::name(),
            kind_version: T::FORMAT_VERSION,
            bytes,
            uncooked,
            references: value.referenced_sub_assets(),
        });
        Ok(())
    }

    /// Records a file the source refers to, which an import from outside the project copies beside it when it can be read.
    pub fn track_dependency(&mut self, path: PathBuf) {
        self.dependencies.push(DependencyEntry { path });
    }

    /// Everything emitted, consuming the context.
    pub fn into_parts(self) -> ImportOutputs {
        ImportOutputs {
            sub_assets: self.sub_assets,
            dependencies: self.dependencies,
            renames: self.renames.into_iter().collect(),
        }
    }
}

fn serialize<T: serde::Serialize>(name: &str, value: &T) -> Result<Vec<u8>, ImportError> {
    bincode::serialize(value).map_err(|err| ImportError::SerializationFailed {
        sub_asset_name: name.to_string(),
        message: err.to_string(),
    })
}
