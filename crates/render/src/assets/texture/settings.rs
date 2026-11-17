use concerto_reflect::Reflect;
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

const OPTIONS_KEY: &str = "texture_settings";

fn compression_name(compression: Compression) -> &'static str {
    match compression {
        Compression::Auto => "auto",
        Compression::Bc7 => "bc7",
        Compression::Bc1 => "bc1",
        Compression::Bc5 => "bc5",
        Compression::Bc4 => "bc4",
        Compression::Uncompressed => "uncompressed",
    }
}

fn color_space_name(color_space: ColorSpace) -> &'static str {
    match color_space {
        ColorSpace::Srgb => "srgb",
        ColorSpace::Linear => "linear",
    }
}

/// The settings an import gives the textures it creates, read from its options' `texture_settings` table; the defaults when there is none.
pub fn initial_texture_settings(options: &toml::Table) -> Result<TextureSettings, String> {
    let mut settings = TextureSettings::default();
    let Some(value) = options.get(OPTIONS_KEY) else {
        return Ok(settings);
    };
    let Some(table) = value.as_table() else {
        return Err(format!(
            "{OPTIONS_KEY} must be a table of compression, color_space, max_size and mips, not {value}"
        ));
    };
    for (key, value) in table {
        let refused =
            |expected: &str| format!("{OPTIONS_KEY}.{key} cannot be {value}; use {expected}");
        match key.as_str() {
            "compression" => {
                settings.compression = match value.as_str() {
                    Some("auto") => Compression::Auto,
                    Some("bc7") => Compression::Bc7,
                    Some("bc1") => Compression::Bc1,
                    Some("bc5") => Compression::Bc5,
                    Some("bc4") => Compression::Bc4,
                    Some("uncompressed") => Compression::Uncompressed,
                    _ => {
                        return Err(refused(
                            "\"auto\", \"bc7\", \"bc1\", \"bc5\", \"bc4\" or \"uncompressed\"",
                        ))
                    }
                }
            }
            "color_space" => {
                settings.color_space = Some(match value.as_str() {
                    Some("srgb") => ColorSpace::Srgb,
                    Some("linear") => ColorSpace::Linear,
                    _ => return Err(refused("\"srgb\" or \"linear\", or leave it out for the role's")),
                })
            }
            "max_size" => {
                settings.max_size = match value.as_integer() {
                    Some(4096) => MaxSize::S4096,
                    Some(2048) => MaxSize::S2048,
                    Some(1024) => MaxSize::S1024,
                    Some(512) => MaxSize::S512,
                    _ => return Err(refused("4096, 2048, 1024 or 512, or leave it out for none")),
                }
            }
            "mips" => settings.mips = value.as_bool().ok_or_else(|| refused("true or false"))?,
            _ => {
                return Err(format!(
                    "{OPTIONS_KEY} has no setting '{key}'; use compression, color_space, max_size or mips"
                ))
            }
        }
    }
    Ok(settings)
}

/// The import options whose `texture_settings` give new textures `settings`; the inverse of [`initial_texture_settings`].
pub fn texture_settings_options(settings: &TextureSettings) -> toml::Table {
    let mut table = toml::Table::new();
    table.insert(
        "compression".into(),
        compression_name(settings.compression).into(),
    );
    if let Some(color_space) = settings.color_space {
        table.insert("color_space".into(), color_space_name(color_space).into());
    }
    if let Some(pixels) = settings.max_size.pixels() {
        table.insert("max_size".into(), i64::from(pixels).into());
    }
    table.insert("mips".into(), settings.mips.into());
    let mut options = toml::Table::new();
    options.insert(OPTIONS_KEY.into(), table.into());
    options
}
