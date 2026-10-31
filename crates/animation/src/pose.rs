use concerto_foundation::blend::Blendable;
pub use concerto_mesh::skeleton::JointPose;
use glam::{Quat, Vec3};

/// How the root moved this frame, in its parent's space, read from the way it faced last frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RootMotion {
    pub translation: Vec3,
    pub rotation: Quat,
}

impl RootMotion {
    /// No motion.
    pub const IDENTITY: RootMotion = RootMotion {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
    };

    pub(crate) fn between(from: &JointPose, to: &JointPose, rest: Quat) -> RootMotion {
        let from_rest = rest.inverse();
        let facing = (from.rotation * from_rest).inverse();
        RootMotion {
            translation: facing * (to.translation - from.translation),
            rotation: facing * (to.rotation * from_rest),
        }
    }

    pub(crate) fn then(self, next: RootMotion) -> RootMotion {
        RootMotion {
            translation: self.translation + self.rotation * next.translation,
            rotation: self.rotation * next.rotation,
        }
    }
}

pub struct Pose {
    joints: Box<[JointPose]>,
    root_motion: RootMotion,
}

impl Pose {
    pub fn identity(bone_count: usize) -> Pose {
        Pose::from_joints(vec![JointPose::IDENTITY; bone_count])
    }

    /// Builds a pose from joints in bone order.
    pub fn from_joints(joints: impl Into<Box<[JointPose]>>) -> Pose {
        Pose {
            joints: joints.into(),
            root_motion: RootMotion::IDENTITY,
        }
    }

    /// The number of joints in this pose.
    pub fn len(&self) -> usize {
        self.joints.len()
    }

    /// Whether this pose has no joints.
    pub fn is_empty(&self) -> bool {
        self.joints.is_empty()
    }

    /// The joints of this pose, in bone order.
    pub fn joints(&self) -> &[JointPose] {
        &self.joints
    }

    /// How the root moved this frame.
    pub fn root_motion(&self) -> RootMotion {
        self.root_motion
    }

    /// Sets how the root moved this frame.
    pub fn set_root_motion(&mut self, root_motion: RootMotion) {
        self.root_motion = root_motion;
    }

    pub fn get_joint_pose(&mut self, bone_index: usize) -> Option<&JointPose> {
        self.joints.get(bone_index)
    }

    pub fn get_joint_pose_mut(&mut self, bone_index: usize) -> Option<&mut JointPose> {
        self.joints.get_mut(bone_index)
    }

    pub fn blend(&mut self, other: &Pose, weight: f32) {
        for (joint, other_joint) in self.joints.iter_mut().zip(other.joints.iter()) {
            joint.translation = joint.translation.lerp(other_joint.translation, weight);
            joint.rotation = joint.rotation.slerp(other_joint.rotation, weight);
            joint.scale = joint.scale.lerp(other_joint.scale, weight);
        }
        self.root_motion = RootMotion {
            translation: self
                .root_motion
                .translation
                .lerp(other.root_motion.translation, weight),
            rotation: self
                .root_motion
                .rotation
                .slerp(other.root_motion.rotation, weight),
        };
    }

    /// Overwrites this pose's joints and root motion with those of `other` (same skeleton / length).
    pub fn copy_from(&mut self, other: &Pose) {
        for (joint, other_joint) in self.joints.iter_mut().zip(other.joints.iter()) {
            *joint = other_joint.clone();
        }
        self.root_motion = other.root_motion;
    }
}

impl Blendable for Pose {
    fn interpolate(_from: Self, _to: Self, _t: f32) -> Self {
        todo!()
    }
}

pub struct EvaluatedPose {
    pub pose: Pose,
    pub weight: f32,
}

pub struct PosePool {
    free_poses: Vec<Pose>,
    bone_count: usize,
}

impl PosePool {
    pub(crate) fn new(bone_count: usize) -> Self {
        Self {
            free_poses: Vec::new(),
            bone_count,
        }
    }

    pub(crate) fn acquire(&mut self) -> Pose {
        let mut pose = self
            .free_poses
            .pop()
            .unwrap_or(Pose::identity(self.bone_count));
        pose.root_motion = RootMotion::IDENTITY;
        pose
    }

    pub(crate) fn release(&mut self, pose: Pose) {
        self.free_poses.push(pose);
    }
}

#[cfg(test)]
mod tests {
    use glam::{Quat, Vec3};

    use super::{JointPose, Pose, PosePool, RootMotion};

    fn moved() -> RootMotion {
        RootMotion {
            translation: Vec3::new(2.0, 0.0, 0.0),
            rotation: Quat::from_rotation_y(0.8),
        }
    }

    fn joint(translation: Vec3, rotation: Quat) -> JointPose {
        JointPose {
            translation,
            rotation,
            scale: Vec3::ONE,
        }
    }

    #[test]
    fn copy_from_carries_root_motion() {
        let mut source = Pose::identity(1);
        source.set_root_motion(moved());

        let mut copy = Pose::identity(1);
        copy.copy_from(&source);

        assert_eq!(copy.root_motion(), moved());
    }

    #[test]
    fn blend_lerps_root_translation_and_slerps_root_rotation() {
        let mut other = Pose::identity(1);
        other.set_root_motion(moved());

        let mut pose = Pose::identity(1);
        pose.blend(&other, 0.25);

        assert!(
            pose.root_motion()
                .translation
                .abs_diff_eq(Vec3::new(0.5, 0.0, 0.0), 1e-5)
        );
        assert!(
            pose.root_motion()
                .rotation
                .abs_diff_eq(Quat::from_rotation_y(0.2), 1e-5)
        );
    }

    #[test]
    fn acquire_clears_the_root_motion_of_a_recycled_pose() {
        let mut pool = PosePool::new(1);
        let mut recycled = pool.acquire();
        recycled.set_root_motion(moved());
        pool.release(recycled);

        assert_eq!(pool.acquire().root_motion(), RootMotion::IDENTITY);
    }

    #[test]
    fn two_stretches_compose_to_the_whole_journey() {
        let rest = Quat::from_rotation_x(-1.2);
        let start = joint(Vec3::new(1.0, 0.0, 0.0), Quat::from_rotation_y(0.3) * rest);
        let middle = joint(Vec3::new(1.0, 0.0, 2.0), Quat::from_rotation_y(0.9) * rest);
        let end = joint(Vec3::new(4.0, 1.0, 2.0), Quat::from_rotation_y(1.4) * rest);

        let composed = RootMotion::between(&start, &middle, rest)
            .then(RootMotion::between(&middle, &end, rest));
        let whole = RootMotion::between(&start, &end, rest);

        assert!(composed.translation.abs_diff_eq(whole.translation, 1e-5));
        assert!(composed.rotation.abs_diff_eq(whole.rotation, 1e-5));
    }
}
