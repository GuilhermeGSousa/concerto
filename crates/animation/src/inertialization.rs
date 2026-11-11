use concerto_foundation::math::spring_damper::{
    angular_velocity, decay_spring_damper_exact, from_scaled_angle_axis, quat_abs,
};
use glam::{Quat, Vec3};

use crate::pose::Pose;

/// Linear and angular velocity per bone.
pub struct PoseVelocity {
    pub linear: Box<[Vec3]>,
    pub angular: Box<[Vec3]>,
}

impl PoseVelocity {
    /// A velocity of zero for `bone_count` bones.
    pub fn zero(bone_count: usize) -> Self {
        Self {
            linear: vec![Vec3::ZERO; bone_count].into_boxed_slice(),
            angular: vec![Vec3::ZERO; bone_count].into_boxed_slice(),
        }
    }

    /// The finite-difference velocity that moves `previous` to `next` in `dt` seconds.
    pub fn between(previous: &Pose, next: &Pose, dt: f32) -> Self {
        debug_assert_eq!(previous.len(), next.len());
        let (linear, angular) = previous
            .joints()
            .iter()
            .zip(next.joints())
            .map(|(previous, next)| {
                (
                    (next.translation - previous.translation) / dt,
                    angular_velocity(next.rotation, previous.rotation, dt),
                )
            })
            .unzip::<Vec3, Vec3, Vec<Vec3>, Vec<Vec3>>();
        Self {
            linear: linear.into_boxed_slice(),
            angular: angular.into_boxed_slice(),
        }
    }

    /// This velocity with every bone's linear and angular velocity multiplied by `factor`.
    pub fn scaled(&self, factor: f32) -> Self {
        Self {
            linear: self.linear.iter().map(|v| *v * factor).collect(),
            angular: self.angular.iter().map(|v| *v * factor).collect(),
        }
    }
}

const SETTLED_EPSILON: f32 = 1.0e-4;
const MIN_HALFLIFE: f32 = 1.0e-5;

fn rotation_vector(rotation: Quat) -> Vec3 {
    let rotation = quat_abs(rotation);
    let axis = Vec3::new(rotation.x, rotation.y, rotation.z);
    let sine = axis.length();
    if sine < 1.0e-8 {
        return axis * 2.0;
    }
    2.0 * sine.atan2(rotation.w) * axis / sine
}

/// Per-bone offsets that start at the difference between two poses and decay to zero.
pub struct Inertializer {
    offset_position: Box<[Vec3]>,
    offset_velocity: Box<[Vec3]>,
    offset_rotation: Box<[Quat]>,
    offset_angular_velocity: Box<[Vec3]>,
}

impl Inertializer {
    /// An inertializer for `bone_count` bones with every offset at zero.
    pub fn new(bone_count: usize) -> Self {
        Self {
            offset_position: vec![Vec3::ZERO; bone_count].into_boxed_slice(),
            offset_velocity: vec![Vec3::ZERO; bone_count].into_boxed_slice(),
            offset_rotation: vec![Quat::IDENTITY; bone_count].into_boxed_slice(),
            offset_angular_velocity: vec![Vec3::ZERO; bone_count].into_boxed_slice(),
        }
    }

    /// The number of bones this inertializer holds offsets for.
    pub fn bone_count(&self) -> usize {
        self.offset_position.len()
    }

    /// Sets the offsets so that `destination` plus the offsets reproduces `source`.
    pub fn transition(
        &mut self,
        source: &Pose,
        source_velocity: &PoseVelocity,
        destination: &Pose,
        destination_velocity: &PoseVelocity,
    ) {
        debug_assert_eq!(source.len(), self.offset_position.len());
        debug_assert_eq!(destination.len(), self.offset_position.len());
        debug_assert_eq!(source_velocity.linear.len(), self.offset_position.len());
        debug_assert_eq!(
            destination_velocity.linear.len(),
            self.offset_position.len()
        );
        for (bone, (source, destination)) in source
            .joints()
            .iter()
            .zip(destination.joints())
            .enumerate()
            .take(self.offset_position.len())
        {
            self.offset_position[bone] = source.translation - destination.translation;
            self.offset_velocity[bone] =
                source_velocity.linear[bone] - destination_velocity.linear[bone];
            self.offset_rotation[bone] = quat_abs(source.rotation * destination.rotation.inverse());
            self.offset_angular_velocity[bone] =
                source_velocity.angular[bone] - destination_velocity.angular[bone];
        }
    }

