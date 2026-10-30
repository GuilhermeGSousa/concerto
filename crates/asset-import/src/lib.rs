mod clip_names;
mod import_context;

pub use clip_names::clip_names;
pub use import_context::{
    hash_file_contents, DependencyEntry, EmittedSubAsset, ImportContext, ImportError,
    ImportOutputs, SubAssetIdResolver,
};

use std::path::{Path, PathBuf};

pub trait Importer: Send + Sync {
    fn supported_extensions(&self) -> &'static [&'static str];

    fn import(&self, source_path: &Path, ctx: &mut ImportContext) -> Result<(), ImportError>;

    fn validate(&self, _sub_assets: &[EmittedSubAsset]) -> Vec<ValidationIssue> {
        Vec::new()
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
