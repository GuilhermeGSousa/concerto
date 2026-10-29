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
            if let Some(output) = output.get_joint_pose_mut(bone) {
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
    use glam::Quat;

    use super::*;

    fn assert_close(a: Vec3, b: Vec3) {
        assert!(a.distance(b) < 1e-4, "{a:?} vs {b:?}");
    }

    #[test]
    fn zero_has_one_entry_per_bone() {
        let velocity = PoseVelocity::zero(3);
        assert_eq!(velocity.linear.len(), 3);
        assert_eq!(velocity.angular.len(), 3);
        assert!(velocity.linear.iter().all(|v| *v == Vec3::ZERO));
        assert!(velocity.angular.iter().all(|v| *v == Vec3::ZERO));
    }

    #[test]
    fn between_gives_linear_difference_over_dt() {
        let previous = Pose::identity(2);
        let mut next = Pose::identity(2);
        next.get_joint_pose_mut(1).unwrap().translation = Vec3::new(1.0, -2.0, 0.5);

        let velocity = PoseVelocity::between(&previous, &next, 0.5);

        assert_close(velocity.linear[0], Vec3::ZERO);
        assert_close(velocity.linear[1], Vec3::new(2.0, -4.0, 1.0));
    }

    #[test]
    fn between_gives_angular_velocity_about_the_rotation_axis() {
        let previous = Pose::identity(1);
        let mut next = Pose::identity(1);
        next.get_joint_pose_mut(0).unwrap().rotation = Quat::from_rotation_z(0.1);

        let velocity = PoseVelocity::between(&previous, &next, 0.5);

        assert_close(velocity.angular[0], Vec3::new(0.0, 0.0, 0.2));
        assert_close(velocity.linear[0], Vec3::ZERO);
    }

    fn pose_at(translation: Vec3, rotation: Quat) -> Pose {
        let mut pose = Pose::identity(1);
        let joint = pose.get_joint_pose_mut(0).unwrap();
        joint.translation = translation;
        joint.rotation = rotation;
        pose
    }

    fn translation(pose: &Pose) -> Vec3 {
        pose.joints()[0].translation
    }

    #[test]
    fn transition_frame_output_equals_source() {
        let source = pose_at(Vec3::new(1.0, 2.0, 3.0), Quat::from_rotation_y(0.7));
        let destination = pose_at(Vec3::new(-1.0, 0.0, 4.0), Quat::from_rotation_x(-0.3));
        let mut inertializer = Inertializer::new(1);
        inertializer.transition(
            &source,
            &PoseVelocity::zero(1),
            &destination,
            &PoseVelocity::zero(1),
        );
        let mut output = Pose::identity(1);

        inertializer.update(&destination, &PoseVelocity::zero(1), 0.2, 0.0, &mut output);

        assert_close(translation(&output), translation(&source));
        assert!(
            output.joints()[0]
                .rotation
                .angle_between(source.joints()[0].rotation)
                < 1e-4
        );
    }

    #[test]
    fn zero_velocity_offset_follows_the_closed_form_after_one_halflife() {
        let source = pose_at(Vec3::new(1.0, 0.0, 0.0), Quat::IDENTITY);
        let destination = Pose::identity(1);
        let mut inertializer = Inertializer::new(1);
        inertializer.transition(
            &source,
            &PoseVelocity::zero(1),
            &destination,
            &PoseVelocity::zero(1),
        );
        let mut output = Pose::identity(1);

        inertializer.update(
            &destination,
            &PoseVelocity::zero(1),
            0.25,
            0.25,
            &mut output,
        );

        let y = std::f32::consts::LN_2 * 2.0 / 0.25;
        let expected = (1.0 + y * 0.25) * (-y * 0.25).exp();
        assert!((translation(&output).x - expected).abs() < 1e-3);
    }

    #[test]
    fn converges_to_the_destination_and_settles() {
        let source = pose_at(Vec3::new(2.0, 0.0, 0.0), Quat::from_rotation_z(1.0));
        let destination = pose_at(Vec3::new(0.0, 1.0, 0.0), Quat::IDENTITY);
        let mut inertializer = Inertializer::new(1);
        inertializer.transition(
            &source,
            &PoseVelocity::zero(1),
            &destination,
            &PoseVelocity::zero(1),
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
        let first = pose_at(Vec3::new(4.0, 0.0, 0.0), Quat::IDENTITY);
        let destination = Pose::identity(1);
        let mut inertializer = Inertializer::new(1);
        inertializer.transition(
            &first,
            &PoseVelocity::zero(1),
            &destination,
            &PoseVelocity::zero(1),
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

    #[test]
    fn non_positive_dt_does_not_decay() {
        let source = pose_at(Vec3::X, Quat::IDENTITY);
        let destination = Pose::identity(1);
        let mut inertializer = Inertializer::new(1);
        inertializer.transition(
            &source,
            &PoseVelocity::zero(1),
            &destination,
            &PoseVelocity::zero(1),
        );
        let mut output = Pose::identity(1);

        inertializer.update(&destination, &PoseVelocity::zero(1), 0.2, -1.0, &mut output);

        assert_close(translation(&output), Vec3::X);
    }

    #[test]
    fn tiny_halflife_is_clamped_and_stays_finite() {
        let source = pose_at(Vec3::X, Quat::IDENTITY);
        let destination = Pose::identity(1);
        let mut inertializer = Inertializer::new(1);
        inertializer.transition(
            &source,
            &PoseVelocity::zero(1),
            &destination,
            &PoseVelocity::zero(1),
        );
        let mut output = Pose::identity(1);

        inertializer.update(&destination, &PoseVelocity::zero(1), 0.0, 0.1, &mut output);

        assert!(translation(&output).is_finite());
        assert!(translation(&output).length() < 1e-3);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic]
    fn between_asserts_on_length_mismatch() {
        PoseVelocity::between(&Pose::identity(2), &Pose::identity(3), 0.1);
    }
}
