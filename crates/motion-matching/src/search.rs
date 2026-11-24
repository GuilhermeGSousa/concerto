use crate::database::MotionDatabase;

struct Rows<'a> {
    values: &'a [f32],
    dims: usize,
}

impl<'a> Rows<'a> {
    fn get(&self, frame: usize) -> Option<&'a [f32]> {
        self.values.get(frame * self.dims..(frame + 1) * self.dims)
    }
}

struct Found {
    frame: usize,
    cost: f32,
}

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

fn distance(features: &[f32], query: &[f32], best: f32) -> Option<f32> {
    let mut sum = 0.0;
    for (feature, target) in features.iter().zip(query) {
        let difference = feature - target;
        sum += difference * difference;
        if sum >= best {
            return None;
        }
    }
    sum.is_finite().then_some(sum)
}

fn scan(
    database: &MotionDatabase,
    current: Option<usize>,
    ignore_surrounding: usize,
    ignore_range_end: usize,
    frame_cost: impl Fn(usize, f32) -> Option<f32>,
) -> Option<Found> {
    let current = current.filter(|current| *current < database.frame_count().unwrap_or(0));
    let mut best = None;
    if let Some(frame) = current {
        let cost = frame_cost(frame, f32::INFINITY)?;
        best = Some(Found { frame, cost });
    }
    for range in database.ranges()? {
        let end = range.end.saturating_sub(ignore_range_end).max(range.start);
        for frame in range.start..end {
            if current.is_some_and(|current| frame.abs_diff(current) < ignore_surrounding) {
                continue;
            }
            let limit = best.as_ref().map_or(f32::INFINITY, |best| best.cost);
            if let Some(cost) = frame_cost(frame, limit) {
                best = Some(Found { frame, cost });
            }
        }
    }
    best
}

