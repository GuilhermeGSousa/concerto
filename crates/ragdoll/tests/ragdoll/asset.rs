use std::f32::consts::FRAC_PI_3;
use std::path::Path;

use concerto_foundation::assets::{
    Asset, AssetId,
    content::{ContentAssetHeader, check_encoding, decode_content_asset, write_content_asset},
    handle::AssetHandle,
};
use concerto_foundation::transform::Transform;
use concerto_physics::physics_pipeline::PhysicsPipeline;
use concerto_physics::physics_state::PhysicsState;
use concerto_physics::ragdoll::BodyPose;
use concerto_ragdoll::{
    PartStrength, Ragdoll, RagdollError, RagdollJoint, RagdollMotor, RagdollPart, RagdollShape,
    StrengthProfile,
};
use glam::{Quat, Vec3};
use proptest::prelude::*;

use crate::common::{BONE, spine_part, spine_ragdoll, spine_skeleton, world_with_floor};

fn encoding_dir() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/encoding"))
}

#[test]
fn a_ragdoll_built_in_code_saves_loads_and_drops_as_one_jointed_body() {
    let authored = spine_ragdoll();
    check_encoding(&authored, encoding_dir());
    let header = ContentAssetHeader {
        asset_id: AssetId::new(),
        references: vec![authored.skeleton.id()],
        kind: Ragdoll::name().to_string(),
        kind_version: Ragdoll::FORMAT_VERSION,
        provenance: None,
        uncooked: None,
        derived: None,
        embedded: Vec::new(),
    };
    let file = write_content_asset(&header, &bincode::serialize(&authored).unwrap()).unwrap();
    let (_, loaded): (_, Ragdoll) = decode_content_asset(&file).unwrap();
    assert_eq!(loaded, authored);

    let skeleton = spine_skeleton();
    let resolved = loaded
        .resolve(&skeleton)
        .expect("the ragdoll fits its skeleton");
    assert_eq!(
        resolved.bones(),
        [1, 2, 4],
        "parts are ordered parents first"
    );
    assert_eq!(resolved.part_of("Spine"), Some(1));
    assert_eq!(
        resolved.followed(),
        [None, Some(0), Some(1), Some(0), Some(2), Some(2)],
        "a bone without a part follows its nearest ancestor that has one"
    );

    let mut world = world_with_floor();
    let owner = world.spawn(Transform::default());
    let mut pipeline = PhysicsPipeline::new();
    let state = world.get_resource_mut::<PhysicsState>().unwrap();
    let root = BodyPose::new(Vec3::new(0.0, 2.0, 0.0), Quat::from_rotation_z(FRAC_PI_3));
    let settings = resolved.settings().clone();
    let start: Vec<BodyPose> = settings
        .parts
        .iter()
        .map(|part| root.then(part.pose))
        .collect();
    let ragdoll = state
        .create_ragdoll(resolved.built(), owner, &start)
        .expect("Jolt creates the ragdoll");
    for _ in 0..300 {
        pipeline.step(state);
    }
    let mut pose = Vec::new();
    state.ragdoll_pose(ragdoll, &mut pose);
    for (index, part) in settings.parts.iter().enumerate() {
        let speed = state
            .linear_velocity(state.ragdoll_bodies(ragdoll)[index])
            .length();
        assert!(
            speed < 0.05,
            "part {index} should be at rest, moved at {speed} m/s"
        );
        let tip = pose[index].transform_point(Vec3::new(0.0, BONE, 0.0));
        assert!(
            pose[index].translation.y > 0.0 && tip.y > 0.0,
            "part {index} should lie on the floor"
        );
        let Some(parent) = part.parent else {
            continue;
        };
        let anchor = pose[parent].transform_point(
            settings.parts[parent]
                .pose
                .inverse()
                .transform_point(part.pose.translation),
        );
        let gap = anchor.distance(pose[index].translation);
        assert!(
            gap < 0.02,
            "part {index} should stay joined to its parent, was {gap} m away"
        );
        let bend = (pose[parent].rotation * Vec3::Y).angle_between(pose[index].rotation * Vec3::Y);
        assert!(
            bend < 0.7,
            "part {index} should stay within its swing limit, was bent {bend} rad"
        );
    }
}

