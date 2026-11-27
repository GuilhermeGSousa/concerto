mod blob;
mod derive;
mod settings;

use anyhow::Context;
use concerto_foundation::{
    assets::{
        content::ContentAssetHeader,
        derived::{DerivedData, DerivedInputs},
        Asset, AssetId, LoadableAsset, UncookedData,
    },
    hash::Fnv1a,
};
use serde::{Deserialize, Serialize};

pub use blob::TextureLevels;
pub use derive::derive_levels;
pub(crate) use derive::resolve;
pub use settings::{ColorSpace, Compression, MaxSize, TextureRole, TextureSettings};
pub use wgpu_types::TextureFormat;

pub(crate) use blob::{block_extent, block_row_bytes, block_rows};

/// Whether a texture is sampled from its levels or drawn into by a camera.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum TextureKind {
    Sampled,
    RenderTarget { width: u32, height: u32 },
}

/// How a texture's source bytes are encoded.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum SourceEncoding {
    Png,
    Jpeg,
}

/// The image file a texture was imported from, as it was imported.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct TextureSource {
    pub bytes: Vec<u8>,
    pub encoding: SourceEncoding,
    pub width: u32,
    pub height: u32,
    pub hash: u64,
}

impl TextureSource {
    /// A source of `bytes` in `encoding`, reading its size from the image header and hashing the bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when the image header cannot be read as `encoding`.
    pub fn new(bytes: Vec<u8>, encoding: SourceEncoding) -> anyhow::Result<Self> {
        let format = match encoding {
            SourceEncoding::Png => image::ImageFormat::Png,
            SourceEncoding::Jpeg => image::ImageFormat::Jpeg,
        };
        let (width, height) = image::ImageReader::with_format(std::io::Cursor::new(&bytes), format)
            .into_dimensions()
            .with_context(|| format!("failed to read the {encoding:?} image header"))?;
        let mut hash = Fnv1a::new();
        hash.bytes(&bytes);
        Ok(Self {
            bytes,
            encoding,
            width,
            height,
            hash: hash.finish(),
        })
    }

    /// What a texture's payload keeps of this source.
    pub fn summary(&self) -> SourceSummary {
        SourceSummary {
            width: self.width,
            height: self.height,
            hash: self.hash,
        }
    }
}

/// What a texture's payload keeps of its source: enough for the derive key and the size.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub struct SourceSummary {
    pub width: u32,
    pub height: u32,
    pub hash: u64,
}

/// A texture asset: its settings and either a summary of its imported source or levels built in code.
///
/// The sampled levels of an imported texture are derived data; the source
/// image itself is the file's uncooked data.
#[derive(Serialize, Deserialize)]
pub struct Texture {
    pub kind: TextureKind,
    pub role: TextureRole,
    pub settings: TextureSettings,
    pub source: Option<SourceSummary>,
    #[serde(skip)]
    levels: Option<TextureLevels>,
}

#[derive(Deserialize)]
struct TextureV2 {
    kind: TextureKind,
    role: TextureRole,
    settings: TextureSettings,
    source: Option<TextureSource>,
}

impl Asset for Texture {
    const FORMAT_VERSION: u32 = 3;

    fn name() -> &'static str {
        "Texture"
    }

    fn migrate(version: u32, payload: &[u8]) -> anyhow::Result<Self> {
        anyhow::ensure!(
            version == 2,
            "{} version {version} has no migration to version {}; re-import its source, or re-create it if it was authored in the editor",
            Self::name(),
            Self::FORMAT_VERSION
        );
        let old: TextureV2 =
            bincode::deserialize(payload).context("failed to decode a version-2 Texture")?;
        Ok(Self {
            kind: old.kind,
            role: old.role,
            settings: old.settings,
            source: old.source.as_ref().map(TextureSource::summary),
            levels: None,
        })
    }
}

impl UncookedData for TextureSource {
    type Asset = Texture;
}

