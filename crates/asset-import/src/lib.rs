mod clip_names;
mod import_context;

pub use clip_names::clip_names;
pub use import_context::{
    DependencyEntry, EmittedSubAsset, ImportContext, ImportError, ImportOutputs,
    PreviousOutputResolver, SubAssetIdResolver,
};

use std::any::TypeId;
use std::path::{Path, PathBuf};

use concerto_reflect::{GetTypeRegistration, Reflect, TypeRegistry};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// The settings of an importer without any.
#[derive(Reflect, Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoSettings {}

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

    fn supported_extensions(&self) -> &'static [&'static str];

    /// The asset settings the source's asset holds now, or `None` when it has none yet.
    fn current_asset_settings(&self, _ctx: &ImportContext) -> Option<Self::AssetSettings> {
        None
    }

    fn import(
        &self,
        source_path: &Path,
        settings: &Self::Settings,
        asset_settings: Option<&Self::AssetSettings>,
        ctx: &mut ImportContext,
    ) -> Result<(), ImportError>;

    fn validate(&self, _sub_assets: &[EmittedSubAsset]) -> Vec<ValidationIssue> {
        Vec::new()
    }
}

/// An [`Importer`] whose settings types are erased behind [`Reflect`].
pub trait ErasedImporter: Send + Sync {
    /// The name sidecars persist for this importer.
    fn name(&self) -> &'static str;

    fn supported_extensions(&self) -> &'static [&'static str];

    /// Registers the importer's settings and asset settings types.
    fn register_types(&self, registry: &mut TypeRegistry);

    /// The default importer settings.
    fn default_settings(&self) -> Box<dyn Reflect>;

    /// The default asset settings, or `None` when the importer has none.
    fn default_asset_settings(&self) -> Option<Box<dyn Reflect>>;

    /// Reads importer settings from a sidecar's `[settings]` table.
    fn settings_from_toml(&self, table: &toml::Table) -> Result<Box<dyn Reflect>, String>;

    /// Writes importer settings as a sidecar's `[settings]` table.
    fn settings_to_toml(&self, settings: &dyn Reflect) -> Result<toml::Table, String>;

    /// A copy of this importer's settings or asset settings.
    fn clone_settings(&self, settings: &dyn Reflect) -> Box<dyn Reflect>;

    /// The asset settings the source's asset holds now, or `None` when it has none yet.
    fn current_asset_settings(&self, ctx: &ImportContext) -> Option<Box<dyn Reflect>>;

    /// Whether the importer has asset settings.
    fn has_asset_settings(&self) -> bool;

    /// Imports with settings of this importer's types; a value of another type fails, naming the expected one.
    fn import(
        &self,
        source_path: &Path,
        settings: &dyn Reflect,
        asset_settings: Option<&dyn Reflect>,
        ctx: &mut ImportContext,
    ) -> Result<(), ImportError>;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationSeverity {
    Warning,
    Error,
}

#[derive(Debug, Clone)]
pub struct ValidationIssue {
    pub severity: ValidationSeverity,
    pub message: String,
    pub source_path: PathBuf,
    pub sub_asset_name: Option<String>,
}
