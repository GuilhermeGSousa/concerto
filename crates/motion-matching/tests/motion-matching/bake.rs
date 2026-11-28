use concerto_animation::clip::{AnimationChanelOutput, AnimationChannel, AnimationClip};
use concerto_animation::mirror::{MirrorError, PoseMirror};
use concerto_foundation::transform::Transform;
use concerto_mesh::{JointPose, Skeleton};
use concerto_motion_matching::bake::{model_pose, model_velocities, reexpress_roots, sample_pose};
use concerto_motion_matching::{
    BakeError, BodyMotion, ChannelSource, FeatureChannel, FeatureChannelData, FrameContext,
    JointAxis, MotionDatabase, MotionEntry, ReflectFeatureChannel, SimulationBoneSettings,
    TimeRange, TrajectorySample, TrajectorySettings,
};
use concerto_reflect::Reflect;
use glam::{Mat4, Quat, Vec3};
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

fn baked(
    definition: &MotionDatabase,
    skeleton: &Skeleton,
    clips: &[&AnimationClip],
) -> MotionDatabase {
    let baked = definition.bake(skeleton, clips).unwrap();
    definition.clone().with_baked(baked)
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

fn entry(range: Option<TimeRange>) -> MotionEntry {
    MotionEntry {
        clip: Default::default(),
        range,
        mirrored: false,
    }
}

fn within(start: f32, end: f32) -> Option<TimeRange> {
    Some(TimeRange { start, end })
}

fn body_skeleton() -> Skeleton {
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

fn walk_model_poses(skeleton: &Skeleton, seconds: f32, rate: f32) -> Vec<Vec<Transform>> {
    let clip = walk_clip(seconds);
    let frames = (seconds * rate) as usize;
    (0..frames)
        .map(|frame| model_pose(skeleton, &sample_pose(skeleton, &clip, frame as f32 / rate)))
        .collect()
}

#[test]
fn a_straight_walk_faces_its_travel_and_moves_at_walk_speed() {
    let skeleton = body_skeleton();
    let poses = walk_model_poses(&skeleton, 2.0, 60.0);

    let settings = SimulationBoneSettings {
        facing_axis: JointAxis::Z,
        ..Default::default()
    };

    let simulation = settings.transforms(&skeleton, &poses).unwrap();

    assert_eq!(simulation.len(), poses.len());
    for (frame, transform) in simulation.iter().enumerate() {
        let facing = transform.rotation * Vec3::Z;
        assert!((facing - Vec3::NEG_Z).length() < 1e-3, "frame {frame}");
        assert!(transform.translation.y.abs() < 1e-6);
    }
    for pair in simulation.windows(2) {
        let speed = (pair[1].translation - pair[0].translation).length() * 60.0;
        assert!((speed - 1.0).abs() < 1e-2, "{speed}");
    }
}

fn moving_locals(frames: usize, step: Vec3, turn: f32) -> Vec<Vec<JointPose>> {
    (0..frames)
        .map(|frame| {
            vec![
                JointPose {
                    translation: step * frame as f32,
                    rotation: Quat::from_rotation_y(turn * frame as f32),
                    ..JointPose::IDENTITY
                },
                JointPose::IDENTITY,
            ]
        })
        .collect()
}

#[test]
fn model_velocities_follow_the_parent_chain() {
    let skeleton = skeleton();
    let model: Vec<Vec<Transform>> = moving_locals(4, Vec3::new(1.0, 0.0, 0.0), 0.0)
        .iter()
        .map(|local| model_pose(&skeleton, local))
        .collect();
    let velocities = model_velocities(&model, 2.0);
    for frame in &velocities {
        assert!(frame[1].distance(Vec3::new(2.0, 0.0, 0.0)) < 1e-4);
    }
}

#[derive(Reflect, Default, Clone, Debug, PartialEq)]
#[reflect(FeatureChannel)]
struct ProbeChannel {
    tag: f32,
}

impl FeatureChannel for ProbeChannel {
    fn dims(&self) -> usize {
        3
    }

    fn extract(&self, frame: &FrameContext, out: &mut [f32]) {
        out[0] = self.tag;
        out[1] = frame.model_pose.len() as f32;
        out[2] = frame.trajectory.len() as f32;
    }
}

#[derive(Reflect, Default, Clone, Debug, PartialEq)]
#[reflect(FeatureChannel)]
struct PoseChannel {
    unused: f32,
}

impl FeatureChannel for PoseChannel {
    fn dims(&self) -> usize {
        10
    }

    fn extract(&self, frame: &FrameContext, out: &mut [f32]) {
        out[0..3].copy_from_slice(&frame.local_pose[0].translation.to_array());
        out[3..7].copy_from_slice(&frame.local_pose[0].rotation.to_array());
        out[7..10].copy_from_slice(&frame.local_pose[1].translation.to_array());
    }
}

#[derive(Reflect, Default, Clone, Debug, PartialEq)]
#[reflect(FeatureChannel)]
struct PairChannel {
    gain: f32,
}

impl FeatureChannel for PairChannel {
    fn dims(&self) -> usize {
        2
    }

    fn extract(&self, frame: &FrameContext, out: &mut [f32]) {
        out[0] = frame.simulation.translation.z;
        out[1] = frame.model_velocities.len() as f32;
    }
}

fn probe(tag: f32) -> FeatureChannelData {
    FeatureChannelData::live(ProbeChannel { tag })
}

fn pose_channel() -> FeatureChannelData {
    FeatureChannelData::live(PoseChannel { unused: 0.0 })
}

fn pair() -> FeatureChannelData {
    FeatureChannelData::live(PairChannel { gain: 1.0 })
}

fn baking_definition(
    entries: Vec<MotionEntry>,
    channels: Vec<FeatureChannelData>,
) -> MotionDatabase {
    MotionDatabase {
        entries,
        rate: 30.0,
        speed: 1.0,
        simulation: SimulationBoneSettings {
            facing_axis: JointAxis::Z,
            position_window: 5,
            facing_window: 5,
            ..Default::default()
        },
        channels,
        ..Default::default()
    }
}

#[test]
fn an_end_to_end_bake_lays_out_bones_ranges_and_channels() {
    let skeleton = body_skeleton();
    let first = walk_clip(1.0);
    let second = walk_clip(2.0);
    let definition = baking_definition(
        vec![entry(None), entry(within(0.5, 1.5))],
        vec![probe(2.0), pair()],
    );

    let database = baked(&definition, &skeleton, &[&first, &second]);

    assert_eq!(database.bone_count().unwrap(), 3);
    assert_eq!(database.frame_count().unwrap(), 60);
    assert_eq!(database.rate, 30.0);
    assert_eq!(database.clips().count(), 2);
    assert_eq!(database.entry_of(29), Some(0));
    assert_eq!(database.entry_of(30), Some(1));
    assert_eq!(database.range_of(30), Some(30..60));
    assert_eq!(database.clip_time(30), Some((1, 0.5)));
    assert_eq!(database.feature_dims().unwrap(), 5);
    assert_eq!(database.features(31).unwrap()[1], 3.0);
    assert_eq!(database.features(31).unwrap()[2], 3.0);

    let simulation = database.simulation_bone(10).unwrap();
    assert!(simulation.translation.y.abs() < 1e-6);
    let facing = simulation.rotation * Vec3::Z;
    assert!((facing - Vec3::NEG_Z).length() < 1e-3);
}

fn turning_clip(duration: f32) -> AnimationClip {
    let mut clip = walk_clip(duration);
    clip.add_channel(
        Uuid::from_u128(2),
        AnimationChannel::new(
            vec![0.0, duration],
            AnimationChanelOutput::Rotation(vec![Quat::IDENTITY, Quat::from_rotation_y(1.0)]),
        ),
    );
    clip
}

#[test]
fn a_frames_pose_equals_the_pose_the_bake_gave_the_channels() {
    let skeleton = body_skeleton();
    let first = walk_clip(1.0);
    let second = turning_clip(2.0);
    let mut definition = baking_definition(
        vec![entry(None), entry(within(0.5, 1.5))],
        vec![pose_channel()],
    );
    definition.speed = 1.5;
    let clips = [&first, &second];

    let database = baked(&definition, &skeleton, &clips);

    assert!(database.frame_count().unwrap() >= 40);
    for frame in 0..database.frame_count().unwrap() {
        let pose = database.pose(frame, &skeleton, &clips).unwrap();
        let features = database.features(frame).unwrap();
        assert_eq!(pose.len(), 3);
        assert_eq!(pose[0].translation.to_array(), features[0..3]);
        assert_eq!(pose[0].rotation.to_array(), features[3..7]);
        assert_eq!(pose[1].translation.to_array(), features[7..10]);
    }
}

#[test]
fn a_pose_is_expressed_relative_to_the_simulation_bone() {
    let skeleton = body_skeleton();
    let clip = walk_clip(2.0);
    let definition = baking_definition(vec![entry(None)], Vec::new());
    let database = baked(&definition, &skeleton, &[&clip]);

    let pose = database.pose(20, &skeleton, &[&clip]).unwrap();

    let (_, time) = database.clip_time(20).unwrap();
    let model = model_pose(&skeleton, &sample_pose(&skeleton, &clip, time));
    let root = Transform::from_translation_rotation_scale(
        pose[0].translation,
        pose[0].rotation,
        pose[0].scale,
    );
    let composed = database.simulation_bone(20).unwrap() * root;
    assert!((composed.translation - model[0].translation).length() < 1e-5);
    assert!(composed.rotation.angle_between(model[0].rotation) < 1e-5);
    assert_eq!(pose[1], skeleton.rest_pose[1]);
}

#[test]
fn clip_time_follows_speed_and_rate() {
    let skeleton = body_skeleton();
    let first = walk_clip(1.0);
    let second = walk_clip(2.0);
    let mut definition = baking_definition(vec![entry(None), entry(within(0.5, 1.5))], Vec::new());
    definition.speed = 2.0;

    let database = baked(&definition, &skeleton, &[&first, &second]);

    assert_eq!(database.frame_count().unwrap(), 15 + 15);
    assert_eq!(database.clip_time(0), Some((0, 0.0)));
    assert_eq!(database.clip_time(3), Some((0, 0.2)));
    assert_eq!(database.clip_time(15), Some((1, 0.5)));
    let (entry, time) = database.clip_time(18).unwrap();
    assert_eq!(entry, 1);
    assert!((time - 0.7).abs() < 1e-5);
    assert_eq!(database.clip_time(30), None);
}

const SAMPLES: usize = 3;
const BODY_DIMS: usize = 6 + 7 + SAMPLES * 10;

#[derive(Reflect, Default, Clone, Debug, PartialEq)]
#[reflect(FeatureChannel)]
struct BodyChannel {
    unused: f32,
}

impl FeatureChannel for BodyChannel {
    fn dims(&self) -> usize {
        BODY_DIMS
    }

    fn extract(&self, frame: &FrameContext, out: &mut [f32]) {
        out[0..3].copy_from_slice(&frame.body.velocity.to_array());
        out[3..6].copy_from_slice(&frame.body.angular_velocity.to_array());
        out[6..9].copy_from_slice(&frame.simulation.translation.to_array());
        out[9..13].copy_from_slice(&frame.simulation.rotation.to_array());
        for (index, sample) in frame.trajectory.iter().enumerate() {
            let at = 13 + index * 10;
            out[at..at + 3].copy_from_slice(&sample.position.to_array());
            out[at + 3..at + 6].copy_from_slice(&sample.velocity.to_array());
            out[at + 6..at + 10].copy_from_slice(&sample.rotation.to_array());
        }
    }
}

const LOCAL_DIMS: usize = 6 + SAMPLES * 10;

#[derive(Reflect, Default, Clone, Debug, PartialEq)]
#[reflect(FeatureChannel)]
struct LocalChannel {
    unused: f32,
}

impl FeatureChannel for LocalChannel {
    fn dims(&self) -> usize {
        LOCAL_DIMS
    }

    fn extract(&self, frame: &FrameContext, out: &mut [f32]) {
        let inverse = frame.simulation.rotation.inverse();
        let origin = frame.simulation.translation;
        out[0..3].copy_from_slice(&(inverse * frame.body.velocity).to_array());
        out[3..6].copy_from_slice(&(inverse * frame.body.angular_velocity).to_array());
        for (index, sample) in frame.trajectory.iter().enumerate() {
            let at = 6 + index * 10;
            out[at..at + 3].copy_from_slice(&(inverse * (sample.position - origin)).to_array());
            out[at + 3..at + 6].copy_from_slice(&(inverse * sample.velocity).to_array());
            out[at + 6..at + 10].copy_from_slice(&(inverse * sample.rotation).to_array());
        }
    }
}

fn body_channel() -> FeatureChannelData {
    FeatureChannelData::live(BodyChannel { unused: 0.0 })
}

fn local_channel() -> FeatureChannelData {
    FeatureChannelData::live(LocalChannel { unused: 0.0 })
}

fn vec3(features: &[f32], at: usize) -> Vec3 {
    Vec3::from_slice(&features[at..at + 3])
}

fn quat(features: &[f32], at: usize) -> Quat {
    Quat::from_slice(&features[at..at + 4])
}

fn trajectory_definition(
    entries: Vec<MotionEntry>,
    speed: f32,
    channels: Vec<FeatureChannelData>,
) -> MotionDatabase {
    let mut definition = baking_definition(entries, channels);
    definition.speed = speed;
    definition.trajectory = TrajectorySettings {
        sample_dt: 0.2,
        sample_count: SAMPLES,
    };
    definition
}

#[test]
fn a_straight_walk_has_trajectory_samples_ahead_along_the_travel() {
    let skeleton = body_skeleton();
    let clip = walk_clip(3.0);
    for speed in [1.0, 2.0] {
        let definition = trajectory_definition(vec![entry(None)], speed, vec![body_channel()]);
        let database = baked(&definition, &skeleton, &[&clip]);

        let features = database.features(5).unwrap();
        assert!((vec3(features, 0) - Vec3::new(0.0, 0.0, -speed)).length() < 1e-3);
        assert!(vec3(features, 3).length() < 1e-3);
        let origin = vec3(features, 6);
        for k in 0..SAMPLES {
            let at = 13 + k * 10;
            let ahead = speed * 0.2 * (k + 1) as f32;
            let position = vec3(features, at) - origin;
            assert!(
                (position - Vec3::new(0.0, 0.0, -ahead)).length() < 1e-3,
                "{k}"
            );
            assert!((vec3(features, at + 3) - Vec3::new(0.0, 0.0, -speed)).length() < 1e-3);
            let facing = quat(features, at + 6) * Vec3::Z;
            assert!((facing - Vec3::NEG_Z).length() < 1e-3);
        }
    }
}

#[test]
fn a_turning_walk_has_the_turn_rate_as_angular_velocity() {
    let skeleton = body_skeleton();
    let clip = turning_clip(2.0);
    let definition = trajectory_definition(vec![entry(None)], 1.0, vec![body_channel()]);
    let database = baked(&definition, &skeleton, &[&clip]);

    for frame in [0, 7, 30, database.frame_count().unwrap() - 1] {
        let angular = vec3(database.features(frame).unwrap(), 3);
        assert!(angular.x.abs() < 1e-3 && angular.z.abs() < 1e-3, "{frame}");
        assert!((angular.y.abs() - 0.5).abs() < 1e-2, "{frame}: {angular:?}");
    }
}

fn two_range_database(channels: Vec<FeatureChannelData>) -> MotionDatabase {
    let skeleton = body_skeleton();
    let clip = walk_clip(3.0);
    let definition = trajectory_definition(
        vec![entry(within(0.0, 1.0)), entry(within(1.0, 3.0))],
        1.0,
        channels,
    );
    baked(&definition, &skeleton, &[&clip, &clip])
}

#[test]
fn trajectory_samples_sit_ahead_of_a_frame_and_stop_at_the_end_of_its_range() {
    let database = two_range_database(Vec::new());
    let positions = |frames: [usize; 3]| {
        frames.map(|frame| database.simulation_bone(frame).unwrap().translation)
    };
    let samples_at = |frame: usize| -> Vec<Vec3> {
        database
            .trajectory_samples(frame)
            .unwrap_or_default()
            .iter()
            .map(|sample| sample.position)
            .collect()
    };

    assert_eq!(samples_at(5), positions([11, 17, 23]));
    assert_eq!(samples_at(20), positions([26, 29, 29]));
    assert_eq!(samples_at(30), positions([36, 42, 48]));
    assert!(samples_at(90).is_empty());
}

#[test]
fn offline_features_equal_online_features_for_a_matching_simulation() {
    let skeleton = body_skeleton();
    let clip = turning_clip(3.0);
    let definition = trajectory_definition(
        vec![entry(None)],
        1.0,
        vec![body_channel(), local_channel()],
    );
    let database = baked(&definition, &skeleton, &[&clip]);
    let local_at = BODY_DIMS;

    let yaw = Quat::from_rotation_y(1.2);
    let offset = Vec3::new(4.0, 0.5, -7.0);
    let place = |point: Vec3| yaw * point + offset;

    for frame in [0, 20, 50] {
        let features = database.features(frame).unwrap();
        let trajectory: Vec<TrajectorySample> = (0..SAMPLES)
            .map(|k| {
                let at = 13 + k * 10;
                TrajectorySample {
                    position: place(vec3(features, at)),
                    velocity: yaw * vec3(features, at + 3),
                    rotation: yaw * quat(features, at + 6),
                }
            })
            .collect();
        let context = FrameContext {
            skeleton: &skeleton,
            local_pose: &[],
            model_pose: &[],
            model_velocities: &[],
            simulation: Transform::from_translation_rotation(
                place(vec3(features, 6)),
                yaw * quat(features, 9),
            ),
            body: BodyMotion {
                velocity: yaw * vec3(features, 0),
                angular_velocity: yaw * vec3(features, 3),
            },
            trajectory: &trajectory,
        };
        let mut online = [0.0; LOCAL_DIMS];
        LocalChannel::default().extract(&context, &mut online);

        let offline = &features[local_at..local_at + LOCAL_DIMS];
        assert!(offline.iter().any(|value| value.abs() > 0.1));
        for (index, (a, b)) in offline.iter().zip(&online).enumerate() {
            assert!(
                (a - b).abs() < 1e-5,
                "frame {frame} value {index}: {a} vs {b}"
            );
        }
    }
}

#[derive(Reflect, Clone, Debug, PartialEq)]
#[reflect(FeatureChannel)]
struct WeightedChannel {
    weight: f32,
    value: f32,
}

impl Default for WeightedChannel {
    fn default() -> Self {
        Self {
            weight: 1.0,
            value: 0.0,
        }
    }
}

impl FeatureChannel for WeightedChannel {
    fn dims(&self) -> usize {
        1
    }

    fn weight(&self) -> f32 {
        self.weight
    }

    fn source(&self) -> ChannelSource {
        ChannelSource::Trajectory
    }

    fn extract(&self, _frame: &FrameContext, out: &mut [f32]) {
        out[0] = self.value;
    }
}

fn weighted(weight: f32, value: f32) -> FeatureChannelData {
    FeatureChannelData::live(WeightedChannel { weight, value })
}

#[test]
fn a_weight_leaves_the_bake_alone() {
    let skeleton = body_skeleton();
    let clip = walk_clip(1.0);
    let weighed = |weight| {
        let definition = baking_definition(vec![entry(None)], vec![weighted(weight, 0.0)]);
        definition.bake(&skeleton, &[&clip]).unwrap()
    };

    assert_eq!(weighed(2.0), weighed(1.0));
}

fn legged_skeleton(right_leg: Vec3) -> Skeleton {
    Skeleton::new(
        vec![Mat4::IDENTITY; 5],
        vec![None, Some(0), Some(1), Some(1), Some(1)],
        vec![
            "Root".into(),
            "Hips".into(),
            "Spine2".into(),
            "LeftLeg".into(),
            "RightLeg".into(),
        ],
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
            JointPose {
                translation: Vec3::new(0.2, -0.1, 0.0),
                ..JointPose::IDENTITY
            },
            JointPose {
                translation: right_leg,
                ..JointPose::IDENTITY
            },
        ],
        (1..=5).map(Uuid::from_u128).collect(),
    )
    .unwrap()
}

fn symmetric_skeleton() -> Skeleton {
    legged_skeleton(Vec3::new(-0.2, -0.1, 0.0))
}

fn diagonal_turning_clip(duration: f32) -> AnimationClip {
    let mut clip = AnimationClip::default();
    clip.add_channel(
        Uuid::from_u128(1),
        AnimationChannel::new(
            vec![0.0, duration],
            AnimationChanelOutput::Translation(vec![
                Vec3::ZERO,
                Vec3::new(0.6 * duration, 0.0, -duration),
            ]),
        ),
    );
    clip.add_channel(
        Uuid::from_u128(2),
        AnimationChannel::new(
            vec![0.0, duration],
            AnimationChanelOutput::Rotation(vec![
                Quat::from_rotation_y(std::f32::consts::PI),
                Quat::from_rotation_y(std::f32::consts::PI + 1.0),
            ]),
        ),
    );
    clip.add_channel(
        Uuid::from_u128(4),
        AnimationChannel::new(
            vec![0.0, duration],
            AnimationChanelOutput::Rotation(vec![Quat::IDENTITY, Quat::from_rotation_x(0.7)]),
        ),
    );
    clip
}

fn mirrored(range: Option<TimeRange>) -> MotionEntry {
    MotionEntry {
        mirrored: true,
        ..entry(range)
    }
}

fn reflect_x(vector: Vec3) -> Vec3 {
    Vec3::new(-vector.x, vector.y, vector.z)
}

#[test]
fn a_mirrored_entry_bakes_the_mirrored_simulation_bone() {
    let skeleton = symmetric_skeleton();
    let clip = diagonal_turning_clip(2.0);
    let definition = baking_definition(vec![entry(None), mirrored(None)], Vec::new());

    let database = baked(&definition, &skeleton, &[&clip, &clip]);

    let frames = database.frames_of_entry(0).unwrap();
    assert_eq!(database.frames_of_entry(1), Some(frames));
    let mut travelled = 0.0f32;
    for frame in 0..frames {
        let original = database.simulation_bone(frame).unwrap();
        let mirror = database.simulation_bone(frames + frame).unwrap();
        travelled = travelled.max(original.translation.x.abs());
        assert!(
            (mirror.translation - reflect_x(original.translation)).length() < 1e-4,
            "frame {frame}: {} vs {}",
            mirror.translation,
            original.translation
        );
        let forward = mirror.rotation * Vec3::Z;
        let expected = reflect_x(original.rotation * Vec3::Z);
        assert!(
            (forward - expected).length() < 1e-4,
            "frame {frame}: {forward} vs {expected}"
        );
    }
    assert!(travelled > 0.5);
}

#[test]
fn a_mirrored_frames_pose_is_the_mirrored_sampled_pose_under_its_simulation_bone() {
    let skeleton = symmetric_skeleton();
    let clip = diagonal_turning_clip(2.0);
    let definition = baking_definition(vec![entry(None), mirrored(within(0.5, 1.5))], Vec::new());
    let clips = [&clip, &clip];
    let database = baked(&definition, &skeleton, &clips);
    let mirror = PoseMirror::new(&skeleton, Vec3::X).unwrap();

    for frame in database.ranges().unwrap().nth(1).unwrap() {
        let (entry, time) = database.clip_time(frame).unwrap();
        assert_eq!(entry, 1);
        let sampled = sample_pose(&skeleton, &clip, time);
        let mut expected = sampled.clone();
        mirror.mirror_into(&sampled, &mut expected).unwrap();
        let model = model_pose(&skeleton, &expected);
        reexpress_roots(
            &skeleton,
            &mut expected,
            &model,
            database.simulation_bone(frame).unwrap(),
        );

        assert_eq!(database.pose(frame, &skeleton, &clips).unwrap(), expected);
    }
}

struct Refused {
    name: &'static str,
    definition: MotionDatabase,
    refusal: fn(&BakeError) -> bool,
}

#[test]
fn a_definition_the_bake_cannot_honour_is_refused() {
    let rows = [
        Refused {
            name: "a range past the clip's end",
            definition: baking_definition(vec![entry(within(0.5, 1.5))], Vec::new()),
            refusal: |error| matches!(error, BakeError::InvalidRange { entry: 0 }),
        },
        Refused {
            name: "a range under two frames",
            definition: baking_definition(vec![entry(within(0.0, 0.02))], Vec::new()),
            refusal: |error| matches!(error, BakeError::RangeTooShort { entry: 0, .. }),
        },
        Refused {
            name: "a simulation joint the skeleton lacks",
            definition: MotionDatabase {
                simulation: SimulationBoneSettings {
                    position_joint: "Nope".into(),
                    ..baking_definition(Vec::new(), Vec::new()).simulation
                },
                ..baking_definition(vec![entry(None)], Vec::new())
            },
            refusal: |error| matches!(error, BakeError::MissingJoint(_)),
        },
        Refused {
            name: "a mirrored entry on an asymmetric skeleton",
            definition: baking_definition(vec![entry(None), mirrored(None)], Vec::new()),
            refusal: |error| matches!(error, BakeError::Mirror(MirrorError::Asymmetric { .. })),
        },
    ];
    let skeleton = legged_skeleton(Vec3::new(-0.2, -0.1, 0.3));
    let clip = diagonal_turning_clip(1.0);

    for row in rows {
        let clips = vec![&clip; row.definition.entries.len()];

        let error = row.definition.bake(&skeleton, &clips).err();

        assert!(
            error.as_ref().is_some_and(row.refusal),
            "{}: {error:?}",
            row.name
        );
    }
}