impl Texture {
    /// A sampled texture from RGBA8 pixels matching `format`, with its mip chain built from them.
    pub fn new(width: u32, height: u32, format: TextureFormat, data: Vec<u8>) -> Self {
        let mips = if format.block_copy_size(None) == Some(4) {
            derive::build_mips(width, height, derive::Texels::of(format), &data)
        } else {
            Vec::new()
        };
        Self {
            kind: TextureKind::Sampled,
            role: if format.is_srgb() {
                TextureRole::Color
            } else {
                TextureRole::Data
            },
            settings: TextureSettings::default().with_compression(Compression::Uncompressed),
            source: None,
            levels: Some(TextureLevels {
                format,
                width,
                height,
                levels: std::iter::once(data).chain(mips).collect(),
            }),
        }
    }

    /// A GPU-only render target; the camera system allocates the wgpu texture.
    pub fn render_target(width: u32, height: u32) -> Self {
        Self {
            kind: TextureKind::RenderTarget { width, height },
            role: TextureRole::Color,
            settings: TextureSettings::default(),
            source: None,
            levels: None,
        }
    }

    /// An imported texture whose levels are derived when it loads from `source`, which its file keeps as uncooked data.
    pub fn imported(role: TextureRole, settings: TextureSettings, source: &TextureSource) -> Self {
        Self {
            kind: TextureKind::Sampled,
            role,
            settings,
            source: Some(source.summary()),
            levels: None,
        }
    }

    /// The levels the GPU samples, once derived or built.
    pub fn levels(&self) -> Option<&TextureLevels> {
        self.levels.as_ref()
    }

    pub fn size(&self) -> wgpu::Extent3d {
        let extent = |width, height| wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        match (&self.kind, &self.levels, &self.source) {
            (TextureKind::RenderTarget { width, height }, _, _) => extent(*width, *height),
            (TextureKind::Sampled, Some(levels), _) => extent(levels.width, levels.height),
            (TextureKind::Sampled, None, Some(source)) => extent(source.width, source.height),
            (TextureKind::Sampled, None, None) => extent(0, 0),
        }
    }
}

impl LoadableAsset for Texture {}

impl DerivedData for Texture {
    const DERIVED_VERSION: u32 = 2;

    fn is_derived(&self) -> bool {
        matches!(self.kind, TextureKind::Sampled)
    }

    fn file_is_derived(header: &ContentAssetHeader, payload: &[u8]) -> anyhow::Result<bool> {
        if header.kind_version < 2 {
            return Ok(false);
        }
        anyhow::ensure!(
            header.kind_version <= Self::FORMAT_VERSION,
            "Texture payload is version {}, newer than this build's version {}",
            header.kind_version,
            Self::FORMAT_VERSION
        );
        let kind: TextureKind =
            bincode::deserialize(payload).context("failed to read a Texture's kind")?;
        Ok(matches!(kind, TextureKind::Sampled))
    }

    fn derived_inputs(&self) -> Vec<AssetId> {
        Vec::new()
    }

    fn derived_key(&self, _inputs: &DerivedInputs) -> anyhow::Result<u64> {
        let source = self
            .source
            .as_ref()
            .context("a sampled Texture has no source to derive from")?;
        let mut hash = Fnv1a::new();
        hash.raw(&source.hash.to_le_bytes());
        hash.bytes(&bincode::serialize(&self.role)?);
        hash.bytes(&bincode::serialize(&self.settings)?);
        hash.raw(&[u8::from(cfg!(target_arch = "wasm32"))]);
        Ok(hash.finish())
    }

    fn derive(
        &self,
        inputs: &DerivedInputs,
        progress: &mut dyn FnMut(usize, usize),
    ) -> anyhow::Result<Vec<u8>> {
        let summary = self
            .source
            .as_ref()
            .context("a sampled Texture has no source to derive from")?;
        let source = inputs
            .own_uncooked::<TextureSource>()
            .context("the texture holds no source image; import its source again")?;
        anyhow::ensure!(
            source.hash == summary.hash,
            "the texture's source image has hash {:016x} but its payload names {:016x}: the file changed while it loaded; load it again",
            source.hash,
            summary.hash
        );
        let settings = if cfg!(target_arch = "wasm32") {
            self.settings.with_compression(Compression::Uncompressed)
        } else {
            self.settings
        };
        let levels = derive_levels(&source, self.role, &settings)?;
        progress(1, 1);
        levels.encode()
    }

    fn apply_derived(&mut self, blob: &[u8]) -> anyhow::Result<()> {
        self.levels = Some(TextureLevels::decode(blob)?);
        Ok(())
    }
}
