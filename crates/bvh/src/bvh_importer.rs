use std::path::Path;

use concerto_asset_import::{ImportContext, ImportError, Importer};

use crate::parser;

/// Imports BVH motion capture files.
pub struct BvhImporter;

impl Importer for BvhImporter {
    fn supported_extensions(&self) -> &'static [&'static str] {
        &["bvh"]
    }

    fn import(&self, source_path: &Path, _ctx: &mut ImportContext) -> Result<(), ImportError> {
        let text =
            std::fs::read_to_string(source_path).map_err(|err| ImportError::SourceUnreadable {
                source_path: source_path.to_path_buf(),
                message: err.to_string(),
            })?;
        parser::parse(&text).map_err(|err| ImportError::MalformedSource {
            source_path: source_path.to_path_buf(),
            message: err.to_string(),
        })?;
        Ok(())
    }
}
