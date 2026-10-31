use concerto_mesh::skeleton::{JointPose, Skeleton, SkeletonError};
use glam::{Mat4, Quat, Vec3};
use uuid::Uuid;

fn translated(x: f32, y: f32, z: f32) -> JointPose {
    JointPose {
        translation: Vec3::new(x, y, z),
        ..JointPose::IDENTITY
    }
}

fn chain() -> Skeleton {
    Skeleton::new(
        vec![Mat4::IDENTITY; 3],
        vec![None, Some(0), Some(1)],
        vec!["hips".to_string(), "spine".to_string(), "head".to_string()],
        vec![
            translated(0.0, 1.0, 0.0),
            translated(0.0, 1.0, 0.0),
            translated(0.0, 1.0, 0.0),
        ],
        vec![Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3)],
    )
    .expect("a parents-first chain must build")
}

#[test]
fn new_rejects_unequal_lengths() {
    let result = Skeleton::new(
        vec![Mat4::IDENTITY; 2],
        vec![None],
        vec!["a".to_string(), "b".to_string()],
        vec![JointPose::IDENTITY; 2],
        vec![Uuid::nil(); 2],
    );
    assert_eq!(
        result.err(),
        Some(SkeletonError::LengthMismatch {
            field: "parents",
            expected: 2,
            actual: 1
        })
    );
}

#[test]
fn new_rejects_a_parent_that_does_not_precede_its_child() {
    let result = Skeleton::new(
        vec![Mat4::IDENTITY; 2],
        vec![Some(1), None],
        vec!["a".to_string(), "b".to_string()],
        vec![JointPose::IDENTITY; 2],
        vec![Uuid::nil(); 2],
    );
    assert_eq!(
        result.err(),
        Some(SkeletonError::ParentNotBefore { bone: 0, parent: 1 })
    );

    let own_parent = Skeleton::new(
        vec![Mat4::IDENTITY],
        vec![Some(0)],
        vec!["a".to_string()],
        vec![JointPose::IDENTITY],
        vec![Uuid::nil()],
    );
    assert!(own_parent.is_err(), "a bone cannot be its own parent");
}

#[test]
fn bone_index_finds_the_first_bone_by_name() {
    let skeleton = chain();
    assert_eq!(skeleton.bone_count(), 3);
    assert_eq!(skeleton.bone_index("spine"), Some(1));
    assert_eq!(skeleton.bone_index("tail"), None);
}

#[test]
fn forward_kinematics_composes_parent_transforms() {
    let skeleton = chain();
    let global = skeleton
        .forward_kinematics(&skeleton.rest_pose)
        .expect("a pose of the right length must evaluate");

    assert_eq!(global.len(), 3);
    assert!(global[0].abs_diff_eq(Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0)), 1e-6));
    assert!(global[2].abs_diff_eq(Mat4::from_translation(Vec3::new(0.0, 3.0, 0.0)), 1e-6));
}

#[test]
fn forward_kinematics_applies_parent_rotation_to_children() {
    let skeleton = chain();
    let mut local = skeleton.rest_pose.to_vec();
    local[0].rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
    let global = skeleton.forward_kinematics(&local).unwrap();

    let head = global[2].transform_point3(Vec3::ZERO);
    assert!(
        head.abs_diff_eq(Vec3::new(-2.0, 1.0, 0.0), 1e-5),
        "got {head}"
    );
}

#[test]
fn forward_kinematics_rejects_a_pose_of_the_wrong_length() {
    let skeleton = chain();
    assert_eq!(
        skeleton.forward_kinematics(&[JointPose::IDENTITY]),
        Err(SkeletonError::PoseLength {
            expected: 3,
            actual: 1
        })
    );
}

#[test]
fn joint_pose_to_mat4_scales_then_rotates_then_translates() {
    let pose = JointPose {
        translation: Vec3::new(1.0, 0.0, 0.0),
        rotation: Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
        scale: Vec3::splat(2.0),
    };
    let point = pose.to_mat4().transform_point3(Vec3::X);
    assert!(
        point.abs_diff_eq(Vec3::new(1.0, 2.0, 0.0), 1e-5),
        "got {point}"
    );
}