    /// Decays the offsets by `dt` and writes `destination` plus the offsets into `output`.
    pub fn update(
        &mut self,
        destination: &Pose,
        destination_velocity: &PoseVelocity,
        halflife: f32,
        dt: f32,
        output: &mut Pose,
    ) {
        debug_assert_eq!(destination.len(), self.offset_position.len());
        debug_assert_eq!(
            destination_velocity.linear.len(),
            self.offset_position.len()
        );
        debug_assert_eq!(output.len(), self.offset_position.len());
        let halflife = halflife.max(MIN_HALFLIFE);
        if dt > 0.0 {
            for bone in 0..self.offset_position.len() {
                let (position, velocity) = decay_spring_damper_exact(
                    self.offset_position[bone],
                    self.offset_velocity[bone],
                    halflife,
                    dt,
                );
                self.offset_position[bone] = position;
                self.offset_velocity[bone] = velocity;
                let (rotation, angular_velocity) = decay_spring_damper_exact(
                    rotation_vector(self.offset_rotation[bone]),
                    self.offset_angular_velocity[bone],
                    halflife,
                    dt,
                );
                self.offset_rotation[bone] = from_scaled_angle_axis(rotation);
                self.offset_angular_velocity[bone] = angular_velocity;
            }
        }
        for (bone, destination) in destination
            .joints()
            .iter()
            .enumerate()
            .take(self.offset_position.len())
        {
            if let Some(output) = output.joint_mut(bone) {
                output.translation = destination.translation + self.offset_position[bone];
                output.rotation = self.offset_rotation[bone] * destination.rotation;
                output.scale = destination.scale;
            }
        }
    }

    /// Whether every offset is small enough to ignore.
    pub fn is_settled(&self) -> bool {
        (0..self.offset_position.len()).all(|bone| {
            self.offset_position[bone].length() < SETTLED_EPSILON
                && self.offset_velocity[bone].length() < SETTLED_EPSILON
                && rotation_vector(self.offset_rotation[bone]).length() < SETTLED_EPSILON
                && self.offset_angular_velocity[bone].length() < SETTLED_EPSILON
        })
    }
}

#[cfg(test)]
mod tests {
    use concerto_foundation::math::spring_damper::halflife_to_damping;
    use glam::Quat;
    use proptest::prelude::*;

    use super::*;

    fn assert_close(a: Vec3, b: Vec3) {
        assert!(a.distance(b) < 1e-4, "{a:?} vs {b:?}");
    }

    #[test]
    fn between_gives_the_linear_and_angular_velocity_over_dt() {
        let previous = Pose::identity(2);
        let mut next = Pose::identity(2);
        next.joint_mut(0).unwrap().rotation = Quat::from_rotation_z(0.1);
        next.joint_mut(1).unwrap().translation = Vec3::new(1.0, -2.0, 0.5);

        let velocity = PoseVelocity::between(&previous, &next, 0.5);

        assert_close(velocity.linear[0], Vec3::ZERO);
        assert_close(velocity.angular[0], Vec3::new(0.0, 0.0, 0.2));
        assert_close(velocity.linear[1], Vec3::new(2.0, -4.0, 1.0));
        assert_close(velocity.angular[1], Vec3::ZERO);
    }

    fn pose_at(translation: Vec3, rotation: Quat) -> Pose {
        let mut pose = Pose::identity(1);
        let joint = pose.joint_mut(0).unwrap();
        joint.translation = translation;
        joint.rotation = rotation;
        pose
    }

