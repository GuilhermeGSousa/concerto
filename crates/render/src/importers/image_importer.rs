use std::path::Path;

use crate::assets::texture::{
    initial_texture_settings, resolve, SourceEncoding, Texture, TextureRole, TextureSettings,
    TextureSource,
};
use concerto_asset_import::{ImportContext, ImportError, Importer};

pub struct ImageImporter;

/// The encoding of image file `bytes`, read from their signature.
pub fn source_encoding(bytes: &[u8]) -> Result<SourceEncoding, String> {
    match image::guess_format(bytes) {
        Ok(image::ImageFormat::Png) => Ok(SourceEncoding::Png),
        Ok(image::ImageFormat::Jpeg) => Ok(SourceEncoding::Jpeg),
        Ok(format) => Err(format!(
            "{format:?} images cannot be textures; use PNG or JPEG"
        )),
        Err(error) => Err(format!("not a PNG or JPEG image: {error}")),
    }
}

/// The texture `name` of `ctx` from image file `bytes`, keeping the settings its previous import or the editor gave it, else taking `initial`; fails when they cannot derive for `role`.
pub fn imported_texture(
    ctx: &ImportContext,
    name: &str,
    role: TextureRole,
    bytes: Vec<u8>,
    initial: TextureSettings,
) -> Result<Texture, String> {
    let encoding = source_encoding(&bytes)?;
    let source = TextureSource::new(bytes, encoding).map_err(|err| format!("{err:#}"))?;
    let settings = ctx
        .previous::<Texture>(name)
        .map_or(initial, |previous| previous.settings);
    resolve(role, &settings, false)
        .map_err(|err| format!("texture `{name}` cannot be derived as {role:?}: {err:#}"))?;
    Ok(Texture::imported(role, settings, source))
}

impl Importer for ImageImporter {
    fn supported_extensions(&self) -> &'static [&'static str] {
        &["png", "jpg", "jpeg"]
    }

    fn import(&self, source_path: &Path, ctx: &mut ImportContext) -> Result<(), ImportError> {
        let malformed = |message: String| ImportError::MalformedSource {
            source_path: source_path.to_path_buf(),
            message,
        };
        let initial = initial_texture_settings(ctx.options()).map_err(malformed)?;
        let bytes = std::fs::read(source_path).map_err(|err| ImportError::SourceUnreadable {
            source_path: source_path.to_path_buf(),
            message: err.to_string(),
        })?;
        let texture =
            imported_texture(ctx, "main", TextureRole::Color, bytes, initial).map_err(malformed)?;
        ctx.emit("main", &texture)
            .map_err(|err| malformed(format!("{err:?}")))?;
        Ok(())
    }
}
