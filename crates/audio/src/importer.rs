use std::io::Cursor;
use std::path::Path;

use concerto_asset_import::{ImportContext, ImportError, Importer};
use kira::sound::static_sound::StaticSoundData;

use crate::AudioClip;

/// Imports an `.ogg` or `.wav` source as one `AudioClip` named `main`, failing when it does not decode.
pub struct AudioImporter;

impl Importer for AudioImporter {
    fn supported_extensions(&self) -> &'static [&'static str] {
        &["ogg", "wav"]
    }

    fn import(&self, source_path: &Path, ctx: &mut ImportContext) -> Result<(), ImportError> {
        let bytes = std::fs::read(source_path).map_err(|err| ImportError::SourceUnreadable {
            source_path: source_path.to_path_buf(),
            message: err.to_string(),
        })?;
        StaticSoundData::from_cursor(Cursor::new(bytes.as_slice())).map_err(|err| {
            ImportError::MalformedSource {
                source_path: source_path.to_path_buf(),
                message: err.to_string(),
            }
        })?;
        ctx.emit("main", &AudioClip::encoded(bytes))
    }
}
