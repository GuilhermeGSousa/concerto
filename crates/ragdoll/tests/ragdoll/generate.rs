use concerto_animation::retarget::RetargetRig;
use concerto_foundation::assets::handle::AssetHandle;
use concerto_foundation::transform::Transform;
use concerto_physics::physics_pipeline::PhysicsPipeline;
use concerto_physics::physics_state::PhysicsState;
use concerto_ragdoll::testing::{humanoid_rig, humanoid_skeleton};
use concerto_ragdoll::{
    GenerateError, HumanoidSettings, Ragdoll, RagdollShape, Segment, Trial, generate_humanoid,
    run_trial,
};
use glam::{Quat, Vec3};

use crate::common::world_with_floor;

fn generated(rig: &RetargetRig, total_mass: f32) -> Result<Ragdoll, GenerateError> {
    let skeleton = humanoid_skeleton();
    let rig = rig.resolve(&skeleton, None).unwrap();
    generate_humanoid(
        AssetHandle::default(),
        &skeleton,
        &rig,
        &[],
        &HumanoidSettings::default().with_total_mass(total_mass),
    )
}

#[test]
fn a_generated_humanoid_has_its_mass_its_one_way_knees_and_a_body_that_rests_in_its_rest_pose() {
    let whole = Segment::Pelvis.mass_fraction()
        + Segment::Spine.mass_fraction()
        + Segment::Head.mass_fraction()
        + 2.0
            * (Segment::UpperArm.mass_fraction()
                + Segment::LowerArm.mass_fraction()
                + Segment::Hand.mass_fraction()
                + Segment::Thigh.mass_fraction()
                + Segment::Shin.mass_fraction()
                + Segment::Foot.mass_fraction());
    assert!(
        (whole - 1.0).abs() < 1e-5,
        "segment fractions sum to {whole}"
    );
    for total in [75.0, 48.0, 110.0] {
        let ragdoll = generated(&humanoid_rig(), total).unwrap();
        assert_eq!(ragdoll.parts.len(), 17);
        let mass: f32 = ragdoll.parts.iter().map(|part| part.mass).sum();
        assert!(
            (mass - total).abs() < 1e-3,
            "parts weigh {mass} kg of {total}"
        );
    }

    struct Hinge {
        bone: &'static str,
        bone_direction: Vec3,
        bends_towards: Vec3,
    }
    let hinges = [
        Hinge {
            bone: "calf_l",
            bone_direction: Vec3::NEG_Y,
            bends_towards: Vec3::NEG_Z,
        },
        Hinge {
            bone: "calf_r",
            bone_direction: Vec3::NEG_Y,
            bends_towards: Vec3::NEG_Z,
        },
        Hinge {
            bone: "lowerarm_l",
            bone_direction: Vec3::X,
            bends_towards: Vec3::Z,
        },
        Hinge {
            bone: "lowerarm_r",
            bone_direction: Vec3::NEG_X,
            bends_towards: Vec3::Z,
        },
    ];
    let ragdoll = generated(&humanoid_rig(), 75.0).unwrap();
    for hinge in hinges {
        let joint = ragdoll.part(hinge.bone).unwrap().joint;
        assert_eq!(joint.twist_min, 0.0, "{} cannot bend backwards", hinge.bone);
        assert!(joint.twist_max > 2.0, "{} bends", hinge.bone);
        assert_eq!(joint.swing_normal, 0.0);
        assert_eq!(joint.swing_plane, 0.0);
        let bent = Quat::from_axis_angle(joint.twist_axis.normalize(), 0.5) * hinge.bone_direction;
        assert!(
            bent.dot(hinge.bends_towards) > 0.4,
            "{} should bend towards {}, went to {bent}",
            hinge.bone,
            hinge.bends_towards
        );
    }

    let mut missing = humanoid_rig();
    missing.chains.retain(|chain| &*chain.name != "neck");
    assert_eq!(
        generated(&missing, 75.0).err(),
        Some(GenerateError::MissingChain {
            chain: "neck".into()
        })
    );

    let skeleton = humanoid_skeleton();
    let resolved = ragdoll
        .resolve(&skeleton)
        .expect("the generated ragdoll fits");
    let mut world = world_with_floor();
    let owner = world.spawn(Transform::default());
    let mut pipeline = PhysicsPipeline::new();
    let state = world.get_resource_mut::<PhysicsState>().unwrap();
    let rest: Vec<_> = resolved
        .settings()
        .parts
        .iter()
        .map(|part| part.pose)
        .collect();
    let body = state
        .create_ragdoll(resolved.built(), owner, &rest)
        .unwrap();
    pipeline.step(state);
    pipeline.step(state);
    for (part, id) in state.ragdoll_bodies(body).iter().enumerate() {
        let speed = state.linear_velocity(*id).length();
        assert!(
            speed < 1.5,
            "part {part} ({:?}) is thrown at {speed} m/s from the rest pose: parts interpenetrate",
            resolved.bone_name(part)
        );
    }
    for _ in 0..300 {
        pipeline.step(state);
    }
    for (part, id) in state.ragdoll_bodies(body).iter().enumerate() {
        let speed = state.linear_velocity(*id).length();
        assert!(
            speed < 0.1,
            "part {part} should come to rest on the floor, moved at {speed} m/s"
        );
    }

    let report = run_trial(&resolved, Trial::Drop { height: 0.5 }, 10.0).unwrap();
    assert!(report.overlaps.is_empty(), "{:?}", report.overlaps);
    assert!(
        report.rested_after.is_some(),
        "a dropped body comes to rest"
    );
    assert!(report.peak_speed > 1.0 && report.peak_speed < 20.0);
    assert!(report.peak_joint_gap < 0.05);
    let poked = Trial::Poke {
        bone: "Head".into(),
        impulse: Vec3::new(0.0, 0.0, 30.0),
    };
    assert!(
        run_trial(&resolved, poked, 10.0)
            .unwrap()
            .rested_after
            .is_some()
    );
    let nowhere = Trial::Poke {
        bone: "tail".into(),
        impulse: Vec3::Z,
    };
    assert_eq!(run_trial(&resolved, nowhere, 1.0), None);

    let mut fat = ragdoll.clone();
    fat.part_mut("thigh_l").unwrap().shape = RagdollShape::Capsule {
        half_height: 0.15,
        radius: 0.3,
    };
    let fat = fat.resolve(&skeleton).unwrap();
    let overlaps = fat.overlapping_parts();
    let thigh = fat.part_of("thigh_l").unwrap();
    let other = fat.part_of("thigh_r").unwrap();
    assert!(
        overlaps
            .iter()
            .any(|pair| pair.first.min(pair.second) == thigh.min(other)
                && pair.first.max(pair.second) == thigh.max(other)
                && pair.depth > 0.1),
        "a thigh as wide as the hips overlaps the other thigh: {overlaps:?}"
    );
}
