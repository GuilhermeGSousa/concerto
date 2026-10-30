use concerto_motion_matching::filter::{median_filter, savitzky_golay};
use concerto_motion_matching::BakeError;

fn cubic(x: f32) -> f32 {
    0.002 * x * x * x - 0.05 * x * x + 0.3 * x + 1.0
}

fn variance(values: &[f32]) -> f32 {
    let mean = values.iter().sum::<f32>() / values.len() as f32;
    values.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / values.len() as f32
}

#[test]
fn reproduces_a_cubic_including_the_edges() {
    let values: Vec<f32> = (0..40).map(|i| cubic(i as f32)).collect();
    let smoothed = savitzky_golay(&values, 11, 3).unwrap();
    assert_eq!(smoothed.len(), values.len());
    for (a, b) in smoothed.iter().zip(&values) {
        assert!((a - b).abs() < 1e-2, "{a} != {b}");
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

#[test]
fn clamps_a_window_longer_than_the_input() {
    let values: Vec<f32> = (0..9).map(|i| cubic(i as f32)).collect();
    let smoothed = savitzky_golay(&values, 61, 3).unwrap();
    assert_eq!(smoothed.len(), 9);
    for (a, b) in smoothed.iter().zip(&values) {
        assert!((a - b).abs() < 1e-2, "{a} != {b}");
    }
}

#[test]
fn clamps_to_an_input_shorter_than_the_order() {
    let smoothed = savitzky_golay(&[1.0, 2.0], 31, 3).unwrap();
    assert_eq!(smoothed.len(), 2);
    assert!(smoothed.iter().all(|v| v.is_finite()));
}

#[test]
fn rejects_even_and_too_small_windows() {
    let values = vec![0.0; 20];
    for window in [0, 1, 2, 3, 4, 10] {
        assert!(
            matches!(
                savitzky_golay(&values, window, 3),
                Err(BakeError::InvalidWindow { window: w, .. }) if w == window
            ),
            "window {window}"
        );
    }
    assert!(savitzky_golay(&values, 5, 3).is_ok());
}

#[test]
fn median_removes_a_one_frame_glitch() {
    let mut values = vec![true; 12];
    values[6] = false;
    assert_eq!(median_filter(&values, 5), vec![true; 12]);

    let mut values = vec![false; 12];
    values[6] = true;
    assert_eq!(median_filter(&values, 5), vec![false; 12]);
}

#[test]
fn median_keeps_the_edges() {
    let values = [true, true, true, false, false, false];
    assert_eq!(median_filter(&values, 3), values.to_vec());
    let values = [false, false, true, true, true, true];
    assert_eq!(median_filter(&values, 3), values.to_vec());
}

#[test]
fn median_with_an_even_window_is_true_at_half() {
    let values = [true, true, true, false, false, false, false, false];
    let filtered = median_filter(&values, 6);
    assert_eq!(filtered.len(), values.len());
    assert!(filtered[0]);
    assert!(!filtered[7]);
}

#[test]
fn median_of_a_trivial_window_is_unchanged() {
    let values = [true, false, true];
    assert_eq!(median_filter(&values, 0), values.to_vec());
    assert_eq!(median_filter(&values, 1), values.to_vec());
    assert!(median_filter(&[], 5).is_empty());
}
