use std::f32::consts::FRAC_PI_2;

use concerto_foundation::transform::Transform;
use concerto_mesh::{JointPose, Skeleton};
use concerto_motion_matching::{
    BodyMotion, BonePositions, BoneVelocities, FeatureChannel, FeatureShape, FrameContext,
    ReflectFeatureChannel, TrajectoryDirections, TrajectoryPositions, TrajectorySample,
};
use concerto_reflect::Reflect;
use glam::{Mat4, Quat, Vec3};
use uuid::Uuid;

const NEAR: f32 = 1e-5;

fn skeleton() -> Skeleton {
    Skeleton::new(
        vec![Mat4::IDENTITY; 2],
        vec![None, Some(0)],
        vec!["Hips".into(), "LeftFoot".into()],
        vec![JointPose::IDENTITY; 2],
        vec![Uuid::from_u128(1), Uuid::from_u128(2)],
    )
    .unwrap()
}

fn simulation() -> Transform {
    Transform::from_translation_rotation(Vec3::new(1.0, 0.0, 2.0), Quat::from_rotation_y(FRAC_PI_2))
}

fn sample_at(position: Vec3) -> TrajectorySample {
    TrajectorySample {
        position,
        ..TrajectorySample::default()
    }
}

fn shapes_of(
    channel: &dyn FeatureChannel,
    model_pose: &[Transform],
    trajectory: &[TrajectorySample],
    features: &[f32],
) -> Vec<FeatureShape> {
    let skeleton = skeleton();
    let frame = FrameContext {
        skeleton: &skeleton,
        local_pose: &[],
        model_pose,
        model_velocities: &[],
        simulation: simulation(),
        body: BodyMotion::default(),
        trajectory,
    };
    let mut shapes = Vec::new();
    channel.preview(&frame, features, &mut shapes);
    shapes
}

fn feet() -> Vec<Transform> {
    vec![
        Transform::from_translation(Vec3::new(1.0, 1.0, 2.0)),
        Transform::from_translation(Vec3::new(2.0, 0.5, 2.0)),
    ]
}

fn assert_near(actual: Vec3, expected: Vec3) {
    assert!(
        actual.distance(expected) < NEAR,
        "{actual} is not {expected}"
    );
}

#[derive(Reflect, Default, Clone)]
#[reflect(FeatureChannel)]
struct SilentChannel;

impl FeatureChannel for SilentChannel {
    fn dims(&self) -> usize {
        1
    }

    fn extract(&self, _frame: &FrameContext, out: &mut [f32]) {
        out[0] = 1.0;
    }
}

#[test]
fn bone_velocities_leave_the_bone_in_the_context_along_the_scaled_slice() {
    let channel = BoneVelocities {
        bones: vec!["LeftFoot".into()],
        weight: 1.0,
    };

    let shapes = shapes_of(&channel, &feet(), &[], &[2.0, 0.0, -1.0]);

    let [FeatureShape::Arrow { start, end }] = shapes[..] else {
        panic!("{shapes:?}");
    };
    assert_near(start, Vec3::new(0.0, 0.5, 1.0));
    assert_near(end, Vec3::new(0.2, 0.5, 0.9));
}

#[test]
fn trajectory_directions_leave_the_sample_in_the_context_at_a_set_length() {
    let channel = TrajectoryDirections {
        samples: vec![1],
        weight: 1.0,
    };
    let trajectory = [sample_at(Vec3::ZERO), sample_at(Vec3::new(1.0, 0.0, 5.0))];

    let shapes = shapes_of(&channel, &[], &trajectory, &[1.2, 1.6]);

    let [FeatureShape::Arrow { start, end }] = shapes[..] else {
        panic!("{shapes:?}");
    };
    assert_near(start, Vec3::new(-3.0, 0.0, 0.0));
    assert_near(end, Vec3::new(-2.82, 0.0, 0.24));
}

#[test]
fn preview_features_gives_each_channel_its_own_span_and_list() {
    let channels: Vec<Box<dyn FeatureChannel>> = vec![
        Box::new(BonePositions {
            bones: vec!["LeftFoot".into()],
            weight: 1.0,
        }),
        Box::new(SilentChannel),
        Box::new(TrajectoryPositions {
            samples: vec![0],
            weight: 1.0,
        }),
    ];
    let skeleton = skeleton();
    let frame = FrameContext {
        skeleton: &skeleton,
        local_pose: &[],
        model_pose: &[],
        model_velocities: &[],
        simulation: simulation(),
        body: BodyMotion::default(),
        trajectory: &[],
    };

    let shapes = frame.preview_features(&channels, &[1.0, 2.0, 3.0, 9.0, 4.0, 5.0]);

    let end = Vec3::new(4.0, 0.0, 5.0);
    assert_eq!(
        shapes,
        vec![
            vec![FeatureShape::Ring {
                center: Vec3::new(1.0, 2.0, 3.0),
                radius: 0.05,
            }],
            Vec::new(),
            vec![
                FeatureShape::Line {
                    start: Vec3::ZERO,
                    end,
                },
                FeatureShape::Ring {
                    center: end,
                    radius: 0.12,
                },
            ],
        ]
    );
}
