use concerto_animation::clip::{AnimationChanelOutput, AnimationChannel, AnimationClip};
use concerto_mesh::{JointPose, Skeleton};
use concerto_motion_matching::bake::{plan_entries, sample_pose};
use concerto_motion_matching::{BakeError, MotionDatabaseDefinition, MotionEntry, TimeRange};
use glam::{Mat4, Vec3};
use uuid::Uuid;

fn skeleton() -> Skeleton {
    Skeleton::new(
        vec![Mat4::IDENTITY; 2],
        vec![None, Some(0)],
        vec!["Root".into(), "Child".into()],
        vec![
            JointPose::IDENTITY,
            JointPose {
                translation: Vec3::new(0.0, 1.0, 0.0),
                ..JointPose::IDENTITY
            },
        ],
        vec![Uuid::from_u128(1), Uuid::from_u128(2)],
    )
    .unwrap()
}

fn walk_clip(duration: f32) -> AnimationClip {
    let mut clip = AnimationClip::default();
    clip.add_channel(
        Uuid::from_u128(1),
        AnimationChannel::new(
            vec![0.0, duration],
            AnimationChanelOutput::Translation(vec![Vec3::ZERO, Vec3::new(0.0, 0.0, -duration)]),
        ),
    );
    clip
}

fn definition(entries: Vec<MotionEntry>, rate: f32, speed: f32) -> MotionDatabaseDefinition {
    MotionDatabaseDefinition {
        entries,
        rate,
        speed,
        ..Default::default()
    }
}

fn entry(range: Option<(f32, f32)>) -> MotionEntry {
    MotionEntry {
        clip: Default::default(),
        range: range.map(|(start, end)| TimeRange { start, end }),
    }
}

#[test]
fn ranges_are_contiguous_for_whole_clips_and_sub_ranges() {
    let skeleton = skeleton();
    let first = walk_clip(1.0);
    let second = walk_clip(2.0);
    let definition = definition(vec![entry(None), entry(Some((0.5, 1.5)))], 60.0, 1.0);

    let plans = plan_entries(&definition, &skeleton, &[&first, &second]).unwrap();

    assert_eq!(plans.len(), 2);
    assert_eq!((plans[0].first_frame, plans[0].frames), (0, 60));
    assert_eq!((plans[1].first_frame, plans[1].frames), (60, 60));
    assert_eq!(plans[1].start, 0.5);
}

#[test]
fn speed_scales_the_frame_count() {
    let skeleton = skeleton();
    let clip = walk_clip(1.0);
    let definition = definition(vec![entry(None)], 60.0, 2.0);

    let plans = plan_entries(&definition, &skeleton, &[&clip]).unwrap();

    assert_eq!(plans[0].frames, 30);
    assert_eq!(plans[0].frame_time(3, 60.0, 2.0), 0.1);
}

#[test]
fn speed_one_reproduces_the_clip_pose() {
    let skeleton = skeleton();
    let clip = walk_clip(1.0);
    let definition = definition(vec![entry(None)], 60.0, 1.0);
    let plan = plan_entries(&definition, &skeleton, &[&clip])
        .unwrap()
        .remove(0);

    let time = plan.frame_time(30, 60.0, 1.0);
    let pose = sample_pose(&skeleton, &clip, time);

    assert!((pose[0].translation.z + 0.5).abs() < 1e-5);
    assert_eq!(pose[1], skeleton.rest_pose[1]);
}

#[test]
fn speed_two_samples_twice_as_far() {
    let skeleton = skeleton();
    let clip = walk_clip(1.0);
    let definition = definition(vec![entry(None)], 60.0, 2.0);
    let plan = plan_entries(&definition, &skeleton, &[&clip])
        .unwrap()
        .remove(0);

    let pose = sample_pose(&skeleton, &clip, plan.frame_time(15, 60.0, 2.0));

    assert!((pose[0].translation.z + 0.5).abs() < 1e-5);
}

#[test]
fn a_range_offsets_the_sampled_time() {
    let skeleton = skeleton();
    let clip = walk_clip(2.0);
    let definition = definition(vec![entry(Some((0.5, 1.5)))], 60.0, 1.0);
    let plan = plan_entries(&definition, &skeleton, &[&clip])
        .unwrap()
        .remove(0);

    let pose = sample_pose(&skeleton, &clip, plan.frame_time(0, 60.0, 1.0));

    assert!((pose[0].translation.z + 0.5).abs() < 1e-5);
}

#[test]
fn rate_and_speed_must_be_positive() {
    let skeleton = skeleton();
    let clip = walk_clip(1.0);
    for rate in [0.0, -1.0, f32::NAN] {
        let definition = definition(vec![entry(None)], rate, 1.0);
        let error = plan_entries(&definition, &skeleton, &[&clip])
            .err()
            .unwrap();
        assert!(matches!(error, BakeError::InvalidRate(_)));
    }
    for speed in [0.0, -1.0, f32::NAN] {
        let definition = definition(vec![entry(None)], 60.0, speed);
        let error = plan_entries(&definition, &skeleton, &[&clip])
            .err()
            .unwrap();
        assert!(matches!(error, BakeError::InvalidSpeed(_)));
    }
}

#[test]
fn the_clip_count_must_match_the_entries() {
    let skeleton = skeleton();
    let clip = walk_clip(1.0);
    let definition = definition(vec![entry(None), entry(None)], 60.0, 1.0);
    let error = plan_entries(&definition, &skeleton, &[&clip])
        .err()
        .unwrap();
    assert_eq!(
        error,
        BakeError::ClipCountMismatch {
            expected: 2,
            actual: 1
        }
    );
}

#[test]
fn a_clip_without_the_skeletons_bones_is_an_error() {
    let skeleton = skeleton();
    let mut clip = AnimationClip::default();
    clip.add_channel(
        Uuid::from_u128(99),
        AnimationChannel::new(
            vec![0.0, 1.0],
            AnimationChanelOutput::Translation(vec![Vec3::ZERO, Vec3::X]),
        ),
    );
    let definition = definition(vec![entry(None)], 60.0, 1.0);
    let error = plan_entries(&definition, &skeleton, &[&clip])
        .err()
        .unwrap();
    assert_eq!(error, BakeError::ClipHasNoBones { entry: 0 });
}

#[test]
fn invalid_ranges_are_errors() {
    let skeleton = skeleton();
    let clip = walk_clip(1.0);
    for range in [(0.5, 0.5), (0.8, 0.2), (-0.1, 0.5), (0.5, 1.5)] {
        let definition = definition(vec![entry(Some(range))], 60.0, 1.0);
        let error = plan_entries(&definition, &skeleton, &[&clip])
            .err()
            .unwrap();
        assert_eq!(error, BakeError::InvalidRange { entry: 0 }, "{range:?}");
    }
}

#[test]
fn a_range_under_two_frames_is_an_error() {
    let skeleton = skeleton();
    let clip = walk_clip(1.0);
    let definition = definition(vec![entry(Some((0.0, 0.02)))], 60.0, 1.0);
    let error = plan_entries(&definition, &skeleton, &[&clip])
        .err()
        .unwrap();
    assert_eq!(
        error,
        BakeError::RangeTooShort {
            entry: 0,
            frames: 1
        }
    );
}
