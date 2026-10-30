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
    use concerto_animation::clip::{AnimationChanelOutput, AnimationChannel, AnimationClip};
    use concerto_mesh::{JointPose, Skeleton};
    use concerto_reflect::{to_value, Reflect, TypeRegistry};
    use glam::{Mat4, Quat, Vec3};
    use uuid::Uuid;

    use super::*;
    use crate::{
        bake::bake,
        channel::{FeatureChannel, FrameContext, ReflectFeatureChannel},
        definition::{
            MotionDatabaseDefinition, MotionEntry, SimulationBoneSettings, StoredChannel,
            TrajectorySettings,
        },
    };

    const TURN_RATE: f32 = 1.5;

    #[derive(Reflect, Default, Clone, Debug, PartialEq)]
    #[reflect(FeatureChannel)]
    struct YawRate {
        weight: f32,
    }

    impl FeatureChannel for YawRate {
        fn dims(&self) -> usize {
            1
        }

        fn extract(&self, frame: &FrameContext, out: &mut [f32]) {
            out[0] = frame.body.angular_velocity.y;
        }

        fn weight(&self) -> f32 {
            self.weight
        }
    }

    #[derive(Reflect, Default, Clone, Debug, PartialEq)]
    #[reflect(FeatureChannel)]
    struct Speed {
        weight: f32,
    }

    impl FeatureChannel for Speed {
        fn dims(&self) -> usize {
            1
        }

        fn extract(&self, frame: &FrameContext, out: &mut [f32]) {
            out[0] = frame.body.velocity.length();
        }

        fn weight(&self) -> f32 {
            self.weight
        }
    }

    fn skeleton() -> Skeleton {
        Skeleton::new(
            vec![Mat4::IDENTITY; 3],
            vec![None, Some(0), Some(1)],
            vec!["Root".into(), "Hips".into(), "Spine2".into()],
            vec![
                JointPose::IDENTITY,
                JointPose {
                    translation: Vec3::new(0.0, 1.0, 0.0),
                    rotation: Quat::from_rotation_y(std::f32::consts::PI),
                    ..JointPose::IDENTITY
                },
                JointPose {
                    translation: Vec3::new(0.0, 0.3, 0.0),
                    ..JointPose::IDENTITY
                },
            ],
            vec![Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3)],
        )
        .unwrap()
    }

    fn walk_forward() -> AnimationClip {
        let mut clip = AnimationClip::default();
        clip.add_channel(
            Uuid::from_u128(1),
            AnimationChannel::new(
                vec![0.0, 2.0],
                AnimationChanelOutput::Translation(vec![Vec3::ZERO, Vec3::new(0.0, 0.0, -2.0)]),
            ),
        );
        clip
    }

    fn turn_left() -> AnimationClip {
        let mut clip = AnimationClip::default();
        let keys = 5;
        let times = (0..keys).map(|key| key as f32 * 0.5).collect();
        let rotations = (0..keys)
            .map(|key| Quat::from_rotation_y(std::f32::consts::PI + TURN_RATE * key as f32 * 0.5))
            .collect();
        clip.add_channel(
            Uuid::from_u128(2),
            AnimationChannel::new(times, AnimationChanelOutput::Rotation(rotations)),
        );
        clip
    }

    fn registry() -> TypeRegistry {
        let mut registry = TypeRegistry::default();
        registry.register::<YawRate>();
        registry.register::<Speed>();
        registry
    }

    fn two_clip_database() -> MotionDatabase {
        let definition = MotionDatabaseDefinition {
            entries: vec![
                MotionEntry {
                    clip: Default::default(),
                    range: None,
                },
                MotionEntry {
                    clip: Default::default(),
                    range: None,
                },
            ],
            rate: 30.0,
            speed: 1.0,
            simulation: SimulationBoneSettings {
                position_window: 5,
                facing_window: 5,
                ..Default::default()
            },
            trajectory: TrajectorySettings {
                sample_dt: 0.25,
                sample_count: 3,
            },
            channels: vec![
                StoredChannel {
                    type_name: "YawRate".into(),
                    value: to_value(&YawRate { weight: 1.0 }).unwrap(),
                },
                StoredChannel {
                    type_name: "Speed".into(),
                    value: to_value(&Speed { weight: 1.0 }).unwrap(),
                },
            ],
            ..Default::default()
        };
        bake(
            &definition,
            &skeleton(),
            &[&walk_forward(), &turn_left()],
            &registry(),
        )
        .unwrap()
    }

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
}
