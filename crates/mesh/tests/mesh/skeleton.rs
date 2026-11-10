use std::any::TypeId;

use concerto_mesh::skeleton::{JointName, JointPose, Skeleton, SkeletonError};
use concerto_name::name;
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

struct Refusal {
    case: &'static str,
    result: Result<(), SkeletonError>,
    error: SkeletonError,
}

fn two_bones(parents: Vec<Option<u16>>) -> Result<(), SkeletonError> {
    let count = 2;
    Skeleton::new(
        vec![Mat4::IDENTITY; count],
        parents,
        vec!["a".into(), "b".into()],
        vec![JointPose::IDENTITY; count],
        vec![Uuid::nil(); count],
    )
    .map(|_| ())
}

#[test]
fn a_skeleton_refuses_mismatched_lengths_unordered_parents_and_poses_of_the_wrong_length() {
    let cases = [
        Refusal {
            case: "unequal lengths",
            result: two_bones(vec![None]),
            error: SkeletonError::LengthMismatch {
                field: "parents",
                expected: 2,
                actual: 1,
            },
        },
        Refusal {
            case: "a parent after its child",
            result: two_bones(vec![Some(1), None]),
            error: SkeletonError::ParentNotBefore { bone: 0, parent: 1 },
        },
        Refusal {
            case: "a bone its own parent",
            result: two_bones(vec![None, Some(1)]),
            error: SkeletonError::ParentNotBefore { bone: 1, parent: 1 },
        },
        Refusal {
            case: "a pose of the wrong length",
            result: chain()
                .forward_kinematics(&[JointPose::IDENTITY])
                .map(|_| ()),
            error: SkeletonError::PoseLength {
                expected: 3,
                actual: 1,
            },
        },
    ];
    for case in cases {
        assert_eq!(case.result, Err(case.error), "{}", case.case);
    }
}

#[test]
fn forward_kinematics_places_each_joint_under_its_parent() {
    let skeleton = chain();
    assert_eq!(skeleton.bone_count(), 3);
    assert_eq!(skeleton.bone_index(name!("spine")), Some(1));
    assert_eq!(skeleton.bone_index(name!("tail")), None);

    let global = skeleton.forward_kinematics(&skeleton.rest_pose).unwrap();
    assert_eq!(global.len(), 3);
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
    let registration = registry.get_by_name("JointName").unwrap();
    assert!(registration.data::<ReflectDefault>().is_some());
    assert!(registration.data::<ReflectJson>().is_some());
    assert_eq!(to_json(&JointName::from("Hips"), &registry), json!("Hips"));
    assert_eq!(
        json_to_value(&json!("Spine"), id, &registry).unwrap(),
        to_value(&JointName::from("Spine")).unwrap()
    );
    assert_eq!(
        json_to_value(&json!(3), id, &registry).unwrap_err(),
        "expected a string"
    );

    let bytes = bincode::serialize(&JointName::from("Hips")).unwrap();
    assert_eq!(bytes, bincode::serialize("Hips").unwrap());
    assert_eq!(
        bincode::deserialize::<JointName>(&bytes).unwrap(),
        JointName::from("Hips")
    );
}
