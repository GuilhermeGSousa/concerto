const GOLDEN_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;
const MAX_OCTAVES: u32 = 8;
const OCTAVE_SHIFT_STREAM: u64 = 1024;
const OCTAVE_SHIFT_RANGE: f64 = 1024.0;

/// A seed for an independent stream, mixed from `seed` and `stream` (SplitMix64's finalizer).
pub fn mix_seed(seed: u64, stream: u64) -> u64 {
    let mut z = seed ^ stream.wrapping_mul(GOLDEN_GAMMA);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A value in `[0, 1)` drawn from `seed`.
pub fn unit_from_seed(seed: u64) -> f32 {
    (mix_seed(seed, 0) >> 40) as f32 / (1u32 << 24) as f32
}

/// 1D gradient (Perlin) noise at `x`.
///
/// The value is in `[-1, 1]`, 0 at every integer, with continuous first and
/// second derivatives. A non-finite `x` gives 0.
pub fn gradient_noise(seed: u64, x: f64) -> f32 {
    if !x.is_finite() {
        return 0.0;
    }
    let floor = x.floor();
    let i = floor as i64;
    let t = x - floor;
    let g0 = gradient(seed, i);
    let g1 = gradient(seed, i.wrapping_add(1));
    let a = g0 * t;
    let b = g1 * (t - 1.0);
    let value = 2.0 * (a + (b - a) * fade(t));
    (value as f32).clamp(-1.0, 1.0)
}

/// `octaves` octaves of gradient noise at `x`, summed and rescaled into `[-1, 1]`.
///
/// Each octave has twice the frequency and half the amplitude of the one
/// before. `octaves` is clamped to `1..=8`.
pub fn fractal_noise(seed: u64, x: f64, octaves: u32) -> f32 {
    let octaves = octaves.clamp(1, MAX_OCTAVES);
    let mut sum = 0.0f64;
    let mut total_weight = 0.0f64;
    let mut weight = 1.0f64;
    let mut frequency = 1.0f64;
    for k in 0..octaves as u64 {
        let shift =
            OCTAVE_SHIFT_RANGE * unit_from_seed(mix_seed(seed, k + OCTAVE_SHIFT_STREAM)) as f64;
        sum += weight * gradient_noise(mix_seed(seed, k), x * frequency + shift) as f64;
        total_weight += weight;
        weight *= 0.5;
        frequency *= 2.0;
    }
    ((sum / total_weight) as f32).clamp(-1.0, 1.0)
}

fn gradient(seed: u64, i: i64) -> f64 {
    2.0 * unit_from_seed(mix_seed(seed, i as u64)) as f64 - 1.0
}

fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn any_x() -> impl Strategy<Value = f64> {
        prop_oneof![
            8 => -1e12f64..1e12,
            1 => Just(f64::NAN),
            1 => Just(f64::INFINITY),
            1 => Just(f64::NEG_INFINITY),
        ]
    }

    proptest! {
        #[test]
        fn gradient_noise_stays_in_range_vanishes_at_integers_and_is_continuous(
            seed in any::<u64>(),
            x in any_x(),
            i in -1_000_000i64..1_000_000,
        ) {
            let value = gradient_noise(seed, x);
            if x.is_finite() {
                prop_assert!((-1.0..=1.0).contains(&value), "{} out of range at {}", value, x);
            } else {
                prop_assert_eq!(value, 0.0);
            }
            prop_assert_eq!(gradient_noise(seed, i as f64), 0.0);
            let below = gradient_noise(seed, i as f64 - 1e-6);
            let above = gradient_noise(seed, i as f64 + 1e-6);
            prop_assert!((below - above).abs() < 1e-4, "jump at {}: {} {}", i, below, above);
            let unit = unit_from_seed(seed);
            prop_assert!((0.0..1.0).contains(&unit), "{} out of range", unit);
        }

        #[test]
        fn fractal_noise_stays_in_range_with_its_octaves_clamped(
            seed in any::<u64>(),
            x in -1e6f64..1e6,
            octaves in 0u32..20,
        ) {
            let value = fractal_noise(seed, x, octaves);
            prop_assert!((-1.0..=1.0).contains(&value), "{} out of range", value);
            prop_assert_eq!(value, fractal_noise(seed, x, octaves.clamp(1, 8)));
            let shift = 1024.0 * unit_from_seed(mix_seed(seed, 1024)) as f64;
            prop_assert_eq!(
                fractal_noise(seed, x, 1),
                gradient_noise(mix_seed(seed, 0), x + shift)
            );
        }
    }

    #[test]
    fn draws_spread_over_their_range_and_differ_by_seed() {
        for seed in [0, 0xDEAD_BEEF] {
            let peak = (0..10_000)
                .map(|n| gradient_noise(seed, -50.0 + n as f64 * 0.0137).abs())
                .fold(0.0f32, f32::max);
            assert!(peak > 0.5, "peak {peak} never passed 0.5 for seed {seed}");
        }
        let differing = (0..10u64)
            .filter(|&pair| gradient_noise(2 * pair, 0.5) != gradient_noise(2 * pair + 1, 0.5))
            .count();
        assert!(differing >= 9, "only {differing} of 10 seed pairs differ");
        let mean = (0..10_000u64)
            .map(|seed| unit_from_seed(seed) as f64)
            .sum::<f64>()
            / 10_000.0;
        assert!((mean - 0.5).abs() < 0.02, "mean {mean}");
    }
}
