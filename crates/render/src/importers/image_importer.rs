use std::path::Path;

use concerto_asset_import::{ImportContext, ImportError, Importer};
use image::GenericImageView;

use crate::assets::texture::{Texture, TextureKind};

pub struct ImageImporter;

impl Importer for ImageImporter {
    fn supported_extensions(&self) -> &'static [&'static str] {
        &["png", "jpg", "jpeg"]
    }

    fn import(&self, source_path: &Path, ctx: &mut ImportContext) -> Result<(), ImportError> {
        let img = image::open(source_path).map_err(|err| ImportError::MalformedSource {
            source_path: source_path.to_path_buf(),
            message: err.to_string(),
        })?;

        let (width, height) = img.dimensions();
        let texture = Texture {
            width,
            height,
            // TODO(asset-import-pipeline): colour space hard-coded sRGB —
            // standalone linear textures (normal/metallic-roughness .png)
            // can't be imported correctly yet; needs a per-entry hint in
            // content.toml or a filename convention.
            format: wgpu_types::TextureFormat::Rgba8UnormSrgb,
            kind: TextureKind::Sampled,
            data: img.to_rgba8().into_raw(),
        };

        ctx.emit("main", &texture)
            .map_err(|err| ImportError::MalformedSource {
                source_path: source_path.to_path_buf(),
                message: format!("{err:?}"),
            })?;

        Ok(())
    }
}