/// The squared distance from `query` to `frame`, or `None` for a query of the wrong length, a frame outside the database or a database not derived.
pub fn cost_of(database: &MotionDatabase, query: &[f32], frame: usize) -> Option<f32> {
    let dims = database.feature_dims()?;
    if query.len() != dims {
        return None;
    }
    if let Some(values) = database.prescaled() {
        return distance(Rows { values, dims }.get(frame)?, query, f32::INFINITY);
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
    let dims = database.feature_dims()?;
    if query.len() != dims {
        return None;
    }
    let found = if let Some(values) = database.prescaled() {
        let rows = Rows { values, dims };
        scan(
            database,
            current,
            ignore_surrounding,
            ignore_range_end,
            |frame, best| distance(rows.get(frame)?, query, best),
        )
    } else {
        let divisors = database.divisors()?;
        let offset = database.offset()?;
        scan(
            database,
            current,
            ignore_surrounding,
            ignore_range_end,
            |frame, best| cost(database.features(frame)?, offset, &divisors, query, best),
        )
    };
    found.map(|found| (found.frame, found.cost))
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::channel::FeatureChannelData;
    use crate::database::{BakedDatabase, Span};
    use crate::test_support::{two_clip_database, YawRate};

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

    #[derive(Clone, Debug)]
    struct Group {
        dims: usize,
        deviation: f32,
        weight: f32,
        reweight: f32,
    }

    #[derive(Clone, Debug)]
    struct Case {
        groups: Vec<Group>,
        frames: usize,
        split: usize,
        values: Vec<f32>,
    }

    impl Case {
        fn dims(&self) -> usize {
            self.groups.iter().map(|group| group.dims).sum()
        }

        fn ranges(&self) -> [std::ops::Range<usize>; 2] {
            let split = self.split.min(self.frames);
            [0..split, split..self.frames]
        }

        fn features(&self) -> &[f32] {
            &self.values[..self.frames * self.dims()]
        }

        fn offset(&self) -> &[f32] {
            &self.values[self.values.len() - self.dims()..]
        }

        fn channels(&self, weight: impl Fn(&Group) -> f32) -> Vec<FeatureChannelData> {
            self.groups
                .iter()
                .map(|group| {
                    FeatureChannelData::live(YawRate {
                        weight: weight(group),
                    })
                })
                .collect()
        }

        fn database(&self) -> MotionDatabase {
            let mut spans = Vec::new();
            let mut start = 0;
            for group in &self.groups {
                spans.push(Span::new(start, group.dims));
                start += group.dims;
            }
            let ranges = self.ranges();
            let baked = BakedDatabase::new(
                1,
                self.frames,
                ranges.iter().map(|range| range.start).collect(),
                ranges.iter().map(|range| range.end).collect(),
                vec![0.0; 2],
                vec![Default::default(); self.frames],
                self.features().to_vec(),
                self.dims(),
                spans,
                self.offset().to_vec(),
                self.groups.iter().map(|group| group.deviation).collect(),
            );
            MotionDatabase {
                channels: self.channels(|group| group.weight),
                ..Default::default()
            }
            .with_baked(baked)
        }

        fn divisors(&self, weight: impl Fn(&Group) -> f32) -> Vec<f32> {
            self.groups
                .iter()
                .flat_map(|group| {
                    let divisor = if group.deviation < 1e-6 {
                        f32::INFINITY
                    } else {
                        group.deviation / weight(group)
                    };
                    std::iter::repeat_n(divisor, group.dims)
                })
                .collect()
        }

        fn reference(
            &self,
            divisors: &[f32],
            query: &[f32],
            current: Option<usize>,
            surrounding: usize,
            range_end: usize,
        ) -> Option<Nearest> {
            let dims = self.dims();
            let current = current.filter(|current| *current < self.frames);
            let mut candidates: Vec<usize> = current.into_iter().collect();
            for range in self.ranges() {
                let end = range.end.saturating_sub(range_end).max(range.start);
                candidates.extend((range.start..end).filter(|frame| {
                    current.is_none_or(|current| frame.abs_diff(current) >= surrounding)
                }));
            }
            let mut best: Option<Nearest> = None;
            for frame in candidates {
                let mut cost = 0.0;
                for dim in 0..dims {
                    let feature = self.features()[frame * dims + dim];
                    let difference = (feature - self.offset()[dim]) / divisors[dim] - query[dim];
                    cost += difference * difference;
                }
                if best.as_ref().is_none_or(|best| cost < best.cost) {
                    best = Some(Nearest { frame, cost });
                }
            }
            best
        }
    }

    struct Variant<'a> {
        database: &'a MotionDatabase,
        divisors: Vec<f32>,
    }

    fn group() -> impl Strategy<Value = Group> {
        (
            1usize..4,
            prop_oneof![Just(0.0f32), Just(5e-7f32), 0.01f32..4.0],
            0.1f32..8.0,
            0.1f32..8.0,
        )
            .prop_map(|(dims, deviation, weight, reweight)| Group {
                dims,
                deviation,
                weight,
                reweight,
            })
    }

    fn case() -> impl Strategy<Value = Case> {
        (
            prop::collection::vec(group(), 1..4),
            1usize..14,
            0usize..14,
            prop::collection::vec((-8i8..8).prop_map(|value| value as f32 * 0.25), 14 * 9 + 9),
        )
            .prop_map(|(groups, frames, split, values)| Case {
                groups,
                frames,
                split,
                values,
            })
    }

    proptest! {
        #[test]
        fn the_search_returns_the_frame_and_cost_of_a_plain_scan_whatever_the_weights_and_deviations(
            case in case(),
            query in prop::collection::vec((-8i8..8).prop_map(|value| value as f32 * 0.25), 9),
            current in prop::option::of(0usize..16),
            surrounding in 0usize..5,
            range_end in 0usize..4,
        ) {
            let query = &query[..case.dims()];
            let loaded = case.database();
            let reweighted = MotionDatabase {
                channels: case.channels(|group| group.reweight),
                ..loaded.clone()
            };
            let variants = [
                Variant { database: &loaded, divisors: case.divisors(|group| group.weight) },
                Variant { database: &reweighted, divisors: case.divisors(|group| group.reweight) },
            ];

            for variant in variants {
                let found = search(variant.database, query, current, surrounding, range_end);
                let expected =
                    case.reference(&variant.divisors, query, current, surrounding, range_end);

                prop_assert_eq!(
                    found.map(|found| found.0),
                    expected.as_ref().map(|nearest| nearest.frame)
                );
                prop_assert_eq!(
                    found.map(|found| found.1.to_bits()),
                    expected.as_ref().map(|nearest| nearest.cost.to_bits())
                );
                let cost = found.and_then(|found| cost_of(variant.database, query, found.0));
                prop_assert_eq!(
                    cost.map(f32::to_bits),
                    expected.map(|nearest| nearest.cost.to_bits())
                );
            }
        }

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
