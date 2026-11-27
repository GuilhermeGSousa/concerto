use concerto_reflect::{Reflect, ReflectDefault};
use serde::{Deserialize, Serialize};

/// What a texture holds, set by its importer; it picks `Auto`'s compression and the default colour space.
#[derive(Reflect, Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum TextureRole {
    /// Colour, such as albedo: sRGB by default, and BC7 with alpha or BC1 without.
    #[default]
    Color,
    /// A tangent-space normal map: linear and BC5 by default.
    Normal,
    /// Other values, such as roughness or masks: linear and BC7 by default.
    Data,
}

/// The block compression a texture's levels are derived in.
#[derive(Reflect, Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum Compression {
    /// Picked from the texture's [`TextureRole`] and whether it has alpha.
    #[default]
    Auto,
    /// BC7: high-quality RGBA, one byte per texel.
    Bc7,
    /// BC1: RGB with one-bit alpha, half a byte per texel.
    Bc1,
    /// BC5: two linear channels, for normal maps; one byte per texel.
    Bc5,
    /// BC4: one linear channel, half a byte per texel.
    Bc4,
    /// Plain RGBA8, four bytes per texel.
    Uncompressed,
}

/// Whether a texture's colour channels are sRGB-encoded or linear.
#[derive(Reflect, Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum ColorSpace {
    /// sRGB-encoded, as colour images are; the GPU decodes it to linear when sampling.
    #[default]
    Srgb,
    /// Linear values, sampled as stored.
    Linear,
}

/// The largest side a texture's first level may have.
#[derive(Reflect, Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum MaxSize {
    /// No limit.
    #[default]
    None,
    /// At most 4096 texels.
    S4096,
    /// At most 2048 texels.
    S2048,
    /// At most 1024 texels.
    S1024,
    /// At most 512 texels.
    S512,
}

impl MaxSize {
    /// The cap in texels, or `None` for no cap.
    pub fn pixels(self) -> Option<u32> {
        match self {
            MaxSize::None => None,
            MaxSize::S4096 => Some(4096),
            MaxSize::S2048 => Some(2048),
            MaxSize::S1024 => Some(1024),
            MaxSize::S512 => Some(512),
        }
    }
}

/// How a texture's sampled levels are derived from its source.
#[derive(Reflect, Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[reflect(Default)]
pub struct TextureSettings {
    /// The block compression of the derived levels.
    pub compression: Compression,
    /// The colour space, or `None` for the [`TextureRole`]'s default.
    pub color_space: Option<ColorSpace>,
    /// The largest side of level 0; a larger source is scaled down.
    pub max_size: MaxSize,
    /// Whether a full mip chain is derived, rather than level 0 alone.
    pub mips: bool,
}

impl Default for TextureSettings {
    fn default() -> Self {
        Self {
            compression: Compression::Auto,
            color_space: None,
            max_size: MaxSize::None,
            mips: true,
        }
    }
}

impl TextureSettings {
    /// These settings with `compression`.
    pub fn with_compression(mut self, compression: Compression) -> Self {
        self.compression = compression;
        self
    }

    /// These settings with `color_space`; `None` takes the role's.
    pub fn with_color_space(mut self, color_space: Option<ColorSpace>) -> Self {
        self.color_space = color_space;
        self
    }

    /// These settings with `max_size`.
    pub fn with_max_size(mut self, max_size: MaxSize) -> Self {
        self.max_size = max_size;
        self
    }

    /// These settings with mips on or off.
    pub fn with_mips(mut self, mips: bool) -> Self {
        self.mips = mips;
        self
    }

    /// The colour space these settings give a texture of `role`.
    pub fn color_space_for(&self, role: TextureRole) -> ColorSpace {
        self.color_space.unwrap_or(match role {
            TextureRole::Color => ColorSpace::Srgb,
            TextureRole::Normal | TextureRole::Data => ColorSpace::Linear,
        })
    }
}
