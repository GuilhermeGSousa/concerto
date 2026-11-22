use std::f32::consts::FRAC_PI_3;

use crate::common::{physics_world, register_bodies};
use concerto_foundation::transform::Transform;
use glam::{Quat, Vec3};

use concerto_physics::collider::Collider;
use concerto_physics::physics_pipeline::PhysicsPipeline;
use concerto_physics::physics_state::PhysicsState;
use concerto_physics::query::{QueryFilter, RagdollQuery};
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
            QueryFilter::default().with_ragdolls(RagdollQuery::Only),
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
            .cast_ray(
                above,
                Vec3::new(0.0, 0.0, -4.0),
                QueryFilter::default().with_ragdolls(RagdollQuery::Include)
            )
            .is_none(),
        "a destroyed ragdoll should leave no bodies behind"
    );
}

struct MotorChain {
    world: concerto_ecs::world::World,
    pipeline: PhysicsPipeline,
    settings: RagdollSettings,
    ragdoll: concerto_physics::ragdoll::RagdollId,
    pose: Vec<BodyPose>,
}

impl MotorChain {
    fn state(&mut self) -> &mut PhysicsState {
        self.world.get_resource_mut::<PhysicsState>().unwrap()
    }

    fn step(&mut self, seconds: f32) {
        let steps = (seconds / 0.033).round() as usize;
        let state = self.world.get_resource_mut::<PhysicsState>().unwrap();
        for _ in 0..steps {
            self.pipeline.step(state);
        }
        state.ragdoll_pose(self.ragdoll, &mut self.pose);
    }

    fn bend(&self, part: usize) -> Quat {
        let parent = self.settings.parts[part].parent.unwrap();
        self.pose[parent].rotation.inverse() * self.pose[part].rotation
    }
}

#[test]
fn a_chain_is_driven_to_a_pose_by_motors_and_kinematically() {
    use concerto_physics::body::BodyId;
    use concerto_physics::ragdoll::{JointDrive, JointTarget, MotorSettings};
    use concerto_physics::rigid_body::{MotionType, RigidBody};

    let strong = MotorSettings {
        frequency: 10.0,
        ..MotorSettings::default()
    };
    let mut settings = chain_settings();
    for part in &mut settings.parts {
        part.joint = part.joint.with_motor(strong).with_max_friction_torque(0.5);
    }
    let mut world = physics_world();
    world.spawn((
        Collider::cuboid(20.0, 0.5, 20.0),
        Transform::from_translation(Vec3::new(0.0, -0.5, 0.0)),
    ));
    let loose = world.spawn((
        RigidBody::default(),
        Collider::cuboid(0.25, 0.25, 0.25),
        Transform::from_translation(Vec3::new(0.8, 0.25, 0.0)),
    ));
    let owner = world.spawn(Transform::default());
    register_bodies(&mut world);
    let built = settings.build().unwrap();
    let hanging = placed(
        &settings,
        BodyPose::from_translation(Vec3::new(0.0, 1.8, 0.0)),
    );
    let state = world.get_resource_mut::<PhysicsState>().unwrap();
    let ragdoll = state.create_ragdoll(&built, owner, &hanging).unwrap();
    state.set_ragdoll_part_motion(ragdoll, 0, MotionType::Kinematic);
    let mut chain = MotorChain {
        world,
        pipeline: PhysicsPipeline::new(),
        settings,
        ragdoll,
        pose: Vec::new(),
    };

    let bent = Quat::from_rotation_z(-0.4);
    let target = vec![
        hanging[0],
        BodyPose::new(hanging[1].translation, bent),
        BodyPose::new(hanging[2].translation, bent * bent),
    ];
    chain.state().drive_ragdoll_motors(ragdoll, &target);
    chain.step(3.0);
    for part in 1..3 {
        let error = chain.bend(part).angle_between(bent);
        assert!(
            error < 0.1,
            "motors should hold part {part} at its target under gravity, was {error} rad off"
        );
        let impulse = chain.state().ragdoll_joint_motor_impulse(ragdoll, part);
        assert!(
            impulse.length() > 0.0,
            "the motor of part {part} should be working"
        );
    }

    let weak = JointDrive::Target(JointTarget {
        rotation: bent,
        motor: MotorSettings {
            torque_limit: 0.5,
            ..strong
        },
    });
    chain.state().set_ragdoll_joint_drive(ragdoll, 1, weak);
    chain.state().set_ragdoll_joint_drive(ragdoll, 2, weak);
    chain.step(3.0);
    let sag = chain.bend(1).angle_between(bent);
    assert!(
        sag > 0.2,
        "a motor limited to 0.5 N·m should sag under the chain's weight, was {sag} rad off"
    );

    chain
        .state()
        .set_ragdoll_joint_drive(ragdoll, 1, JointDrive::Off);
    chain
        .state()
        .set_ragdoll_joint_drive(ragdoll, 2, JointDrive::Off);
    chain.step(6.0);
    for part in 1..3 {
        let lean = (chain.pose[part].rotation * Vec3::NEG_Y).angle_between(Vec3::NEG_Y);
        assert!(
            lean < 0.25,
            "with motors off part {part} should hang, leaned {lean} rad"
        );
        let impulse = chain.state().ragdoll_joint_motor_impulse(ragdoll, part);
        assert_eq!(impulse, Vec3::ZERO);
    }

    for part in 0..3 {
        chain
            .state()
            .set_ragdoll_part_motion(ragdoll, part, MotionType::Kinematic);
    }
    let mut carried = hanging.clone();
    for step in 1..=30 {
        carried = placed(
            &chain.settings,
            BodyPose::from_translation(Vec3::new(0.05 * step as f32, 1.8, 0.0)),
        );
        chain.state().drive_ragdoll_kinematic(ragdoll, &carried);
        chain.step(0.033);
    }
    for (part, (read, target)) in chain.pose.iter().zip(&carried).enumerate() {
        let off = read.translation.distance(target.translation);
        assert!(
            off < 1e-3,
            "kinematic part {part} should be on its target, was {off} m off"
        );
        assert!(read.rotation.dot(target.rotation).abs() > 0.9999);
    }
    let body = *chain
        .world
        .get_component_for_entity::<BodyId>(loose)
        .unwrap();
    let pushed = chain.state().body_transform(body).translation.x;
    assert!(
        pushed > 1.2,
        "the kinematic chain should push the loose box out of its way, it was at x = {pushed}"
    );
}
