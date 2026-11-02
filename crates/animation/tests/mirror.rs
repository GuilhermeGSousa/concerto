use concerto_animation::mirror::{MirrorError, PoseMirror};
use concerto_mesh::skeleton::{JointPose, Skeleton};
use glam::{Mat3, Mat4, Quat, Vec3};
use uuid::Uuid;

fn joint(translation: Vec3, rotation: Quat) -> JointPose {
    JointPose {
        translation,
        rotation,
        scale: Vec3::ONE,
    }
}

fn skeleton(bones: &[(&str, Option<u16>, JointPose)]) -> Skeleton {
    Skeleton::new(
        vec![Mat4::IDENTITY; bones.len()],
        bones.iter().map(|(_, parent, _)| *parent).collect(),
        bones.iter().map(|(name, _, _)| name.to_string()).collect(),
        bones.iter().map(|(_, _, pose)| pose.clone()).collect(),
        bones.iter().map(|_| Uuid::new_v4()).collect(),
    )
    .unwrap()
}

fn reflection(axis: Vec3) -> Mat3 {
    let n = axis.normalize();
    Mat3::IDENTITY - 2.0 * Mat3::from_cols(n * n.x, n * n.y, n * n.z)
}

fn mirrored_rotation(rotation: Quat, axis: Vec3) -> Quat {
    let m = reflection(axis);
    Quat::from_mat3(&(m * Mat3::from_quat(rotation) * m)).normalize()
}

fn symmetric_skeleton(leg_rest: Quat) -> Skeleton {
    let foot = Vec3::new(0.0, -0.4, 0.0);
    skeleton(&[
        (
            "Hips",
            None,
            joint(Vec3::new(0.0, 1.0, 0.0), Quat::IDENTITY),
        ),
        (
            "LeftLeg",
            Some(0),
            joint(Vec3::new(0.1, -0.1, 0.0), leg_rest),
        ),
        (
            "RightLeg",
            Some(0),
            joint(
                Vec3::new(-0.1, -0.1, 0.0),
                mirrored_rotation(leg_rest, Vec3::X),
            ),
        ),
        (
            "Spine",
            Some(0),
            joint(Vec3::new(0.0, 0.2, 0.0), Quat::IDENTITY),
        ),
        ("LeftFoot", Some(1), joint(foot, Quat::IDENTITY)),
        ("RightFoot", Some(2), joint(foot, Quat::IDENTITY)),
    ])
}

fn rotated_rest() -> Quat {
    Quat::from_euler(glam::EulerRot::XYZ, 0.3, 0.2, 0.4)
}

fn posed(skeleton: &Skeleton) -> Vec<JointPose> {
    let mut pose = skeleton.rest_pose.to_vec();
    pose[0] = joint(
        Vec3::new(0.3, 0.9, 0.2),
        Quat::from_euler(glam::EulerRot::YXZ, 0.7, 0.1, -0.2),
    );
    pose[1].rotation = Quat::from_euler(glam::EulerRot::XYZ, 0.8, -0.3, 0.5);
    pose[2].rotation = Quat::from_euler(glam::EulerRot::XYZ, -0.2, 0.6, 0.1);
    pose[3].rotation = Quat::from_euler(glam::EulerRot::XYZ, 0.1, 0.4, -0.3);
    pose[4].rotation = Quat::from_euler(glam::EulerRot::XYZ, 0.5, 0.0, 0.2);
    pose[5].rotation = Quat::from_euler(glam::EulerRot::XYZ, -0.4, 0.3, 0.0);
    pose
}

fn mirror(mirror: &PoseMirror, source: &[JointPose]) -> Vec<JointPose> {
    let mut out = vec![JointPose::IDENTITY; source.len()];
    mirror.mirror_into(source, &mut out).unwrap();
    out
}

fn same_rotation(a: Quat, b: Quat) -> bool {
    a.dot(b).abs() > 1.0 - 1e-5
}

#[test]
fn left_and_right_joints_are_twins_and_the_rest_are_their_own() {
    let mirror = PoseMirror::new(&symmetric_skeleton(Quat::IDENTITY), Vec3::X).unwrap();

    assert_eq!(mirror.twin(0), Some(0));
    assert_eq!(mirror.twin(1), Some(2));
    assert_eq!(mirror.twin(2), Some(1));
    assert_eq!(mirror.twin(3), Some(3));
    assert_eq!(mirror.twin(4), Some(5));
    assert_eq!(mirror.twin(5), Some(4));
    assert_eq!(mirror.twin(6), None);
}

