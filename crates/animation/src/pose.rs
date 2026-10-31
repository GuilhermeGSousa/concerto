use concerto_foundation::blend::Blendable;
pub use concerto_mesh::skeleton::JointPose;

pub struct Pose(Box<[JointPose]>);

impl Pose {
    pub fn identity(bone_count: usize) -> Pose {
        Pose(vec![JointPose::IDENTITY; bone_count].into_boxed_slice())
    }

    /// Builds a pose from joints in bone order.
    pub fn from_joints(joints: impl Into<Box<[JointPose]>>) -> Pose {
        Pose(joints.into())
    }

    /// The number of joints in this pose.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether this pose has no joints.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The joints of this pose, in bone order.
    pub fn joints(&self) -> &[JointPose] {
        &self.0
    }

    pub fn get_joint_pose(&mut self, bone_index: usize) -> Option<&JointPose> {
        self.0.get(bone_index)
    }

    pub fn get_joint_pose_mut(&mut self, bone_index: usize) -> Option<&mut JointPose> {
        self.0.get_mut(bone_index)
    }

    pub fn blend(&mut self, other: &Pose, weight: f32) {
        for (joint, other_joint) in self.0.iter_mut().zip(other.0.iter()) {
            joint.translation = joint.translation.lerp(other_joint.translation, weight);
            joint.rotation = joint.rotation.slerp(other_joint.rotation, weight);
            joint.scale = joint.scale.lerp(other_joint.scale, weight);
        }
    }

    /// Overwrites this pose's joints with those of `other` (same skeleton / length).
    pub fn copy_from(&mut self, other: &Pose) {
        for (joint, other_joint) in self.0.iter_mut().zip(other.0.iter()) {
            *joint = other_joint.clone();
        }
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
        self.free_poses
            .pop()
            .unwrap_or(Pose::identity(self.bone_count))
    }

    pub(crate) fn release(&mut self, pose: Pose) {
        self.free_poses.push(pose);
    }
}

#[cfg(test)]
mod tests {
    use glam::{Quat, Vec3};

    use super::{JointPose, Pose};

    #[test]
    fn a_pose_built_from_joints_keeps_them_in_order() {
        let joints = vec![
            JointPose::IDENTITY,
            JointPose {
                translation: Vec3::new(1.0, 2.0, 3.0),
                rotation: Quat::from_rotation_y(0.5),
                scale: Vec3::splat(2.0),
            },
        ];

        let mut pose = Pose::from_joints(joints.clone());

        assert_eq!(pose.len(), 2);
        assert_eq!(pose.joints(), joints.as_slice());
        assert_eq!(pose.get_joint_pose(1), Some(&joints[1]));
    }
}
