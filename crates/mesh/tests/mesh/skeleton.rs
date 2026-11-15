use std::any::TypeId;

use concerto_mesh::skeleton::{JointName, JointPose, Skeleton};
use concerto_name::name;
use concerto_reflect::{
    apply_value, json_to_value, to_json, to_value, ReflectedValue, TypeRegistry,
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
        vec!["hips".into(), "spine".into(), "head".into()],
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
fn forward_kinematics_places_each_joint_under_its_parent() {
    let skeleton = chain();
    assert_eq!(skeleton.bone_index(name!("spine")), Some(1));

    let global = skeleton.forward_kinematics(&skeleton.rest_pose).unwrap();
    assert!(global[0].abs_diff_eq(Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0)), 1e-6));
    assert!(global[2].abs_diff_eq(Mat4::from_translation(Vec3::new(0.0, 3.0, 0.0)), 1e-6));

    let mut local = skeleton.rest_pose.to_vec();
    local[0].rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
    let head = skeleton.forward_kinematics(&local).unwrap()[2].transform_point3(Vec3::ZERO);
    assert!(
        head.abs_diff_eq(Vec3::new(-2.0, 1.0, 0.0), 1e-5),
        "got {head}"
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
fn a_joint_name_encodes_as_its_text_in_reflection_json_and_bincode() {
    let stored = to_value(&JointName::from("Hips")).unwrap();
    assert!(matches!(stored, ReflectedValue::Leaf(_)));
    assert_eq!(stored, to_value(&"Hips".to_string()).unwrap());
    let mut read = JointName::default();
    assert!(apply_value(&mut read, &stored).is_empty());
    assert_eq!(read, JointName::from("Hips"));

    let mut registry = TypeRegistry::default();
    registry.register::<JointName>();
    let id = TypeId::of::<JointName>();
    assert_eq!(to_json(&JointName::from("Hips"), &registry), json!("Hips"));
    assert_eq!(
        json_to_value(&json!("Spine"), id, &registry).unwrap(),
        to_value(&JointName::from("Spine")).unwrap()
    );
    assert!(json_to_value(&json!(3), id, &registry).is_err());

    let bytes = bincode::serialize(&JointName::from("Hips")).unwrap();
    assert_eq!(bytes, bincode::serialize("Hips").unwrap());
    assert_eq!(
        bincode::deserialize::<JointName>(&bytes).unwrap(),
        JointName::from("Hips")
    );
}
