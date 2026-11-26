use crate::database::MotionDatabase;

fn cost(
    features: &[f32],
    offset: &[f32],
    divisors: &[f32],
    query: &[f32],
    best: f32,
) -> Option<f32> {
    let mut sum = 0.0;
    for (((feature, offset), divisor), target) in
        features.iter().zip(offset).zip(divisors).zip(query)
    {
        let difference = (feature - offset) / divisor - target;
        sum += difference * difference;
        if sum >= best {
            return None;
        }
    }
    sum.is_finite().then_some(sum)
}

/// The squared distance from `query` to `frame`.
///
/// `None` for a query of the wrong length, a frame outside the database or a
/// database not derived.
pub fn cost_of(database: &MotionDatabase, query: &[f32], frame: usize) -> Option<f32> {
    if query.len() != database.feature_dims()? {
        return None;
    }
    cost(
        database.features(frame)?,
        database.offset()?,
        &database.divisors()?,
        query,
        f32::INFINITY,
    )
}

/// The frame nearest `query` by squared distance, and that distance.
///
/// `current` seeds the best. No frame within `ignore_surrounding` frames of
/// it, or in the last `ignore_range_end` frames of a range, is a candidate.
/// `None` for a query of the wrong length or a database not derived.
pub fn search(
    database: &MotionDatabase,
    query: &[f32],
    current: Option<usize>,
    ignore_surrounding: usize,
    ignore_range_end: usize,
) -> Option<(usize, f32)> {
    if query.len() != database.feature_dims()? {
        return None;
    }
    let divisors = database.divisors()?;
    let offset = database.offset()?;
    let frame_cost =
        |frame: usize, best: f32| cost(database.features(frame)?, offset, &divisors, query, best);
    let current = current.filter(|current| *current < database.frame_count().unwrap_or(0));
    let mut best = None;
    if let Some(current) = current {
        best = Some((current, frame_cost(current, f32::INFINITY)?));
    }
    for range in database.ranges()? {
        let end = range.end.saturating_sub(ignore_range_end).max(range.start);
        for frame in range.start..end {
            if current.is_some_and(|current| frame.abs_diff(current) < ignore_surrounding) {
                continue;
            }
            let limit = best.map_or(f32::INFINITY, |(_, best)| best);
            if let Some(value) = frame_cost(frame, limit) {
                best = Some((frame, value));
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::test_support::two_clip_database;

    struct Nearest {
        frame: usize,
        cost: f32,
    }

    fn brute_force(
        database: &MotionDatabase,
        query: &[f32],
        current: Option<usize>,
        surrounding: usize,
        range_end: usize,
    ) -> Option<Nearest> {
        let mut candidates: Vec<usize> = current.into_iter().collect();
        for range in database.ranges().unwrap() {
            let end = range.end.saturating_sub(range_end).max(range.start);
            candidates.extend((range.start..end).filter(|frame| {
                current.is_none_or(|current| frame.abs_diff(current) >= surrounding)
            }));
        }
        let mut best: Option<Nearest> = None;
        for frame in candidates {
            let cost = cost_of(database, query, frame)?;
            if best.as_ref().is_none_or(|best| cost < best.cost) {
                best = Some(Nearest { frame, cost });
            }
        }
        best
    }

    proptest! {
        #[test]
        fn the_search_finds_the_nearest_frame_a_brute_force_finds_outside_the_ignored_frames(
            target in 0usize..120,
            nudge in prop::array::uniform2(-1.0f32..1.0),
            current in prop::option::of(0usize..120),
            surrounding in 0usize..30,
            range_end in 0usize..70,
        ) {
            let database = two_clip_database();
            let mut query = database.normalised_features(target).unwrap();
            query[0] += nudge[0];
            query[1] += nudge[1];

            let found = search(&database, &query, current, surrounding, range_end);
            let expected = brute_force(&database, &query, current, surrounding, range_end);

            prop_assert_eq!(found.map(|found| found.0), expected.as_ref().map(|nearest| nearest.frame));
            prop_assert_eq!(found.map(|found| found.1), expected.map(|nearest| nearest.cost));
        }
    }

    #[test]
    fn the_cost_of_a_frame_is_its_squared_distance_from_the_query() {
        let database = two_clip_database();
        let query = database.normalised_features(70).unwrap();
        let expected: f32 = database
            .normalised_features(10)
            .unwrap()
            .iter()
            .zip(&query)
            .map(|(feature, target)| (feature - target) * (feature - target))
            .sum();

        let value = cost_of(&database, &query, 10).unwrap();

        assert!((value - expected).abs() < 1e-5);
        assert!(value > cost_of(&database, &query, 70).unwrap());
    }
}
