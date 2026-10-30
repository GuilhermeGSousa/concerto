use crate::database::MotionDatabase;

/// How many frames at the end of each range are never search results: the trajectory window of the database, in frames.
pub fn excluded_tail(database: &MotionDatabase) -> usize {
    let settings = database.trajectory();
    (settings.sample_count as f32 * settings.sample_dt * database.rate()).ceil() as usize
}

fn cost(
    database: &MotionDatabase,
    divisors: &[f32],
    query: &[f32],
    frame: usize,
    best: f32,
) -> Option<f32> {
    let mut sum = 0.0;
    for (((feature, offset), divisor), target) in database
        .features(frame)
        .iter()
        .zip(database.offset())
        .zip(divisors)
        .zip(query)
    {
        let difference = (feature - offset) / divisor - target;
        sum += difference * difference;
        if sum >= best {
            return None;
        }
    }
    Some(sum)
}

/// The squared distance from `query` to `frame`, or `None` for a query of the wrong length or a frame outside the database.
pub fn cost_of(database: &MotionDatabase, query: &[f32], frame: usize) -> Option<f32> {
    if query.len() != database.feature_dims() || frame >= database.frame_count() {
        return None;
    }
    cost(database, &database.divisors(), query, frame, f32::INFINITY)
}

/// The frame nearest `query` by squared distance and that distance; `current_cost_frame` seeds the best, and the last `tail` frames of each range are never candidates.
pub fn search(
    database: &MotionDatabase,
    query: &[f32],
    current_cost_frame: usize,
    tail: usize,
) -> Option<(usize, f32)> {
    if query.len() != database.feature_dims() {
        return None;
    }
    let divisors = database.divisors();
    let mut best = None;
    if current_cost_frame < database.frame_count() {
        let seed = cost(
            database,
            &divisors,
            query,
            current_cost_frame,
            f32::INFINITY,
        )?;
        best = Some((current_cost_frame, seed));
    }
    for range in database.ranges() {
        let end = range.end.saturating_sub(tail).max(range.start);
        for frame in range.start..end {
            let limit = best.map_or(f32::INFINITY, |(_, best)| best);
            if let Some(value) = cost(database, &divisors, query, frame, limit) {
                best = Some((frame, value));
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{two_clip_database, TURN_RATE};

    fn normalised(database: &MotionDatabase, yaw_rate: f32, speed: f32) -> Vec<f32> {
        let mut query = vec![yaw_rate, speed];
        database.normalise(&mut query);
        query
    }

    fn reference(
        database: &MotionDatabase,
        query: &[f32],
        current: usize,
        tail: usize,
    ) -> (usize, f32) {
        let distance = |frame: usize| -> f32 {
            database
                .normalised_features(frame)
                .iter()
                .zip(query)
                .map(|(feature, target)| (feature - target) * (feature - target))
                .sum()
        };
        let mut best = (current, distance(current));
        for range in database.ranges() {
            for frame in range.start..range.end - tail {
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

        assert_eq!(database.ranges().collect::<Vec<_>>(), [0..60, 60..120]);
    }

    #[test]
    fn the_excluded_tail_is_the_trajectory_window_in_frames_rounded_up() {
        let database = two_clip_database();

        assert_eq!(excluded_tail(&database), 23);
    }

    #[test]
    fn a_left_turn_query_picks_a_frame_of_the_turn_clip() {
        let database = two_clip_database();
        let query = normalised(&database, TURN_RATE, 0.0);

        let (frame, _) = search(&database, &query, 5, excluded_tail(&database)).unwrap();

        assert_eq!(database.entry_of(frame), Some(1), "frame {frame}");
    }

    #[test]
    fn a_forward_query_picks_a_frame_of_the_walk_clip() {
        let database = two_clip_database();
        let query = normalised(&database, 0.0, 1.0);

        let (frame, _) = search(&database, &query, 80, excluded_tail(&database)).unwrap();

        assert_eq!(database.entry_of(frame), Some(0), "frame {frame}");
    }

    #[test]
    fn the_last_frames_of_each_range_are_never_returned() {
        let database = two_clip_database();
        let tail = excluded_tail(&database);

        for target in 0..database.frame_count() {
            let query = database.normalised_features(target);
            let (frame, _) = search(&database, &query, 0, tail).unwrap();
            let range = database.range_of(frame).unwrap();
            assert!(
                frame < range.end - tail,
                "query {target} gave frame {frame}"
            );
        }
    }

    #[test]
    fn the_early_out_returns_the_same_answer_as_the_full_sum() {
        let database = two_clip_database();
        let tail = excluded_tail(&database);

        for target in (0..database.frame_count()).step_by(7) {
            for current in [0, 45, 100] {
                let mut query = database.normalised_features(target);
                query[0] += 0.37;
                query[1] -= 0.21;

                let found = search(&database, &query, current, tail).unwrap();
                let expected = reference(&database, &query, current, tail);

                assert_eq!(found.0, expected.0, "target {target} current {current}");
                assert!((found.1 - expected.1).abs() < 1e-5);
            }
        }
    }

    #[test]
    fn the_current_frames_cost_seeds_the_best_even_inside_the_tail() {
        let database = two_clip_database();
        let tail = excluded_tail(&database);
        let current = 55;
        let query = database.normalised_features(current);

        let (frame, value) = search(&database, &query, current, tail).unwrap();

        assert_eq!(frame, current);
        assert!(value < 1e-10);
    }

    #[test]
    fn a_tie_with_the_current_frame_keeps_the_current_frame() {
        let database = two_clip_database();
        let query = database.normalised_features(70);

        let (frame, _) = search(&database, &query, 70, 0).unwrap();

        assert_eq!(frame, 70);
    }

    #[test]
    fn a_query_of_the_wrong_length_finds_nothing() {
        let database = two_clip_database();

        assert_eq!(search(&database, &[0.0], 0, 0), None);
    }

    #[test]
    fn a_tail_longer_than_a_range_leaves_only_the_current_frame() {
        let database = two_clip_database();
        let query = database.normalised_features(10);

        let (frame, _) = search(&database, &query, 3, 500).unwrap();

        assert_eq!(frame, 3);
    }

    #[test]
    fn the_cost_of_a_frame_matches_the_search_cost_when_that_frame_wins() {
        let database = two_clip_database();
        let query = database.normalised_features(70);

        let (frame, value) = search(&database, &query, 70, 0).unwrap();

        assert_eq!(frame, 70);
        assert_eq!(cost_of(&database, &query, 70), Some(value));
    }

    #[test]
    fn the_cost_of_a_frame_is_its_distance_when_another_frame_wins() {
        let database = two_clip_database();
        let query = database.normalised_features(70);
        let expected: f32 = database
            .normalised_features(10)
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
        let query = database.normalised_features(0);

        assert_eq!(cost_of(&database, &[0.0], 0), None);
        assert_eq!(cost_of(&database, &query, database.frame_count()), None);
    }
}
