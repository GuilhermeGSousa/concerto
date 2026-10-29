use concerto_foundation::math::spring_damper::angular_velocity;
use glam::Vec3;

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

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic]
    fn between_asserts_on_length_mismatch() {
        PoseVelocity::between(&Pose::identity(2), &Pose::identity(3), 0.1);
    }
}
