use std::f32::consts::FRAC_PI_3;

use crate::common::{physics_world, register_bodies};
use concerto_foundation::transform::Transform;
use glam::{Quat, Vec3};

use concerto_physics::collider::Collider;
use concerto_physics::physics_pipeline::PhysicsPipeline;
use concerto_physics::physics_state::PhysicsState;
use concerto_physics::query::QueryFilter;
use concerto_physics::ragdoll::{
    BodyPose, JointSettings, PartSettings, PartShape, RagdollSettings,
};

const LINK: f32 = 0.5;
const SWING: f32 = 0.6;

pub fn chain_settings() -> RagdollSettings {
    let link = |index: usize| {
        PartSettings::new(
            BodyPose::from_translation(Vec3::new(0.0, -LINK * index as f32, 0.0)),
            PartShape::Capsule {
                half_height: 0.15,
                radius: 0.08,
            },
        )
        .with_shape_offset(BodyPose::from_translation(Vec3::new(0.0, -LINK / 2.0, 0.0)))
        .with_mass(2.0)
    };
    let joint = JointSettings::new(Vec3::NEG_Y, Vec3::X)
        .with_swing(SWING, SWING)
        .with_twist(-0.3, 0.3);
    RagdollSettings::new()
        .with_part(link(0))
        .with_part(link(1).with_parent(0, joint))
        .with_part(link(2).with_parent(1, joint))
}

pub fn placed(settings: &RagdollSettings, root: BodyPose) -> Vec<BodyPose> {
    settings
        .parts
        .iter()
        .map(|part| root.then(part.pose))
        .collect()
}

#[test]
fn a_three_part_chain_dropped_on_a_floor_stays_joined_within_its_limits_and_comes_to_rest() {
    let mut world = physics_world();
    let mut pipeline = PhysicsPipeline::new();
    world.spawn((
        Collider::cuboid(20.0, 0.5, 20.0),
        Transform::from_translation(Vec3::new(0.0, -0.5, 0.0)),
    ));
    let owner = world.spawn(Transform::default());
    register_bodies(&mut world);
    let settings = chain_settings();
    let built = settings.build().expect("the chain should build");
    assert_eq!(built.part_count(), 3);
    let state = world.get_resource_mut::<PhysicsState>().unwrap();

    let root = BodyPose::new(Vec3::new(0.0, 2.5, 0.0), Quat::from_rotation_z(FRAC_PI_3));
    let start = placed(&settings, root);
    assert!(
        state.create_ragdoll(&built, owner, &start[..2]).is_none(),
        "a pose with a part missing is refused"
    );
    let ragdoll = state
        .create_ragdoll(&built, owner, &start)
        .expect("Jolt should create the chain");
    assert_eq!(state.ragdoll_bodies(ragdoll).len(), 3);

    let middle = start[1].translation + start[1].rotation * Vec3::new(0.0, -LINK / 2.0, 0.0);
    let hit = state
        .cast_ray(
            middle + Vec3::new(0.0, 0.0, 2.0),
            Vec3::new(0.0, 0.0, -4.0),
            QueryFilter::default(),
        )
        .expect("a ray at the middle link should hit it");
    assert_eq!(hit.entity, Some(owner));
    assert_eq!(hit.part, Some(1));

    let mut pose = Vec::new();
    for _ in 0..300 {
        pipeline.step(state);
    }
    state.ragdoll_pose(ragdoll, &mut pose);
    for (index, part) in settings.parts.iter().enumerate() {
        let body = state.ragdoll_bodies(ragdoll)[index];
        let speed = state.linear_velocity(body).length();
        assert!(
            speed < 0.05,
            "part {index} should be at rest, moved at {speed} m/s"
        );
        let lowest = pose[index]
            .translation
            .y
            .min((pose[index].translation + pose[index].rotation * Vec3::new(0.0, -LINK, 0.0)).y);
        assert!(
            lowest > 0.0,
            "part {index} should lie on the floor, was at y = {lowest}"
        );
        let Some(parent) = part.parent else {
            continue;
        };
        let anchor = pose[parent].translation
            + pose[parent].rotation
                * (part.pose.translation - settings.parts[parent].pose.translation);
        let gap = anchor.distance(pose[index].translation);
        assert!(
            gap < 0.02,
            "part {index} should stay joined to its parent, was {gap} m away"
        );
        let bend =
            (pose[parent].rotation * Vec3::NEG_Y).angle_between(pose[index].rotation * Vec3::NEG_Y);
        assert!(
            bend < SWING + 0.1,
            "part {index} should stay within its swing limit of {SWING} rad, was bent {bend} rad"
        );
    }

    let lifted = placed(
        &settings,
        BodyPose::from_translation(Vec3::new(3.0, 9.0, -2.0)),
    );
    state.set_ragdoll_pose(ragdoll, &lifted);
    state.ragdoll_pose(ragdoll, &mut pose);
    for (read, set) in pose.iter().zip(&lifted) {
        assert!(read.translation.distance(set.translation) < 1e-4);
        assert!(read.rotation.dot(set.rotation).abs() > 0.9999);
    }

    state.destroy_ragdoll(ragdoll);
    assert!(state.ragdoll_bodies(ragdoll).is_empty());
    let above = lifted[1].translation + Vec3::new(0.0, -LINK / 2.0, 2.0);
    assert!(
        state
            .cast_ray(above, Vec3::new(0.0, 0.0, -4.0), QueryFilter::default())
            .is_none(),
        "a destroyed ragdoll should leave no bodies behind"
    );
}
