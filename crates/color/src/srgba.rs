use crate::LinearRgba;

/// sRGB + linear alpha color.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Srgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Srgba {
    #[inline]
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    #[inline]
    pub const fn to_array(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }

    /// Construct from raw sRGB bytes (0–255).
    #[inline]
    pub fn from_bytes(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self::new(
            r as f32 / 255.0,
            g as f32 / 255.0,
            b as f32 / 255.0,
            a as f32 / 255.0,
        )
    }

    /// Construct from a CSS-style hex string (`#RRGGBB` or `#RRGGBBAA`).
    ///
    /// Returns `None` if the string is malformed.
    pub fn from_hex(hex: &str) -> Option<Self> {
        let hex = hex.strip_prefix('#').unwrap_or(hex);
        let parse_byte = |s: &str| u8::from_str_radix(s, 16).ok();
        match hex.len() {
            6 => Some(Self::from_bytes(
                parse_byte(&hex[0..2])?,
                parse_byte(&hex[2..4])?,
                parse_byte(&hex[4..6])?,
                255,
            )),
            8 => Some(Self::from_bytes(
                parse_byte(&hex[0..2])?,
                parse_byte(&hex[2..4])?,
                parse_byte(&hex[4..6])?,
                parse_byte(&hex[6..8])?,
            )),
            _ => None,
        }
    }

    /// Relative luminance computed after converting to linear light.
    #[inline]
    pub fn luminance(&self) -> f32 {
        LinearRgba::from(*self).luminance()
    }

    #[inline]
    pub fn to_linear(self) -> LinearRgba {
        LinearRgba::from(self)
    }
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

impl From<Srgba> for LinearRgba {
    fn from(c: Srgba) -> Self {
        Self::new(
            srgb_to_linear(c.r),
            srgb_to_linear(c.g),
            srgb_to_linear(c.b),
            c.a,
        )
    }
}

impl From<LinearRgba> for Srgba {
    fn from(c: LinearRgba) -> Self {
        Self::new(
            linear_to_srgb(c.r),
            linear_to_srgb(c.g),
            linear_to_srgb(c.b),
            c.a,
        )
    }
}
