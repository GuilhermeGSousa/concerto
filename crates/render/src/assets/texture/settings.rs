use concerto_reflect::{Reflect, ReflectDefault};
use serde::{Deserialize, Serialize};

/// What a texture holds, set by its importer; it picks `Auto`'s compression and the default colour space.
#[derive(Reflect, Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum TextureRole {
    #[default]
    Color,
    Normal,
    Data,
}

/// The block compression a texture's levels are derived in.
#[derive(Reflect, Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum Compression {
    #[default]
    Auto,
    Bc7,
    Bc1,
    Bc5,
    Bc4,
    Uncompressed,
}

/// Whether a texture's colour channels are sRGB-encoded or linear.
#[derive(Reflect, Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum ColorSpace {
    #[default]
    Srgb,
    Linear,
}

/// The largest side a texture's first level may have.
#[derive(Reflect, Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum MaxSize {
    #[default]
    None,
    S4096,
    S2048,
    S1024,
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
    pub compression: Compression,
    pub color_space: Option<ColorSpace>,
    pub max_size: MaxSize,
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
