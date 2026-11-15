use concerto_animation::mirror::{MirrorError, PoseMirror};
use concerto_mesh::skeleton::{JointPose, Skeleton};
use glam::{Mat3, Mat4, Quat, Vec3};
use proptest::prelude::*;
use uuid::Uuid;

fn joint(translation: Vec3, rotation: Quat) -> JointPose {
    JointPose {
        translation,
        rotation,
        scale: Vec3::ONE,
    }
}

struct Bone {
    name: &'static str,
    parent: Option<u16>,
    pose: JointPose,
}

fn bone(name: &'static str, parent: Option<u16>, pose: JointPose) -> Bone {
    Bone { name, parent, pose }
}

fn skeleton(bones: &[Bone]) -> Skeleton {
    Skeleton::new(
        vec![Mat4::IDENTITY; bones.len()],
        bones.iter().map(|bone| bone.parent).collect(),
        bones.iter().map(|bone| bone.name.into()).collect(),
        bones.iter().map(|bone| bone.pose.clone()).collect(),
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
        bone(
            "Hips",
            None,
            joint(Vec3::new(0.0, 1.0, 0.0), Quat::IDENTITY),
        ),
        bone(
            "LeftLeg",
            Some(0),
            joint(Vec3::new(0.1, -0.1, 0.0), leg_rest),
        ),
        bone(
            "RightLeg",
            Some(0),
            joint(
                Vec3::new(-0.1, -0.1, 0.0),
                mirrored_rotation(leg_rest, Vec3::X),
            ),
        ),
        bone(
            "Spine",
            Some(0),
            joint(Vec3::new(0.0, 0.2, 0.0), Quat::IDENTITY),
        ),
        bone("LeftFoot", Some(1), joint(foot, Quat::IDENTITY)),
        bone("RightFoot", Some(2), joint(foot, Quat::IDENTITY)),
    ])
}

fn rotated_rest() -> Quat {
    Quat::from_euler(glam::EulerRot::XYZ, 0.3, 0.2, 0.4)
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
        bone(
            "Hips",
            None,
            joint(Vec3::new(0.0, 1.0, 0.0), Quat::IDENTITY),
        ),
        bone("Spine", Some(0), joint(Vec3::new(0.0, 0.2, 0.0), spine)),
        bone(
            "Neck",
            Some(1),
            joint(Vec3::new(0.0, 0.3, 0.0), Quat::IDENTITY),
        ),
        bone("LeftArm", Some(1), joint(left_arm_offset, left_arm)),
        bone("RightArm", Some(1), joint(right_arm_offset, right_arm)),
        bone("LeftHand", Some(3), joint(left_hand_offset, Quat::IDENTITY)),
        bone(
            "RightHand",
            Some(4),
            joint(right_hand_offset, Quat::IDENTITY),
        ),
    ])
}

fn mirror(mirror: &PoseMirror, source: &[JointPose]) -> Vec<JointPose> {
    let mut out = vec![JointPose::IDENTITY; source.len()];
    mirror.mirror_into(source, &mut out).unwrap();
    out
}

fn same_rotation(a: Quat, b: Quat) -> bool {
    a.dot(b).abs() > 1.0 - 1e-5
}

fn any_turn() -> impl Strategy<Value = Quat> {
    prop::array::uniform3(-1.5f32..1.5)
        .prop_map(|[x, y, z]| Quat::from_euler(glam::EulerRot::XYZ, x, y, z))
}

proptest! {
    #[test]
    fn mirroring_keeps_the_rest_pose_puts_each_joint_at_its_twins_reflection_and_undoes_itself(
        which in 0usize..3,
        turns in prop::collection::vec(any_turn(), 7),
        hips_at in prop::array::uniform3(-1.0f32..1.0).prop_map(Vec3::from_array),
    ) {
        let identity_rests = which == 0;
        let skeleton = match which {
            0 => symmetric_skeleton(Quat::IDENTITY),
            1 => symmetric_skeleton(rotated_rest()),
            _ => yawed_skeleton(),
        };
        let mirror_x = PoseMirror::new(&skeleton, Vec3::X).unwrap();
        let m = reflection(Vec3::X);
        let mut pose = skeleton.rest_pose.to_vec();
        pose[0].translation = hips_at;
        for (joint, turn) in pose.iter_mut().zip(&turns) {
            joint.rotation = *turn * joint.rotation;
        }

        let rest = mirror(&mirror_x, &skeleton.rest_pose);
        let mirrored = mirror(&mirror_x, &pose);
        let twice = mirror(&mirror_x, &mirrored);

        for (result, rest) in rest.iter().zip(skeleton.rest_pose.iter()) {
            prop_assert!(result.translation.abs_diff_eq(rest.translation, 1e-5));
            prop_assert!(same_rotation(result.rotation, rest.rotation));
            prop_assert_eq!(result.scale, rest.scale);
        }
        for (result, original) in twice.iter().zip(pose.iter()) {
            prop_assert!(result.translation.abs_diff_eq(original.translation, 1e-4));
            prop_assert!(same_rotation(result.rotation, original.rotation));
        }
        let original = skeleton.forward_kinematics(&pose).unwrap();
        let reflected = skeleton.forward_kinematics(&mirrored).unwrap();
        for (bone, model) in reflected.iter().enumerate() {
            let twin = mirror_x.twin(bone).unwrap();
            prop_assert!(
                model.w_axis.truncate().abs_diff_eq(m * original[twin].w_axis.truncate(), 1e-4),
                "{}",
                skeleton.names[bone]
            );
            if identity_rests {
                let expected = m * Mat3::from_mat4(original[twin]) * m;
                prop_assert!(Mat3::from_mat4(*model).abs_diff_eq(expected, 1e-4));
            }
        }
    }
}

#[test]
fn a_skeleton_that_is_not_mirror_symmetric_about_the_axis_is_refused() {
    let across_z = skeleton(&[
        bone(
            "Hips",
            None,
            joint(Vec3::new(0.0, 1.0, 0.0), Quat::IDENTITY),
        ),
        bone(
            "LeftLeg",
            Some(0),
            joint(Vec3::new(0.0, -0.1, 0.1), Quat::IDENTITY),
        ),
        bone(
            "RightLeg",
            Some(0),
            joint(Vec3::new(0.0, -0.1, -0.1), Quat::IDENTITY),
        ),
    ]);

    assert!(PoseMirror::new(&across_z, Vec3::Z).is_ok());
    assert!(matches!(
        PoseMirror::new(&across_z, Vec3::X),
        Err(MirrorError::Asymmetric { .. })
    ));
}
