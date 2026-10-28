mod hsl;
mod linear_rgba;
mod srgba;

pub use hsl::Hsla;
pub use linear_rgba::LinearRgba;
pub use srgba::Srgba;

/// A color authored in one of several color spaces.
///
/// Construct in whichever space is most convenient to work in; use
/// [`Color::to_linear`] to get the canonical [`LinearRgba`] representation
/// (e.g. for GPU upload).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Color {
    LinearRgba(LinearRgba),
    Srgba(Srgba),
    Hsla(Hsla),
}

impl Default for Color {
    fn default() -> Self {
        Self::WHITE
    }
}

impl Color {
    pub const WHITE: Self = Self::LinearRgba(LinearRgba::WHITE);
    pub const BLACK: Self = Self::LinearRgba(LinearRgba::BLACK);
    pub const RED: Self = Self::LinearRgba(LinearRgba::RED);
    pub const GREEN: Self = Self::LinearRgba(LinearRgba::GREEN);
    pub const BLUE: Self = Self::LinearRgba(LinearRgba::BLUE);
    pub const TRANSPARENT: Self = Self::LinearRgba(LinearRgba::TRANSPARENT);

    #[inline]
    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self::LinearRgba(LinearRgba::new(r, g, b, a))
    }

    #[inline]
    pub const fn srgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self::Srgba(Srgba::new(r, g, b, a))
    }

    /// Construct from sRGB-encoded HSL with hue in degrees.
    #[inline]
    pub const fn hsl(h: f32, s: f32, l: f32, a: f32) -> Self {
        Self::Hsla(Hsla::new(h, s, l, a))
    }

    /// Construct from raw RGBA bytes (0–255), interpreting them as linear values.
    #[inline]
    pub fn from_rgba_bytes(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self::LinearRgba(LinearRgba::from_bytes(r, g, b, a))
    }

    /// Construct from a packed `u32` in RGBA byte order.
    #[inline]
    pub fn from_rgba_u32(packed: u32) -> Self {
        Self::LinearRgba(LinearRgba::from_rgba_u32(packed))
    }

    pub fn random_color() -> Self {
        Self::LinearRgba(LinearRgba::random_color())
    }

    /// Relative luminance (ITU-R BT.709), valid for linear light.
    #[inline]
    pub fn luminance(self) -> f32 {
        self.to_linear().luminance()
    }

    /// Return as `[f32; 4]` in linear light, for use where a raw array is needed.
    #[inline]
    pub fn to_array(self) -> [f32; 4] {
        self.to_linear().to_array()
    }

    /// Convert to the canonical linear-light representation.
    #[inline]
    pub fn to_linear(self) -> LinearRgba {
        match self {
            Self::LinearRgba(c) => c,
            Self::Srgba(c) => c.to_linear(),
            Self::Hsla(c) => c.to_linear(),
        }
    }

    #[inline]
    pub fn to_srgba(self) -> Srgba {
        match self {
            Self::Srgba(c) => c,
            _ => self.to_linear().to_srgba(),
        }
    }

    #[inline]
    pub fn to_hsl(self) -> Hsla {
        match self {
            Self::Hsla(c) => c,
            Self::Srgba(c) => c.to_hsl(),
            Self::LinearRgba(c) => c.to_hsl(),
        }
    }
}

impl From<[f32; 4]> for Color {
    fn from(arr: [f32; 4]) -> Self {
        Self::LinearRgba(LinearRgba::from(arr))
    }
}

impl From<Color> for [f32; 4] {
    fn from(c: Color) -> Self {
        c.to_array()
    }
}

impl From<LinearRgba> for Color {
    fn from(c: LinearRgba) -> Self {
        Self::LinearRgba(c)
    }
}

impl From<Srgba> for Color {
    fn from(c: Srgba) -> Self {
        Self::Srgba(c)
    }
}

impl From<Hsla> for Color {
    fn from(c: Hsla) -> Self {
        Self::Hsla(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgba_to_hsl_converts_directly() {
        let srgba = Srgba::new(0.2, 0.55, 0.9, 0.7);
        assert_eq!(Color::Srgba(srgba).to_hsl(), srgba.to_hsl());
    }
}
