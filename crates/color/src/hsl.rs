use crate::{LinearRgba, Srgba};

/// Hue-Saturation-Lightness + alpha over sRGB-encoded values: hue in degrees, `s` and `l` not clamped.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Hsla {
    pub h: f32,
    pub s: f32,
    pub l: f32,
    pub a: f32,
}

impl Hsla {
    #[inline]
    pub const fn new(h: f32, s: f32, l: f32, a: f32) -> Self {
        Self { h, s, l, a }
    }

    #[inline]
    pub fn to_linear(self) -> LinearRgba {
        LinearRgba::from(self)
    }

    #[inline]
    pub fn to_srgba(self) -> Srgba {
        Srgba::from(self)
    }
}

impl From<Srgba> for Hsla {
    fn from(c: Srgba) -> Self {
        let max = c.r.max(c.g).max(c.b);
        let min = c.r.min(c.g).min(c.b);
        let delta = max - min;
        let l = (max + min) / 2.0;

        let denominator = 1.0 - (2.0 * l - 1.0).abs();
        let s = if denominator == 0.0 {
            0.0
        } else {
            delta / denominator
        };

        let sector = if delta == 0.0 {
            0.0
        } else if max == c.r {
            ((c.g - c.b) / delta).rem_euclid(6.0)
        } else if max == c.g {
            (c.b - c.r) / delta + 2.0
        } else {
            (c.r - c.g) / delta + 4.0
        };

        Self::new((sector * 60.0).rem_euclid(360.0), s, l, c.a)
    }
}

impl From<Hsla> for Srgba {
    fn from(c: Hsla) -> Self {
        if c.s == 0.0 {
            return Srgba::new(c.l, c.l, c.l, c.a);
        }

        let chroma = (1.0 - (2.0 * c.l - 1.0).abs()) * c.s;
        let h6 = c.h.rem_euclid(360.0) / 60.0;
        let x = chroma * (1.0 - (h6 % 2.0 - 1.0).abs());
        let m = c.l - chroma / 2.0;

        let (r1, g1, b1) = match h6 as u32 {
            0 => (chroma, x, 0.0),
            1 => (x, chroma, 0.0),
            2 => (0.0, chroma, x),
            3 => (0.0, x, chroma),
            4 => (x, 0.0, chroma),
            _ => (chroma, 0.0, x),
        };

        Srgba::new(r1 + m, g1 + m, b1 + m, c.a)
    }
}

impl From<LinearRgba> for Hsla {
    fn from(c: LinearRgba) -> Self {
        Self::from(Srgba::from(c))
    }
}

