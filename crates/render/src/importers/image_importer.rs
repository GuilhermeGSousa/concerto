use std::path::Path;

use crate::assets::texture::{
    resolve, SourceEncoding, Texture, TextureRole, TextureSettings, TextureSource,
};
use concerto_asset_import::{ImportContext, ImportError, Importer, NoSettings};

pub struct ImageImporter;

/// The encoding of image file `bytes`, read from their signature.
///
/// # Errors
///
/// Returns a message when `bytes` are not a PNG or JPEG image.
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

/// A texture an importer emits, with the source image its file keeps as uncooked data.
pub struct ImportedTexture {
    pub texture: Texture,
    pub source: TextureSource,
}

/// The texture `name` of `ctx` from image file `bytes`.
///
/// It takes the `given` settings, else the ones its previous import or the
/// editor gave it, else the defaults.
///
/// # Errors
///
/// Returns a message when `bytes` are not a PNG or JPEG image, when the image
/// header cannot be read, or when the settings cannot derive a texture for
/// `role`.
pub fn imported_texture(
    ctx: &ImportContext,
    name: &str,
    role: TextureRole,
    bytes: Vec<u8>,
    given: Option<TextureSettings>,
) -> Result<ImportedTexture, String> {
    let encoding = source_encoding(&bytes)?;
    let source = TextureSource::new(bytes, encoding).map_err(|err| format!("{err:#}"))?;
    let settings = given
        .or_else(|| {
            ctx.previous::<Texture>(name)
                .map(|previous| previous.settings)
        })
        .unwrap_or_default();
    resolve(role, &settings, false)
        .map_err(|err| format!("texture `{name}` cannot be derived as {role:?}: {err:#}"))?;
    Ok(ImportedTexture {
        texture: Texture::imported(role, settings, &source),
        source,
    })
}

impl Importer for ImageImporter {
    type Settings = NoSettings;
    type AssetSettings = TextureSettings;

    fn name(&self) -> &'static str {
        "image"
    }

    fn supported_extensions(&self) -> &'static [&'static str] {
        &["png", "jpg", "jpeg"]
    }

    fn current_asset_settings(&self, ctx: &ImportContext) -> Option<TextureSettings> {
        ctx.previous::<Texture>("main")
            .map(|texture| texture.settings)
    }

    fn import(
        &self,
        source_path: &Path,
        _settings: &NoSettings,
        asset_settings: Option<&TextureSettings>,
        ctx: &mut ImportContext,
    ) -> Result<(), ImportError> {
        let malformed = |message: String| ImportError::MalformedSource {
            source_path: source_path.to_path_buf(),
            message,
        };
        let bytes = std::fs::read(source_path).map_err(|err| ImportError::SourceUnreadable {
            source_path: source_path.to_path_buf(),
            message: err.to_string(),
        })?;
        let ImportedTexture { texture, source } = imported_texture(
            ctx,
            "main",
            TextureRole::Color,
            bytes,
            asset_settings.copied(),
        )
        .map_err(malformed)?;
        ctx.emit_with("main", &texture, &source)
            .map_err(|err| malformed(format!("{err:?}")))?;
        Ok(())
    }
}
