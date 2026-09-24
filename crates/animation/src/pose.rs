use concerto_core::blend::Blendable;
use glam::{Quat, Vec3};

#[derive(Clone)]
pub struct JointPose {
    pub translation: Vec3,
    pub rotation: Quat,
    // Uniform scaling? No scaling at all?
    pub scale: Vec3,
}

pub struct Pose(Box<[JointPose]>);

impl Pose {
    pub fn identity(bone_count: usize) -> Pose {
        Pose(
            vec![
                JointPose {
                    translation: Vec3::ZERO,
                    rotation: Quat::IDENTITY,
                    scale: Vec3::ONE,
                };
                bone_count
            ]
            .into_boxed_slice(),
        )
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