impl From<Hsla> for LinearRgba {
    fn from(c: Hsla) -> Self {
        LinearRgba::from(Srgba::from(c))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-5;

    fn assert_srgba_near(a: Srgba, b: Srgba, eps: f32) {
        assert!(
            (a.r - b.r).abs() <= eps
                && (a.g - b.g).abs() <= eps
                && (a.b - b.b).abs() <= eps
                && (a.a - b.a).abs() <= eps,
            "{a:?} != {b:?}"
        );
    }

    fn assert_linear_near(a: LinearRgba, b: LinearRgba, eps: f32) {
        assert!(
            (a.r - b.r).abs() <= eps
                && (a.g - b.g).abs() <= eps
                && (a.b - b.b).abs() <= eps
                && (a.a - b.a).abs() <= eps,
            "{a:?} != {b:?}"
        );
    }

    #[test]
    fn pure_green_is_hue_120() {
        let c = Srgba::from(Hsla::new(120.0, 1.0, 0.5, 1.0));
        assert_srgba_near(c, Srgba::from_hex("#00ff00").unwrap(), EPS);
    }

    #[test]
    fn hsl_210_half_half_is_a_steel_blue() {
        let c = Srgba::from(Hsla::new(210.0, 0.5, 0.5, 1.0));
        assert_srgba_near(c, Srgba::new(0.25, 0.5, 0.75, 1.0), EPS);
        assert_srgba_near(c, Srgba::from_hex("#4080bf").unwrap(), 0.005);
    }

    #[test]
    fn zero_saturation_is_grey_of_lightness() {
        let c = Srgba::from(Hsla::new(0.0, 0.0, 0.5, 1.0));
        assert_srgba_near(c, Srgba::new(0.5, 0.5, 0.5, 1.0), EPS);
    }

    #[test]
    fn srgba_round_trips_over_a_grid() {
        let steps = [0.0, 0.1, 0.25, 0.5, 0.75, 0.9, 1.0];
        for &r in &steps {
            for &g in &steps {
                for &b in &steps {
                    let c = Srgba::new(r, g, b, 0.7);
                    let back = Srgba::from(Hsla::from(c));
                    assert_srgba_near(c, back, EPS);
                }
            }
        }
    }

    #[test]
    fn linear_rgba_round_trips_over_a_grid() {
        let steps = [0.0, 0.1, 0.25, 0.5, 0.75, 1.0];
        for &r in &steps {
            for &g in &steps {
                for &b in &steps {
                    let c = LinearRgba::new(r, g, b, 0.7);
                    let back = LinearRgba::from(Hsla::from(c));
                    assert_linear_near(c, back, 1e-4);
                }
            }
        }
    }

    #[test]
    fn linear_conversion_goes_through_srgb() {
        let linear = LinearRgba::new(0.2, 0.4, 0.6, 1.0);
        let expected = Hsla::from(Srgba::from(linear));
        assert_eq!(linear.to_hsl(), expected);
        assert_eq!(expected.to_linear(), LinearRgba::from(expected.to_srgba()));
    }

    #[test]
    fn hue_360_equals_hue_0() {
        let a = Srgba::from(Hsla::new(360.0, 0.6, 0.4, 1.0));
        let b = Srgba::from(Hsla::new(0.0, 0.6, 0.4, 1.0));
        assert_srgba_near(a, b, EPS);
    }

    #[test]
    fn negative_hue_wraps() {
        let a = Srgba::from(Hsla::new(-90.0, 0.6, 0.4, 1.0));
        let b = Srgba::from(Hsla::new(270.0, 0.6, 0.4, 1.0));
        assert_srgba_near(a, b, EPS);
    }

    #[test]
    fn hue_from_srgba_is_in_degrees_below_360() {
        let steps = [0.0, 0.2, 0.5, 0.8, 1.0];
        for &r in &steps {
            for &g in &steps {
                for &b in &steps {
                    let h = Hsla::from(Srgba::new(r, g, b, 1.0)).h;
                    assert!((0.0..360.0).contains(&h), "hue {h} for ({r}, {g}, {b})");
                }
            }
        }
        assert!((Hsla::from(Srgba::new(0.0, 0.0, 1.0, 1.0)).h - 240.0).abs() < EPS);
        assert!((Hsla::from(Srgba::new(1.0, 0.0, 0.5, 1.0)).h - 330.0).abs() < EPS);
    }

    #[test]
    fn alpha_passes_through() {
        let hsla = Hsla::from(Srgba::new(0.3, 0.6, 0.9, 0.25));
        assert_eq!(hsla.a, 0.25);
        assert_eq!(Srgba::from(hsla).a, 0.25);
        assert_eq!(LinearRgba::from(hsla).a, 0.25);
    }

    #[test]
    fn hdr_srgba_round_trips() {
        let c = Srgba::new(1.5, 0.2, 0.2, 1.0);
        let hsla = Hsla::from(c);
        assert!((hsla.l - 0.85).abs() < EPS);
        assert_srgba_near(Srgba::from(hsla), c, 1e-4);
    }

    #[test]
    fn zero_denominator_gives_zero_saturation() {
        let hsla = Hsla::from(Srgba::new(1.5, 0.5, 0.5, 1.0));
        assert!(hsla.s.is_finite());
        assert_eq!(hsla.s, 0.0);
    }
}
