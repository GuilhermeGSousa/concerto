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

/// 1D gradient (Perlin) noise at `x`: in `[-1, 1]`, 0 at every integer, with continuous first and second derivatives.
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

/// `octaves` octaves of gradient noise at `x`, each twice the frequency and half the amplitude of the one before, summed and rescaled into `[-1, 1]`.
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

    const SEEDS: [u64; 2] = [0, 0xDEAD_BEEF];

    fn sample_x(n: u32) -> f64 {
        -50.0 + n as f64 * 0.0137
    }

    #[test]
    fn gradient_noise_is_zero_at_integers() {
        for seed in SEEDS {
            for i in -3..=3 {
                assert_eq!(gradient_noise(seed, i as f64), 0.0, "seed {seed} at {i}");
            }
        }
    }

    #[test]
    fn gradient_noise_spans_the_unit_range() {
        for seed in SEEDS {
            let mut peak = 0.0f32;
            for n in 0..10_000 {
                let value = gradient_noise(seed, sample_x(n));
                assert!((-1.0..=1.0).contains(&value), "{value} out of range");
                peak = peak.max(value.abs());
            }
            assert!(peak > 0.5, "peak {peak} never passed 0.5 for seed {seed}");
        }
    }

    #[test]
    fn gradient_noise_is_continuous_across_integers() {
        for seed in SEEDS {
            for i in -3..=3 {
                let below = gradient_noise(seed, i as f64 - 1e-6);
                let above = gradient_noise(seed, i as f64 + 1e-6);
                assert!((below - above).abs() < 1e-4, "jump at {i}: {below} {above}");
            }
        }
    }

    #[test]
    fn gradient_noise_depends_on_seed_alone() {
        assert_eq!(gradient_noise(7, 2.375), gradient_noise(7, 2.375));
        let differing = (0..10u64)
            .filter(|&pair| gradient_noise(2 * pair, 0.5) != gradient_noise(2 * pair + 1, 0.5))
            .count();
        assert!(differing >= 9, "only {differing} of 10 seed pairs differ");
    }

    #[test]
    fn gradient_noise_holds_far_from_the_origin() {
        for seed in SEEDS {
            let value = gradient_noise(seed, 1e9 + 0.25);
            assert!(value.is_finite());
            assert!((-1.0..=1.0).contains(&value));
        }
    }

    #[test]
    fn gradient_noise_of_non_finite_is_zero() {
        assert_eq!(gradient_noise(3, f64::NAN), 0.0);
        assert_eq!(gradient_noise(3, f64::INFINITY), 0.0);
    }

    #[test]
    fn fractal_noise_stays_in_range() {
        for octaves in [1, 3, 8] {
            for n in 0..10_000 {
                let value = fractal_noise(11, sample_x(n), octaves);
                assert!(
                    (-1.0..=1.0).contains(&value),
                    "{value} out of range with {octaves} octaves"
                );
            }
        }
    }

    #[test]
    fn fractal_noise_with_one_octave_is_shifted_gradient_noise() {
        let seed = 5;
        let shift = 1024.0 * unit_from_seed(mix_seed(seed, 1024)) as f64;
        for n in 0..100 {
            let x = sample_x(n * 37);
            assert_eq!(
                fractal_noise(seed, x, 1),
                gradient_noise(mix_seed(seed, 0), x + shift)
            );
        }
    }

    #[test]
    fn fractal_noise_clamps_octaves() {
        for n in 0..100 {
            let x = sample_x(n * 53);
            assert_eq!(fractal_noise(9, x, 0), fractal_noise(9, x, 1));
            assert_eq!(fractal_noise(9, x, 20), fractal_noise(9, x, 8));
        }
    }

    #[test]
    fn unit_from_seed_is_uniform_in_unit_interval() {
        let mut sum = 0.0f64;
        for seed in 0..10_000u64 {
            let value = unit_from_seed(seed);
            assert!((0.0..1.0).contains(&value), "{value} out of range");
            sum += value as f64;
        }
        let mean = sum / 10_000.0;
        assert!((mean - 0.5).abs() < 0.02, "mean {mean}");
    }
}
