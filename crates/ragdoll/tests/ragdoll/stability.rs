use concerto_physics::ragdoll::BodyPose;
use concerto_ragdoll::testing::{RagdollWorld, reference_humanoid};
use glam::Vec3;
use proptest::prelude::*;

const MAX_IMPULSE: f32 = 500.0;
const MAX_SPEED: f32 = 50.0;
const MAX_JOINT_GAP: f32 = 0.6;
const RESTING_JOINT_GAP: f32 = 0.02;
const SECONDS: f32 = 12.0;

fn any_direction() -> impl Strategy<Value = Vec3> {
    (-1.0f32..1.0, -1.0f32..1.0, -1.0f32..1.0)
        .prop_filter_map("a direction needs a length", |(x, y, z)| {
            Vec3::new(x, y, z).try_normalize()
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    #[test]
    fn a_humanoid_hit_on_any_part_never_explodes_and_ends_at_rest(
        part in 0usize..17,
        direction in any_direction(),
        strength in 0.0f32..MAX_IMPULSE,
    ) {
        let humanoid = reference_humanoid();
        let mut world = RagdollWorld::new();
        let placed = world.place(&humanoid.resolved, BodyPose::from_translation(Vec3::new(0.0, 0.02, 0.0)));
        let bone = humanoid.resolved.bone_name(part).unwrap();
        world.impulse(&humanoid.resolved, placed, &bone, direction * strength);

        let steps = (SECONDS / 0.033) as usize;
        for step in 0..steps {
            world.step();
            let pose = world.pose(placed);
            for (index, part) in pose.iter().enumerate() {
                prop_assert!(
                    part.translation.is_finite() && part.rotation.is_finite(),
                    "part {index} is not finite at step {step}"
                );
            }
            for (index, speed) in world.speeds(placed).into_iter().enumerate() {
                prop_assert!(speed.is_finite() && speed <= MAX_SPEED * 1.01, "part {index} moves at {speed} m/s at step {step}");
            }
            for (joint, gap) in humanoid.resolved.joint_gaps(&pose).into_iter().enumerate() {
                prop_assert!(gap < MAX_JOINT_GAP, "joint {joint} is {gap} m apart at step {step}");
            }
        }
        let pose = world.pose(placed);
        for (joint, gap) in humanoid.resolved.joint_gaps(&pose).into_iter().enumerate() {
            prop_assert!(gap < RESTING_JOINT_GAP, "joint {joint} rests {gap} m apart");
        }
        for (index, speed) in world.speeds(placed).into_iter().enumerate() {
            prop_assert!(speed < 0.2, "part {index} still moves at {speed} m/s after {SECONDS} s");
        }
    }
}
