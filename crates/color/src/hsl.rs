use crate::{LinearRgba, Srgba};

/// A hue, saturation and lightness color with alpha, over sRGB-encoded values.
///
/// The hue `h` is in degrees. Saturation `s` and lightness `l` are nominally in
/// 0.0 to 1.0 and are not clamped.
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
    use crate::Color;
    use proptest::prelude::*;

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

    fn assert_hsla_near(a: Hsla, b: Hsla) {
        assert!(
            (a.h - b.h).abs() <= 1e-4
                && (a.s - b.s).abs() <= EPS
                && (a.l - b.l).abs() <= EPS
                && (a.a - b.a).abs() <= EPS,
            "{a:?} != {b:?}"
        );
    }

    fn linear_near(a: LinearRgba, b: LinearRgba, eps: f32) -> bool {
        (a.r - b.r).abs() <= eps
            && (a.g - b.g).abs() <= eps
            && (a.b - b.b).abs() <= eps
            && (a.a - b.a).abs() <= eps
    }

    struct ToSrgb {
        hsla: Hsla,
        srgba: Srgba,
    }

    #[test]
    fn hsl_converts_to_the_srgb_it_names() {
        let cases = [
            ToSrgb {
                hsla: Hsla::new(210.0, 0.5, 0.5, 1.0),
                srgba: Srgba::new(0.25, 0.5, 0.75, 1.0),
            },
            ToSrgb {
                hsla: Hsla::new(0.0, 0.0, 0.5, 1.0),
                srgba: Srgba::new(0.5, 0.5, 0.5, 1.0),
            },
            ToSrgb {
                hsla: Hsla::new(360.0, 0.6, 0.4, 1.0),
                srgba: Srgba::new(0.64, 0.16, 0.16, 1.0),
            },
            ToSrgb {
                hsla: Hsla::new(-90.0, 0.6, 0.4, 0.25),
                srgba: Srgba::new(0.4, 0.16, 0.64, 0.25),
            },
        ];
        for case in cases {
            assert_srgba_near(Srgba::from(case.hsla), case.srgba, EPS);
        }
    }

    struct FromSrgb {
        srgba: Srgba,
        hsla: Hsla,
        back: Srgba,
    }

    #[test]
    fn srgb_converts_to_hsl_in_degrees_even_out_of_gamut() {
        let cases = [
            FromSrgb {
                srgba: Srgba::new(1.0, 0.0, 0.5, 1.0),
                hsla: Hsla::new(330.0, 1.0, 0.5, 1.0),
                back: Srgba::new(1.0, 0.0, 0.5, 1.0),
            },
            FromSrgb {
                srgba: Srgba::new(1.5, 0.2, 0.2, 1.0),
                hsla: Hsla::new(0.0, 1.3 / 0.3, 0.85, 1.0),
                back: Srgba::new(1.5, 0.2, 0.2, 1.0),
            },
            FromSrgb {
                srgba: Srgba::new(1.5, 0.5, 0.5, 1.0),
                hsla: Hsla::new(0.0, 0.0, 1.0, 1.0),
                back: Srgba::new(1.0, 1.0, 1.0, 1.0),
            },
        ];
        for case in cases {
            let hsla = Hsla::from(case.srgba);
            assert_hsla_near(hsla, case.hsla);
            assert_srgba_near(Srgba::from(hsla), case.back, 1e-4);
        }
    }

    proptest! {
        #[test]
        fn hsl_round_trips_in_gamut_with_its_hue_below_360(
            r in 0.0f32..=1.0,
            g in 0.0f32..=1.0,
            b in 0.0f32..=1.0,
            a in 0.0f32..=1.0,
        ) {
            let srgba = Srgba::new(r, g, b, a);
            let hsla = Hsla::from(srgba);
            prop_assert!((0.0..360.0).contains(&hsla.h), "hue {}", hsla.h);
            prop_assert_eq!(hsla.a, a);
            let back = Srgba::from(hsla);
            prop_assert!(
                (back.r - r).abs() <= EPS && (back.g - g).abs() <= EPS && (back.b - b).abs() <= EPS,
                "{:?} != {:?}", back, srgba
            );
            prop_assert_eq!(back.a, a);
            prop_assert_eq!(Color::Srgba(srgba).to_hsl(), srgba.to_hsl());

            let linear = LinearRgba::new(r, g, b, a);
            prop_assert_eq!(linear.to_hsl(), Hsla::from(Srgba::from(linear)));
            let linear_back = LinearRgba::from(linear.to_hsl());
            prop_assert!(linear_near(linear_back, linear, 1e-4), "{:?} != {:?}", linear_back, linear);
            prop_assert_eq!(linear.to_hsl().to_linear(), LinearRgba::from(linear.to_hsl().to_srgba()));
        }
    }
}