#[test]
fn mirroring_the_rest_pose_returns_the_rest_pose() {
    let skeleton = symmetric_skeleton(rotated_rest());
    let mirror_x = PoseMirror::new(&skeleton, Vec3::X).unwrap();

    let out = mirror(&mirror_x, &skeleton.rest_pose);

    for (result, rest) in out.iter().zip(skeleton.rest_pose.iter()) {
        assert!(result.translation.abs_diff_eq(rest.translation, 1e-5));
        assert!(same_rotation(result.rotation, rest.rotation));
        assert_eq!(result.scale, rest.scale);
    }
}

#[test]
fn mirroring_twice_returns_the_original_pose() {
    let skeleton = symmetric_skeleton(rotated_rest());
    let mirror_x = PoseMirror::new(&skeleton, Vec3::X).unwrap();
    let pose = posed(&skeleton);

    let twice = mirror(&mirror_x, &mirror(&mirror_x, &pose));

    for (result, original) in twice.iter().zip(pose.iter()) {
        assert!(result.translation.abs_diff_eq(original.translation, 1e-5));
        assert!(same_rotation(result.rotation, original.rotation));
    }
}

#[test]
fn each_joint_lands_at_the_reflection_of_its_twin() {
    let skeleton = symmetric_skeleton(rotated_rest());
    let mirror_x = PoseMirror::new(&skeleton, Vec3::X).unwrap();
    let m = reflection(Vec3::X);
    let pose = posed(&skeleton);

    let original = skeleton.forward_kinematics(&pose).unwrap();
    let mirrored = skeleton
        .forward_kinematics(&mirror(&mirror_x, &pose))
        .unwrap();

    let right_foot = mirrored[5].w_axis.truncate();
    let left_foot = original[4].w_axis.truncate();
    assert!(right_foot.abs_diff_eq(m * left_foot, 1e-5));
    for (bone, model) in mirrored.iter().enumerate() {
        let twin = mirror_x.twin(bone).unwrap();
        let position = model.w_axis.truncate();
        assert!(position.abs_diff_eq(m * original[twin].w_axis.truncate(), 1e-5));
    }
}

#[test]
fn with_identity_rest_rotations_the_model_rotation_is_conjugated_by_the_reflection() {
    let skeleton = symmetric_skeleton(Quat::IDENTITY);
    let mirror_x = PoseMirror::new(&skeleton, Vec3::X).unwrap();
    let m = reflection(Vec3::X);
    let pose = posed(&skeleton);

    let original = skeleton.forward_kinematics(&pose).unwrap();
    let mirrored = skeleton
        .forward_kinematics(&mirror(&mirror_x, &pose))
        .unwrap();

    for (bone, model) in mirrored.iter().enumerate() {
        let twin = mirror_x.twin(bone).unwrap();
        let expected = m * Mat3::from_mat4(original[twin]) * m;
        assert!(Mat3::from_mat4(*model).abs_diff_eq(expected, 1e-5));
    }
}

#[test]
fn a_skeleton_symmetric_across_z_mirrors_across_z_and_not_x() {
    let skeleton = skeleton(&[
        (
            "Hips",
            None,
            joint(Vec3::new(0.0, 1.0, 0.0), Quat::IDENTITY),
        ),
        (
            "LeftLeg",
            Some(0),
            joint(Vec3::new(0.0, -0.1, 0.1), Quat::IDENTITY),
        ),
        (
            "RightLeg",
            Some(0),
            joint(Vec3::new(0.0, -0.1, -0.1), Quat::IDENTITY),
        ),
    ]);

    assert!(matches!(
        PoseMirror::new(&skeleton, Vec3::X),
        Err(MirrorError::Asymmetric { .. })
    ));
    assert!(PoseMirror::new(&skeleton, Vec3::Z).is_ok());
}

#[test]
fn a_left_joint_without_a_right_twin_is_missing_its_twin() {
    let skeleton = skeleton(&[
        ("Hips", None, JointPose::IDENTITY),
        ("LeftHand", Some(0), JointPose::IDENTITY),
    ]);

    assert_eq!(
        PoseMirror::new(&skeleton, Vec3::X),
        Err(MirrorError::MissingTwin {
            joint: "LeftHand".to_string(),
            twin: "RightHand".to_string(),
        })
    );
}

#[test]
fn twins_under_parents_that_are_not_twins_are_rejected() {
    let skeleton = skeleton(&[
        ("Hips", None, JointPose::IDENTITY),
        ("LeftArm", Some(0), JointPose::IDENTITY),
        ("RightArm", Some(0), JointPose::IDENTITY),
        ("LeftHand", Some(1), JointPose::IDENTITY),
        ("RightHand", Some(1), JointPose::IDENTITY),
    ]);

    assert_eq!(
        PoseMirror::new(&skeleton, Vec3::X),
        Err(MirrorError::TwinParents {
            joint: "LeftHand".to_string(),
        })
    );
}

