use concerto_motion_matching::filter::savitzky_golay;
use concerto_motion_matching::BakeError;
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

struct Window {
    count: usize,
    window: usize,
    refused: bool,
}

#[test]
fn a_window_must_be_odd_and_over_three_and_is_clamped_to_the_input() {
    let refused = |window| Window {
        count: 20,
        window,
        refused: true,
    };
    let rows = [
        refused(0),
        refused(1),
        refused(2),
        refused(3),
        refused(4),
        refused(10),
        Window {
            count: 20,
            window: 5,
            refused: false,
        },
        Window {
            count: 9,
            window: 61,
            refused: false,
        },
        Window {
            count: 2,
            window: 31,
            refused: false,
        },
        Window {
            count: 1,
            window: 5,
            refused: false,
        },
    ];

    for row in rows {
        let values: Vec<f32> = (0..row.count).map(|i| i as f32 * 0.5).collect();

        let smoothed = savitzky_golay(&values, row.window, 3);

        if row.refused {
            assert!(
                matches!(
                    smoothed,
                    Err(BakeError::InvalidWindow { window, .. }) if window == row.window
                ),
                "window {}",
                row.window
            );
        } else {
            let smoothed = smoothed.unwrap();
            assert_eq!(smoothed.len(), row.count, "window {}", row.window);
            assert!(
                smoothed.iter().all(|v| v.is_finite()),
                "window {}",
                row.window
            );
        }
    }
}