#[test]
fn a_ragdoll_that_does_not_fit_its_skeleton_is_refused_with_the_reason() {
    struct Case {
        name: &'static str,
        ragdoll: Ragdoll,
        error: RagdollError,
    }
    let empty = Ragdoll::new(AssetHandle::default());
    let cases = [
        Case {
            name: "no parts",
            ragdoll: empty.clone(),
            error: RagdollError::NoParts,
        },
        Case {
            name: "a bone the skeleton lacks",
            ragdoll: empty
                .clone()
                .with_part(spine_part("Hips"))
                .with_part(spine_part("Elbow")),
            error: RagdollError::UnknownBone {
                bone: "Elbow".into(),
            },
        },
        Case {
            name: "two parts on one bone",
            ragdoll: empty
                .clone()
                .with_part(spine_part("Hips"))
                .with_part(spine_part("Hips")),
            error: RagdollError::DuplicateBone {
                bone: "Hips".into(),
            },
        },
        Case {
            name: "two trees",
            ragdoll: empty
                .clone()
                .with_part(spine_part("Spine"))
                .with_part(spine_part("Tail")),
            error: RagdollError::SecondRoot {
                root: "Spine".into(),
                bone: "Tail".into(),
            },
        },
    ];
    let skeleton = spine_skeleton();
    for case in cases {
        assert_eq!(
            case.ragdoll.resolve(&skeleton).err(),
            Some(case.error),
            "{}",
            case.name
        );
    }
}

fn any_shape() -> impl Strategy<Value = RagdollShape> {
    prop_oneof![
        (0.0f32..2.0, 0.0f32..1.0).prop_map(|(half_height, radius)| RagdollShape::Capsule {
            half_height,
            radius
        }),
        (0.0f32..1.0, 0.0f32..1.0, 0.0f32..1.0).prop_map(|(x, y, z)| RagdollShape::Box {
            half_extents: Vec3::new(x, y, z)
        }),
        (0.0f32..1.0).prop_map(|radius| RagdollShape::Sphere { radius }),
    ]
}

fn any_vec3() -> impl Strategy<Value = Vec3> {
    (-1.0f32..1.0, -1.0f32..1.0, -1.0f32..1.0).prop_map(|(x, y, z)| Vec3::new(x, y, z))
}

fn any_part() -> impl Strategy<Value = RagdollPart> {
    (
        "[A-Za-z]{1,12}",
        any_shape(),
        any_vec3(),
        any_vec3(),
        (0.1f32..90.0, -3.0f32..0.0, 0.0f32..3.0),
        (0.0f32..30.0, 0.0f32..2.0, 0.0f32..1000.0),
    )
        .prop_map(|(bone, shape, offset, axis, numbers, motor)| {
            RagdollPart::new(bone.as_str(), shape)
                .with_offset(offset)
                .with_rotation(Quat::from_scaled_axis(axis))
                .with_mass(numbers.0)
                .with_joint(
                    RagdollJoint::new(axis, offset)
                        .with_swing(numbers.2, -numbers.1)
                        .with_twist(numbers.1, numbers.2)
                        .with_friction(motor.1)
                        .with_motor(RagdollMotor {
                            frequency: motor.0,
                            damping: motor.1,
                            torque_limit: motor.2,
                        }),
                )
        })
}

proptest! {
    #[test]
    fn any_ragdoll_decodes_to_what_was_encoded(parts in prop::collection::vec(any_part(), 0..20)) {
        let ragdoll = Ragdoll {
            skeleton: AssetHandle::weak(AssetId::new()),
            parts,
            profiles: vec![
                StrengthProfile::new("limp", PartStrength::Limp)
                    .with_part("Hips", PartStrength::powered(400.0, 120.0)),
            ],
        };
        let decoded: Ragdoll = bincode::deserialize(&bincode::serialize(&ragdoll).unwrap()).unwrap();
        prop_assert_eq!(decoded, ragdoll);
    }
}
