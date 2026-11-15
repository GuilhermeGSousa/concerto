use concerto_motion_matching::filter::savitzky_golay;
use proptest::prelude::*;

fn variance(values: &[f32]) -> f32 {
    let mean = values.iter().sum::<f32>() / values.len() as f32;
    values.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / values.len() as f32
}

proptest! {
    #[test]
    fn a_cubic_comes_through_unchanged_edges_included(
        coefficients in prop::array::uniform4(-1.0f32..1.0),
        count in 4usize..60,
        half_window in 2usize..40,
    ) {
        let cubic = |i: usize| {
            let x = i as f32 / count as f32;
            coefficients[0] + x * (coefficients[1] + x * (coefficients[2] + x * coefficients[3]))
        };
        let values: Vec<f32> = (0..count).map(cubic).collect();

        let smoothed = savitzky_golay(&values, 2 * half_window + 1, 3).unwrap();

        prop_assert_eq!(smoothed.len(), count);
        for (smooth, value) in smoothed.iter().zip(&values) {
            prop_assert!((smooth - value).abs() < 1e-3, "{} != {}", smooth, value);
        }
    }
}

#[test]
fn reduces_the_variance_of_a_noisy_line() {
    let noise = |i: usize| if i.is_multiple_of(2) { 0.5 } else { -0.5 };
    let values: Vec<f32> = (0..60).map(|i| 0.1 * i as f32 + noise(i)).collect();
    let smoothed = savitzky_golay(&values, 11, 3).unwrap();
    let residual = |series: &[f32]| -> Vec<f32> {
        series
            .iter()
            .enumerate()
            .map(|(i, v)| v - 0.1 * i as f32)
            .collect()
    };
    assert!(variance(&residual(&smoothed)) < variance(&residual(&values)) * 0.2);
}
