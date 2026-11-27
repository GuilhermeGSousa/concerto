//! The importer interface: an [`Importer`] reads a source file and emits its sub-assets into an
//! [`ImportContext`], which the import engine writes as content assets.

mod clip_names;
mod import_context;

pub use clip_names::clip_names;
pub use import_context::{
    DependencyEntry, EmittedSubAsset, EmittedUncooked, ImportContext, ImportError, ImportOutputs,
    PreviousOutputResolver, SubAssetIdResolver,
};

use std::any::TypeId;
use std::path::{Path, PathBuf};

use concerto_reflect::{GetTypeRegistration, Reflect, ReflectDefault, TypeRegistry};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// The settings of an importer without any.
#[derive(Reflect, Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[reflect(Default)]
pub struct NoSettings {}

/// Turns one kind of source file, such as a glTF, into content assets.
pub trait Importer: Send + Sync {
    /// The importer's own settings, kept in the `[settings]` table of each source's sidecar.
    type Settings: Reflect + GetTypeRegistration + Default + Clone + Serialize + DeserializeOwned;
    /// The settings of the one asset a source makes, kept in that asset; [`NoSettings`] for an importer without them.
    type AssetSettings: Reflect
        + GetTypeRegistration
        + Default
        + Clone
        + Serialize
        + DeserializeOwned;

    /// The name sidecars persist for this importer.
    fn name(&self) -> &'static str;

    /// The file extensions this importer reads, lowercase and without the dot.
    fn supported_extensions(&self) -> &'static [&'static str];

    /// The asset settings the source's asset holds now, or `None` when it has none yet.
    fn current_asset_settings(&self, _ctx: &ImportContext) -> Option<Self::AssetSettings> {
        None
    }

    /// Imports the source at `source_path` with `settings`, emitting its sub-assets into `ctx`.
    ///
    /// # Errors
    ///
    /// Returns an [`ImportError`] when the source cannot be read, is malformed or lacks data the
    /// import needs, or when an emitted sub-asset cannot be serialized.
    fn import(
        &self,
        source_path: &Path,
        settings: &Self::Settings,
        asset_settings: Option<&Self::AssetSettings>,
        ctx: &mut ImportContext,
    ) -> Result<(), ImportError>;

    /// Checks the sub-assets an import emitted and returns what is wrong with them; none by
    /// default.
    fn validate(&self, _sub_assets: &[EmittedSubAsset]) -> Vec<ValidationIssue> {
        Vec::new()
    }
}

/// An [`Importer`] whose settings types are erased behind [`Reflect`].
pub trait ErasedImporter: Send + Sync {
    /// The name sidecars persist for this importer.
    fn name(&self) -> &'static str;

    /// The file extensions this importer reads, lowercase and without the dot.
    fn supported_extensions(&self) -> &'static [&'static str];

    /// Registers the importer's settings and asset settings types.
    fn register_types(&self, registry: &mut TypeRegistry);

    /// The default importer settings.
    fn default_settings(&self) -> Box<dyn Reflect>;

    /// The default asset settings, or `None` when the importer has none.
    fn default_asset_settings(&self) -> Option<Box<dyn Reflect>>;

    /// Reads importer settings from a sidecar's `[settings]` table.
    ///
    /// # Errors
    ///
    /// Returns the deserializer's message when the table does not match the importer's settings type.
    fn settings_from_toml(&self, table: &toml::Table) -> Result<Box<dyn Reflect>, String>;

    /// Writes importer settings as a sidecar's `[settings]` table.
    ///
    /// # Errors
    ///
    /// Returns a message when `settings` is not of the importer's settings type or does not serialize to TOML.
    fn settings_to_toml(&self, settings: &dyn Reflect) -> Result<toml::Table, String>;

    /// A copy of this importer's settings or asset settings.
    ///
    /// # Panics
    ///
    /// Panics when `settings` is of neither the importer's settings type nor its asset settings type.
    fn clone_settings(&self, settings: &dyn Reflect) -> Box<dyn Reflect>;

    /// The asset settings the source's asset holds now, or `None` when it has none yet.
    fn current_asset_settings(&self, ctx: &ImportContext) -> Option<Box<dyn Reflect>>;

    /// Whether the importer has asset settings.
    fn has_asset_settings(&self) -> bool;

    /// Imports with settings of this importer's types.
    ///
    /// # Errors
    ///
    /// Returns [`ImportError::MalformedSource`] naming the expected type when `settings` or
    /// `asset_settings` is of another type, and otherwise whatever the importer returns.
    fn import(
        &self,
        source_path: &Path,
        settings: &dyn Reflect,
        asset_settings: Option<&dyn Reflect>,
        ctx: &mut ImportContext,
    ) -> Result<(), ImportError>;

    /// Checks the sub-assets an import emitted and returns what is wrong with them.
    fn validate(&self, sub_assets: &[EmittedSubAsset]) -> Vec<ValidationIssue>;
}

