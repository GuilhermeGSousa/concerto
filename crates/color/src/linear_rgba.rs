use bytemuck::{Pod, Zeroable};
use rand::RngExt;

use crate::{Hsla, Srgba};

/// Linear RGBA color, suitable for use as a GPU uniform.
///
/// The red, green and blue channels are linear light, usually in 0.0 to 1.0,
/// and may exceed 1.0 for HDR. Alpha is in 0.0 to 1.0. This is the canonical
/// representation every [`Color`](crate::Color) variant converts to.
#[repr(C)]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Pod,
    Zeroable,
    encase::ShaderType,
    serde::Serialize,
    serde::Deserialize,
)]
pub struct LinearRgba {
    /// The red channel in linear light.
    pub r: f32,
    /// The green channel in linear light.
    pub g: f32,
    /// The blue channel in linear light.
    pub b: f32,
    /// The alpha, 0.0 transparent to 1.0 opaque.
    pub a: f32,
}

impl LinearRgba {
    /// Opaque white.
    pub const WHITE: Self = Self::new(1.0, 1.0, 1.0, 1.0);
    /// Opaque black.
    pub const BLACK: Self = Self::new(0.0, 0.0, 0.0, 1.0);
    /// Opaque pure red.
    pub const RED: Self = Self::new(1.0, 0.0, 0.0, 1.0);
    /// Opaque pure green.
    pub const GREEN: Self = Self::new(0.0, 1.0, 0.0, 1.0);
    /// Opaque pure blue.
    pub const BLUE: Self = Self::new(0.0, 0.0, 1.0, 1.0);
    /// Fully transparent black.
    pub const TRANSPARENT: Self = Self::new(0.0, 0.0, 0.0, 0.0);

    /// Creates a color from linear-light channels and alpha.
    #[inline]
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    /// Construct from raw RGBA bytes (0–255), interpreting them as linear values.
    #[inline]
    pub fn from_bytes(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self::new(
            r as f32 / 255.0,
            g as f32 / 255.0,
            b as f32 / 255.0,
            a as f32 / 255.0,
        )
    }

    /// Construct from a packed `u32` in RGBA byte order.
    #[inline]
    pub fn from_rgba_u32(packed: u32) -> Self {
        let [r, g, b, a] = packed.to_be_bytes();
        Self::from_bytes(r, g, b, a)
    }

    /// Returns an opaque color with each channel drawn uniformly from 0.0 to 1.0.
    pub fn random_color() -> Self {
        let mut rng = rand::rng();
        Self {
            r: rng.random(),
            g: rng.random(),
            b: rng.random(),
            a: 1.0,
        }
    }

    /// Relative luminance (ITU-R BT.709), valid for linear light.
    #[inline]
    pub fn luminance(&self) -> f32 {
        0.2126 * self.r + 0.7152 * self.g + 0.0722 * self.b
    }

    /// Return as `[f32; 4]` for use where a raw array is needed.
    #[inline]
    pub fn to_array(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }

    /// Converts to sRGB-encoded values.
    #[inline]
    pub fn to_srgba(self) -> Srgba {
        Srgba::from(self)
    }

    /// Converts to hue, saturation and lightness.
    #[inline]
    pub fn to_hsl(self) -> Hsla {
        Hsla::from(self)
    }
}

impl From<[f32; 4]> for LinearRgba {
    fn from([r, g, b, a]: [f32; 4]) -> Self {
        Self::new(r, g, b, a)
    }
}

impl From<LinearRgba> for [f32; 4] {
    fn from(c: LinearRgba) -> Self {
        c.to_array()
    }
}
