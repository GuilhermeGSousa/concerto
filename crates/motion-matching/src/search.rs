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

/// The squared distance from `query` to `frame`, or `None` for a query of the wrong length, a frame outside the database or a database not derived.
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

/// The frame nearest `query` by squared distance and that distance; `current` seeds the best, and no frame within `ignore_surrounding` of it or in the last `ignore_range_end` of a range is a candidate; `None` for a database not derived.
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
    use super::*;
    use crate::test_support::{two_clip_database, unbaked, TURN_RATE};
    use crate::{channel::FeatureChannelData, test_support::Speed};

    const SURROUNDING: usize = 20;
    const RANGE_END: usize = 20;

    fn normalised(database: &MotionDatabase, yaw_rate: f32, speed: f32) -> Vec<f32> {
        let mut query = vec![yaw_rate, speed];
        database.normalise(&mut query).unwrap();
        query
    }

    fn reference(
        database: &MotionDatabase,
        query: &[f32],
        current: usize,
        surrounding: usize,
        range_end: usize,
    ) -> (usize, f32) {
        let distance = |frame: usize| -> f32 {
            database
                .normalised_features(frame)
                .unwrap()
                .iter()
                .zip(query)
                .map(|(feature, target)| (feature - target) * (feature - target))
                .sum()
        };
        let mut best = (current, distance(current));
        for range in database.ranges().unwrap() {
            for frame in range.start..range.end - range_end {
                if frame.abs_diff(current) < surrounding {
                    continue;
                }
                let value = distance(frame);
                if value < best.1 {
                    best = (frame, value);
                }
            }
        }
        best
    }

    #[test]
    fn the_database_has_a_walk_range_then_a_turn_range() {
        let database = two_clip_database();

        assert_eq!(
            database.ranges().unwrap().collect::<Vec<_>>(),
            [0..60, 60..120]
        );
    }

    #[test]
    fn a_nan_query_finds_no_frame() {
        let database = two_clip_database();
        let query = vec![f32::NAN; database.feature_dims().unwrap()];

        assert_eq!(
            search(&database, &query, Some(5), SURROUNDING, RANGE_END),
            None
        );
        assert_eq!(
            search(&database, &query, None, SURROUNDING, RANGE_END),
            None
        );
    }

    #[test]
    fn a_left_turn_query_picks_a_frame_of_the_turn_clip() {
        let database = two_clip_database();
        let query = normalised(&database, TURN_RATE, 0.0);

        let (frame, _) = search(&database, &query, Some(5), SURROUNDING, RANGE_END).unwrap();

        assert_eq!(database.entry_of(frame), Some(1), "frame {frame}");
    }

    #[test]
    fn a_forward_query_picks_a_frame_of_the_walk_clip() {
        let database = two_clip_database();
        let query = normalised(&database, 0.0, 1.0);

        let (frame, _) = search(&database, &query, Some(80), SURROUNDING, RANGE_END).unwrap();

        assert_eq!(database.entry_of(frame), Some(0), "frame {frame}");
    }

    #[test]
    fn the_last_frames_of_each_range_are_never_returned() {
        let database = two_clip_database();

        for target in 0..database.frame_count().unwrap() {
            let query = database.normalised_features(target).unwrap();
            let (frame, _) = search(&database, &query, Some(0), 0, RANGE_END).unwrap();
            let range = database.range_of(frame).unwrap();
            assert!(
                frame < range.end - RANGE_END,
                "query {target} gave frame {frame}"
            );
        }
    }

    #[test]
    fn frames_near_the_current_one_are_never_returned() {
        let database = two_clip_database();
        let current = 63;

        for target in 0..database.frame_count().unwrap() {
            let query = database.normalised_features(target).unwrap();
            let (frame, _) = search(&database, &query, Some(current), SURROUNDING, 0).unwrap();
            assert!(
                frame == current || frame.abs_diff(current) >= SURROUNDING,
                "query {target} gave frame {frame}"
            );
        }
    }

    #[test]
    fn a_frame_near_the_current_one_wins_with_no_surrounding_ignored() {
        let database = two_clip_database();
        let query = database.normalised_features(66).unwrap();

        let (frame, value) = search(&database, &query, Some(63), 0, 0).unwrap();

        assert_eq!(frame, 66);
        assert!(value < 1e-10);
    }

    #[test]
    fn the_early_out_returns_the_same_answer_as_the_full_sum() {
        let database = two_clip_database();

        for target in (0..database.frame_count().unwrap()).step_by(7) {
            for current in [0, 45, 100] {
                let mut query = database.normalised_features(target).unwrap();
                query[0] += 0.37;
                query[1] -= 0.21;

                let found =
                    search(&database, &query, Some(current), SURROUNDING, RANGE_END).unwrap();
                let expected = reference(&database, &query, current, SURROUNDING, RANGE_END);

                assert_eq!(found.0, expected.0, "target {target} current {current}");
                assert!((found.1 - expected.1).abs() < 1e-5);
            }
        }
    }

    #[test]
    fn the_current_frames_cost_seeds_the_best_even_inside_the_range_end() {
        let database = two_clip_database();
        let current = 55;
        let query = database.normalised_features(current).unwrap();

        let (frame, value) =
            search(&database, &query, Some(current), SURROUNDING, RANGE_END).unwrap();

        assert_eq!(frame, current);
        assert!(value < 1e-10);
    }

    #[test]
    fn a_tie_with_the_current_frame_keeps_the_current_frame() {
        let database = two_clip_database();
        let query = database.normalised_features(70).unwrap();

        let (frame, _) = search(&database, &query, Some(70), 0, 0).unwrap();

        assert_eq!(frame, 70);
    }

    #[test]
    fn a_query_of_the_wrong_length_finds_nothing() {
        let database = two_clip_database();

        assert_eq!(search(&database, &[0.0], Some(0), 0, 0), None);
    }

    #[test]
    fn a_range_end_longer_than_a_range_leaves_only_the_current_frame() {
        let database = two_clip_database();
        let query = database.normalised_features(10).unwrap();

        let (frame, _) = search(&database, &query, Some(3), 0, 500).unwrap();

        assert_eq!(frame, 3);
    }

    #[test]
    fn the_cost_of_a_frame_matches_the_search_cost_when_that_frame_wins() {
        let database = two_clip_database();
        let query = database.normalised_features(70).unwrap();

        let (frame, value) = search(&database, &query, Some(70), 0, 0).unwrap();

        assert_eq!(frame, 70);
        assert_eq!(cost_of(&database, &query, 70), Some(value));
    }

    #[test]
    fn the_cost_of_a_frame_is_its_distance_when_another_frame_wins() {
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

    #[test]
    fn the_cost_of_a_bad_query_or_frame_is_none() {
        let database = two_clip_database();
        let query = database.normalised_features(0).unwrap();

        assert_eq!(cost_of(&database, &[0.0], 0), None);
        assert_eq!(
            cost_of(&database, &query, database.frame_count().unwrap()),
            None
        );
    }

    #[test]
    fn a_database_not_derived_finds_and_costs_nothing() {
        let database = unbaked(two_clip_database());

        assert_eq!(search(&database, &[0.0, 0.0], Some(0), 0, 0), None);
        assert_eq!(cost_of(&database, &[0.0, 0.0], 0), None);
    }

    #[test]
    fn a_weight_changed_after_the_bake_scales_its_group() {
        let database = two_clip_database();
        let mut reweighed = database.clone();
        reweighed.channels[1] = FeatureChannelData::live(Speed { weight: 4.0 });

        let scales = database.scales().unwrap();
        let rescaled = reweighed.scales().unwrap();

        assert_eq!(rescaled[0], scales[0]);
        assert!((rescaled[1] * 4.0 - scales[1]).abs() < 1e-6);
        let query = reweighed.normalised_features(70).unwrap();
        assert_eq!(search(&reweighed, &query, Some(70), 0, 0).unwrap().0, 70);
    }
}