fn typed<'a, T: Reflect>(value: &'a dyn Reflect, source_path: &Path) -> Result<&'a T, ImportError> {
    value
        .downcast_ref::<T>()
        .ok_or_else(|| ImportError::MalformedSource {
            source_path: source_path.to_path_buf(),
            message: format!(
                "settings must be a {}, not a {}",
                std::any::type_name::<T>(),
                value.type_name()
            ),
        })
}

impl<I: Importer> ErasedImporter for I {
    fn name(&self) -> &'static str {
        Importer::name(self)
    }

    fn supported_extensions(&self) -> &'static [&'static str] {
        Importer::supported_extensions(self)
    }

    fn register_types(&self, registry: &mut TypeRegistry) {
        registry.register::<I::Settings>();
        registry.register::<I::AssetSettings>();
    }

    fn default_settings(&self) -> Box<dyn Reflect> {
        Box::new(I::Settings::default())
    }

    fn default_asset_settings(&self) -> Option<Box<dyn Reflect>> {
        ErasedImporter::has_asset_settings(self)
            .then(|| Box::new(I::AssetSettings::default()) as Box<dyn Reflect>)
    }

    fn settings_from_toml(&self, table: &toml::Table) -> Result<Box<dyn Reflect>, String> {
        table
            .clone()
            .try_into::<I::Settings>()
            .map(|settings| Box::new(settings) as Box<dyn Reflect>)
            .map_err(|err| err.to_string())
    }

    fn settings_to_toml(&self, settings: &dyn Reflect) -> Result<toml::Table, String> {
        let settings = settings.downcast_ref::<I::Settings>().ok_or_else(|| {
            format!(
                "settings must be a {}, not a {}",
                std::any::type_name::<I::Settings>(),
                settings.type_name()
            )
        })?;
        toml::Table::try_from(settings).map_err(|err| err.to_string())
    }

    fn clone_settings(&self, settings: &dyn Reflect) -> Box<dyn Reflect> {
        if let Some(settings) = settings.downcast_ref::<I::Settings>() {
            return Box::new(settings.clone());
        }
        match settings.downcast_ref::<I::AssetSettings>() {
            Some(settings) => Box::new(settings.clone()),
            None => panic!(
                "{} is neither {} nor {}",
                settings.type_name(),
                std::any::type_name::<I::Settings>(),
                std::any::type_name::<I::AssetSettings>()
            ),
        }
    }

    fn current_asset_settings(&self, ctx: &ImportContext) -> Option<Box<dyn Reflect>> {
        Importer::current_asset_settings(self, ctx)
            .map(|settings| Box::new(settings) as Box<dyn Reflect>)
    }

    fn has_asset_settings(&self) -> bool {
        TypeId::of::<I::AssetSettings>() != TypeId::of::<NoSettings>()
    }

    fn import(
        &self,
        source_path: &Path,
        settings: &dyn Reflect,
        asset_settings: Option<&dyn Reflect>,
        ctx: &mut ImportContext,
    ) -> Result<(), ImportError> {
        let settings = typed::<I::Settings>(settings, source_path)?;
        let asset_settings = asset_settings
            .map(|value| typed::<I::AssetSettings>(value, source_path))
            .transpose()?;
        Importer::import(self, source_path, settings, asset_settings, ctx)
    }

    fn validate(&self, sub_assets: &[EmittedSubAsset]) -> Vec<ValidationIssue> {
        Importer::validate(self, sub_assets)
    }
}

/// How serious a [`ValidationIssue`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationSeverity {
    /// The assets are usable but may not be what the author meant.
    Warning,
    /// The assets are not usable.
    Error,
}

/// A problem [`Importer::validate`] found in an import's sub-assets.
#[derive(Debug, Clone)]
pub struct ValidationIssue {
    /// How serious the problem is.
    pub severity: ValidationSeverity,
    /// What is wrong, for the user to read.
    pub message: String,
    /// The source the sub-assets came from.
    pub source_path: PathBuf,
    /// The name of the sub-asset at fault, or `None` when the problem is the import's as a whole.
    pub sub_asset_name: Option<String>,
}
