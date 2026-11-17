use std::path::Path;

use crate::assets::texture::{
    SourceEncoding, Texture, TextureRole, TextureSettings, TextureSource,
};
use concerto_asset_import::{ImportContext, ImportError, Importer};

pub struct ImageImporter;

impl Importer for ImageImporter {
    fn supported_extensions(&self) -> &'static [&'static str] {
        &["png", "jpg", "jpeg"]
    }

    fn import(&self, source_path: &Path, ctx: &mut ImportContext) -> Result<(), ImportError> {
        let malformed = |message: String| ImportError::MalformedSource {
            source_path: source_path.to_path_buf(),
            message,
        };
        let img = image::open(source_path).map_err(|err| malformed(err.to_string()))?;
        let mut bytes = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .map_err(|err| malformed(err.to_string()))?;
        let source = TextureSource::new(bytes, SourceEncoding::Png)
            .map_err(|err| malformed(format!("{err:#}")))?;
        let texture = Texture::imported(TextureRole::Color, TextureSettings::default(), source);

        ctx.emit("main", &texture)
            .map_err(|err| malformed(format!("{err:?}")))?;

        Ok(())
    }
}
