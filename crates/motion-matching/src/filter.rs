use crate::database::BakeError;

/// Smooths `values` with a Savitzky-Golay filter, fitting a polynomial over the first or last window at the edges.
pub fn savitzky_golay(values: &[f32], window: usize, order: usize) -> Result<Vec<f32>, BakeError> {
    if window % 2 == 0 || window <= 3 {
        return Err(BakeError::InvalidWindow {
            name: "smoothing",
            window,
        });
    }
    let count = values.len();
    if count == 0 {
        return Ok(Vec::new());
    }
    let mut window = window.min(count);
    if window % 2 == 0 {
        window -= 1;
    }
    let order = order.min(window - 1);
    let half = window / 2;
    let terms = order + 1;

    let mut smoothed = Vec::with_capacity(count);
    for i in 0..count {
        let start = i.saturating_sub(half).min(count - window);
        let center = start as f64 + (window - 1) as f64 / 2.0;
        let mut matrix = vec![vec![0.0f64; terms + 1]; terms];
        for (offset, value) in values[start..start + window].iter().enumerate() {
            let x = start as f64 + offset as f64 - center;
            let powers: Vec<f64> = (0..=2 * order).map(|p| x.powi(p as i32)).collect();
            for (row, cells) in matrix.iter_mut().enumerate() {
                for (column, cell) in cells.iter_mut().take(terms).enumerate() {
                    *cell += powers[row + column];
                }
                cells[terms] += powers[row] * *value as f64;
            }
        }
        let coefficients = solve(matrix, terms);
        let x = i as f64 - center;
        let mut result = 0.0f64;
        for coefficient in coefficients.iter().rev() {
            result = result * x + coefficient;
        }
        smoothed.push(result as f32);
    }
    Ok(smoothed)
}

fn solve(mut matrix: Vec<Vec<f64>>, size: usize) -> Vec<f64> {
    for column in 0..size {
        let pivot = (column..size)
            .max_by(|a, b| {
                matrix[*a][column]
                    .abs()
                    .total_cmp(&matrix[*b][column].abs())
            })
            .unwrap_or(column);
        matrix.swap(column, pivot);
        let divisor = matrix[column][column];
        if divisor.abs() < 1e-12 {
            continue;
        }
        let pivot_row = matrix[column].clone();
        for (row, cells) in matrix.iter_mut().enumerate() {
            if row == column {
                continue;
            }
            let factor = cells[column] / divisor;
            for (cell, pivot_cell) in cells.iter_mut().zip(&pivot_row).skip(column) {
                *cell -= factor * pivot_cell;
            }
        }
    }
    matrix
        .iter()
        .enumerate()
        .map(|(row, cells)| {
            let divisor = cells[row];
            if divisor.abs() < 1e-12 {
                0.0
            } else {
                cells[size] / divisor
            }
        })
        .collect()
}

/// Filters booleans with a median window, replicating the nearest value at the edges; a sample is true when at least half the window is.
pub fn median_filter(values: &[bool], window: usize) -> Vec<bool> {
    if window <= 1 || values.is_empty() {
        return values.to_vec();
    }
    let last = values.len() - 1;
    let before = window / 2;
    (0..values.len())
        .map(|i| {
            let trues = (0..window)
                .filter(|k| {
                    let index = (i + k).saturating_sub(before).min(last);
                    values[index]
                })
                .count();
            trues * 2 >= window
        })
        .collect()
}