#[test]
fn a_zero_axis_is_invalid() {
    assert_eq!(
        PoseMirror::new(&symmetric_skeleton(Quat::IDENTITY), Vec3::ZERO),
        Err(MirrorError::InvalidAxis(Vec3::ZERO))
    );
}

#[test]
fn a_short_pose_does_not_fit_the_mirror() {
    let skeleton = symmetric_skeleton(Quat::IDENTITY);
    let mirror_x = PoseMirror::new(&skeleton, Vec3::X).unwrap();
    let mut out = vec![JointPose::IDENTITY; skeleton.bone_count()];

    assert_eq!(
        mirror_x.mirror_into(&skeleton.rest_pose[..2], &mut out),
        Err(MirrorError::BoneCountMismatch {
            expected: 6,
            actual: 2,
        })
    );
}

fn yawed_skeleton() -> Skeleton {
    let m = reflection(Vec3::X);
    let spine = Quat::from_rotation_y(0.5);
    let left_arm = rotated_rest();
    let right_arm = (mirrored_rotation(left_arm, Vec3::X)
        * Quat::from_rotation_x(std::f32::consts::PI))
    .normalize();
    let spine_model = Mat3::from_quat(spine);
    let right_arm_offset = Vec3::new(-0.2, 0.1, 0.05);
    let left_arm_offset = spine_model.transpose() * m * spine_model * right_arm_offset;
    let left_model = spine_model * Mat3::from_quat(left_arm);
    let right_model = spine_model * Mat3::from_quat(right_arm);
    let right_hand_offset = Vec3::new(0.05, -0.3, 0.1);
    let left_hand_offset = left_model.transpose() * m * right_model * right_hand_offset;
    skeleton(&[
        (
            "Hips",
            None,
            joint(Vec3::new(0.0, 1.0, 0.0), Quat::IDENTITY),
        ),
        ("Spine", Some(0), joint(Vec3::new(0.0, 0.2, 0.0), spine)),
        (
            "Neck",
            Some(1),
            joint(Vec3::new(0.0, 0.3, 0.0), Quat::IDENTITY),
        ),
        ("LeftArm", Some(1), joint(left_arm_offset, left_arm)),
        ("RightArm", Some(1), joint(right_arm_offset, right_arm)),
        ("LeftHand", Some(3), joint(left_hand_offset, Quat::IDENTITY)),
        (
            "RightHand",
            Some(4),
            joint(right_hand_offset, Quat::IDENTITY),
        ),
    ])
}

#[test]
fn a_yawed_spine_and_twins_whose_rests_differ_by_a_rotation_keep_their_rest_pose() {
    let skeleton = yawed_skeleton();
    let mirror_x = PoseMirror::new(&skeleton, Vec3::X).unwrap();

    let out = mirror(&mirror_x, &skeleton.rest_pose);

    for (result, rest) in out.iter().zip(skeleton.rest_pose.iter()) {
        assert!(result.translation.abs_diff_eq(rest.translation, 1e-5));
        assert!(same_rotation(result.rotation, rest.rotation));
    }
}

#[test]
fn a_yawed_spine_and_twins_whose_rests_differ_by_a_rotation_land_at_their_twins_reflection() {
    let skeleton = yawed_skeleton();
    let mirror_x = PoseMirror::new(&skeleton, Vec3::X).unwrap();
    let m = reflection(Vec3::X);
    let mut pose = skeleton.rest_pose.to_vec();
    pose[0].rotation = Quat::from_euler(glam::EulerRot::YXZ, 0.7, 0.1, -0.2);
    pose[1].rotation = Quat::from_euler(glam::EulerRot::XYZ, 0.2, 0.9, -0.1);
    pose[3].rotation = Quat::from_euler(glam::EulerRot::XYZ, 0.8, -0.3, 0.5);
    pose[4].rotation = Quat::from_euler(glam::EulerRot::XYZ, -0.2, 0.6, 0.1);

    let original = skeleton.forward_kinematics(&pose).unwrap();
    let mirrored = skeleton
        .forward_kinematics(&mirror(&mirror_x, &pose))
        .unwrap();

    for (bone, model) in mirrored.iter().enumerate() {
        let twin = mirror_x.twin(bone).unwrap();
        let position = model.w_axis.truncate();
        assert!(
            position.abs_diff_eq(m * original[twin].w_axis.truncate(), 1e-5),
            "{}",
            skeleton.names[bone]
        );
    }
}