    fn translation(pose: &Pose) -> Vec3 {
        pose.joints()[0].translation
    }

    fn transitioned(source: &Pose, destination: &Pose) -> Inertializer {
        let mut inertializer = Inertializer::new(1);
        inertializer.transition(
            source,
            &PoseVelocity::zero(1),
            destination,
            &PoseVelocity::zero(1),
        );
        inertializer
    }

    fn any_vec3() -> impl Strategy<Value = Vec3> {
        prop::array::uniform3(-5.0f32..5.0).prop_map(Vec3::from_array)
    }

    fn any_turn() -> impl Strategy<Value = Quat> {
        prop::array::uniform3(-1.0f32..1.0)
            .prop_map(|[x, y, z]| Quat::from_euler(glam::EulerRot::XYZ, x, y, z))
    }

    proptest! {
        #[test]
        fn one_update_decays_a_still_offset_by_the_critically_damped_closed_form(
            source_at in any_vec3(),
            destination_at in any_vec3(),
            source_turn in any_turn(),
            destination_turn in any_turn(),
            halflife in prop_oneof![Just(0.0f32), 0.01f32..1.0],
            dt in prop_oneof![Just(0.0f32), -1.0f32..0.0, 0.0f32..0.5],
        ) {
            let source = pose_at(source_at, source_turn);
            let destination = pose_at(destination_at, destination_turn);
            let mut inertializer = transitioned(&source, &destination);
            let mut output = Pose::identity(1);

            inertializer.update(&destination, &PoseVelocity::zero(1), halflife, dt, &mut output);

            let y = halflife_to_damping(halflife.max(MIN_HALFLIFE)) / 2.0;
            let left = if dt > 0.0 { (1.0 + y * dt) * (-y * dt).exp() } else { 1.0 };
            let expected = destination_at + (source_at - destination_at) * left;
            prop_assert!(translation(&output).is_finite());
            prop_assert!(
                translation(&output).distance(expected) < 1e-3,
                "{} != {expected}",
                translation(&output)
            );
            let rotation = output.joints()[0].rotation;
            let angle = source_turn.angle_between(destination_turn);
            prop_assert!(
                (rotation.angle_between(destination_turn) - angle * left).abs() < 5e-3,
                "{} of {angle} left",
                rotation.angle_between(destination_turn)
            );
        }
    }

    #[test]
    fn converges_to_the_destination_and_settles() {
        let destination = pose_at(Vec3::new(0.0, 1.0, 0.0), Quat::IDENTITY);
        let mut inertializer = transitioned(
            &pose_at(Vec3::new(2.0, 0.0, 0.0), Quat::from_rotation_z(1.0)),
            &destination,
        );
        assert!(!inertializer.is_settled());
        let mut output = Pose::identity(1);

        for _ in 0..600 {
            inertializer.update(
                &destination,
                &PoseVelocity::zero(1),
                0.1,
                1.0 / 60.0,
                &mut output,
            );
        }

        assert!(inertializer.is_settled());
        assert_close(translation(&output), translation(&destination));
        assert!(output.joints()[0].rotation.angle_between(Quat::IDENTITY) < 1e-3);
    }

    #[test]
    fn transition_during_a_decay_starts_from_the_output_pose() {
        let destination = Pose::identity(1);
        let mut inertializer = transitioned(
            &pose_at(Vec3::new(4.0, 0.0, 0.0), Quat::IDENTITY),
            &destination,
        );
        let mut output = Pose::identity(1);
        inertializer.update(&destination, &PoseVelocity::zero(1), 0.2, 0.1, &mut output);
        let running = translation(&output);

        let next = pose_at(Vec3::new(0.0, 5.0, 0.0), Quat::IDENTITY);
        inertializer.transition(
            &output,
            &PoseVelocity::zero(1),
            &next,
            &PoseVelocity::zero(1),
        );
        inertializer.update(&next, &PoseVelocity::zero(1), 0.2, 0.0, &mut output);

        assert_close(translation(&output), running);
    }
}
