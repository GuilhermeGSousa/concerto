use std::any::TypeId;

use concerto_mesh::skeleton::{JointName, JointPose, Skeleton, SkeletonError};
use concerto_reflect::{
    apply_value, json_to_value, to_json, to_value, ReflectDefault, ReflectJson, ReflectedValue,
    TypeRegistry,
};
use glam::{Mat4, Quat, Vec3};
use serde_json::json;
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

#[test]
fn skeleton_round_trips_through_bincode() {
    let skeleton = chain();
    let bytes = bincode::serialize(&skeleton).expect("Skeleton must serialize");
    let decoded: Skeleton = bincode::deserialize(&bytes).expect("Skeleton must round-trip");

    assert_eq!(decoded.parents, skeleton.parents);
    assert_eq!(decoded.names, skeleton.names);
    assert_eq!(decoded.rest_pose, skeleton.rest_pose);
    assert_eq!(decoded.bone_ids, skeleton.bone_ids);
    assert_eq!(decoded.inverse_bindposes, skeleton.inverse_bindposes);
}

#[test]
fn a_joint_name_derefs_to_its_text() {
    let name = JointName::from("Hips");

    assert_eq!(&*name, "Hips");
    assert_eq!(name.len(), 4);
    assert_eq!(chain().bone_index(&JointName::from("spine")), Some(1));
}

#[test]
fn a_joint_name_reflects_as_a_leaf_holding_the_bytes_of_its_text() {
    let stored = to_value(&JointName::from("Hips")).unwrap();

    assert!(matches!(stored, ReflectedValue::Leaf(_)));
    assert_eq!(stored, to_value(&"Hips".to_string()).unwrap());

    let mut read = JointName::default();
    assert!(apply_value(&mut read, &stored).is_empty());
    assert_eq!(read, JointName::from("Hips"));
}

#[test]
fn a_joint_name_writes_and_reads_json_as_a_string() {
    let mut registry = TypeRegistry::default();
    registry.register::<JointName>();
    let id = TypeId::of::<JointName>();

    assert_eq!(to_json(&JointName::from("Hips"), &registry), json!("Hips"));
    assert_eq!(
        json_to_value(&json!("Spine"), id, &registry).unwrap(),
        to_value(&JointName::from("Spine")).unwrap()
    );
    assert_eq!(
        json_to_value(&json!(3), id, &registry).unwrap_err(),
        "expected a string"
    );
}

#[test]
fn a_joint_name_registers_its_default_and_json_form() {
    let mut registry = TypeRegistry::default();
    registry.register::<JointName>();
    let registration = registry.get_by_name("JointName").unwrap();

    assert!(registration.data::<ReflectDefault>().is_some());
    assert!(registration.data::<ReflectJson>().is_some());
}

#[test]
fn a_joint_name_round_trips_through_bincode_as_its_text() {
    let bytes = bincode::serialize(&JointName::from("Hips")).unwrap();

    assert_eq!(bytes, bincode::serialize("Hips").unwrap());
    assert_eq!(
        bincode::deserialize::<JointName>(&bytes).unwrap(),
        JointName::from("Hips")
    );
}
