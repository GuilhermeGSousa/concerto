use concerto_foundation::{assets::AssetId, blend::Blendable};
pub use concerto_mesh::skeleton::JointPose;
use glam::{Quat, Vec3};

use crate::attribute::{PoseAttribute, PoseAttributes};

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

    /// Blends toward `other` by `weight`: translation by lerp, rotation by slerp.
    pub fn blend(self, other: RootMotion, weight: f32) -> RootMotion {
        RootMotion {
            translation: self.translation.lerp(other.translation, weight),
            rotation: self.rotation.slerp(other.rotation, weight),
        }
    }

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

/// The part of `rotation` that turns about the unit vector `axis`, or identity for a half turn across it.
pub fn twist_about(rotation: Quat, axis: Vec3) -> Quat {
    let along = axis * rotation.xyz().dot(axis);
    let twist = Quat::from_xyzw(along.x, along.y, along.z, rotation.w);
    if twist.length_squared() < f32::EPSILON {
        return Quat::IDENTITY;
    }
    twist.normalize()
}

/// Layers an additive `delta` over `base` at `weight`: the rotation turns by the delta slerped from identity, the translation moves by the weighted delta, and scale is left alone.
pub fn apply_additive(base: &mut JointPose, delta: &JointPose, weight: f32) {
    base.rotation = Quat::IDENTITY.slerp(delta.rotation, weight) * base.rotation;
    base.translation += delta.translation * weight;
}

/// A clip event crossed this frame: the key on its clip, and the weight its source has in the frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FiredEvent {
    pub clip: AssetId,
    pub key: u32,
    pub time: f32,
    pub weight: f32,
    pub mirrored: bool,
}

/// The joints of a skeleton, in bone order.
#[derive(Clone, Debug, PartialEq)]
pub struct Pose {
    joints: Box<[JointPose]>,
}

impl Pose {
    /// A pose of `bone_count` identity joints.
    pub fn identity(bone_count: usize) -> Pose {
        Pose::from_joints(vec![JointPose::IDENTITY; bone_count])
    }

    /// Builds a pose from joints in bone order.
    pub fn from_joints(joints: impl Into<Box<[JointPose]>>) -> Pose {
        Pose {
            joints: joints.into(),
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

    /// The joints of this pose, in bone order, to change.
    pub fn joints_mut(&mut self) -> &mut [JointPose] {
        &mut self.joints
    }

    /// The joint of the bone at `bone_index`.
    pub fn joint(&self, bone_index: usize) -> Option<&JointPose> {
        self.joints.get(bone_index)
    }

    /// The joint of the bone at `bone_index`, to change.
    pub fn joint_mut(&mut self, bone_index: usize) -> Option<&mut JointPose> {
        self.joints.get_mut(bone_index)
    }

    /// Moves each joint toward the one of `other` by `weight`, lerping translation and scale and slerping rotation.
    pub fn blend(&mut self, other: &Pose, weight: f32) {
        for (joint, other_joint) in self.joints.iter_mut().zip(other.joints.iter()) {
            joint.translation = joint.translation.lerp(other_joint.translation, weight);
            joint.rotation = joint.rotation.slerp(other_joint.rotation, weight);
            joint.scale = joint.scale.lerp(other_joint.scale, weight);
        }
    }

    /// Overwrites this pose's joints with those of `other` (same skeleton / length).
    pub fn copy_from(&mut self, other: &Pose) {
        for (joint, other_joint) in self.joints.iter_mut().zip(other.joints.iter()) {
            *joint = other_joint.clone();
        }
    }
}

/// What an animation graph gives for one frame: the pose, its attributes and the clip events crossed.
pub struct AnimationFrame {
    pose: Pose,
    attributes: PoseAttributes,
    pub(crate) events: Vec<FiredEvent>,
}

impl AnimationFrame {
    /// A frame of `bone_count` identity joints, no events.
    pub fn identity(bone_count: usize) -> AnimationFrame {
        AnimationFrame::new(Pose::identity(bone_count))
    }

    /// A frame of `pose`, with no events.
    pub fn new(pose: Pose) -> AnimationFrame {
        AnimationFrame {
            pose,
            attributes: PoseAttributes::default(),
            events: Vec::new(),
        }
    }

    /// The pose of this frame.
    pub fn pose(&self) -> &Pose {
        &self.pose
    }

    /// The pose of this frame, to change.
    pub fn pose_mut(&mut self) -> &mut Pose {
        &mut self.pose
    }

    /// The per-bone attributes this frame carries.
    pub fn attributes(&self) -> &PoseAttributes {
        &self.attributes
    }

    /// The per-bone attributes this frame carries, to change.
    pub fn attributes_mut(&mut self) -> &mut PoseAttributes {
        &mut self.attributes
    }

    /// The value of `A` on `bone`, neutral when no node set it.
    pub fn attribute<A: PoseAttribute>(&self, bone: usize) -> A {
        self.attributes.get(bone)
    }

    /// The number of joints in this frame's pose.
    pub fn len(&self) -> usize {
        self.pose.len()
    }

    /// Whether this frame's pose has no joints.
    pub fn is_empty(&self) -> bool {
        self.pose.is_empty()
    }

    /// The joints of this frame's pose, in bone order.
    pub fn joints(&self) -> &[JointPose] {
        self.pose.joints()
    }

    /// The joint of the bone at `bone_index`.
    pub fn joint(&self, bone_index: usize) -> Option<&JointPose> {
        self.pose.joint(bone_index)
    }

    /// The joint of the bone at `bone_index`, to change.
    pub fn joint_mut(&mut self, bone_index: usize) -> Option<&mut JointPose> {
        self.pose.joint_mut(bone_index)
    }

    /// The clip events this frame carries, each weighted by its source.
    pub fn events(&self) -> &[FiredEvent] {
        &self.events
    }

    /// Adds a clip event to this frame.
    pub fn push_event(&mut self, event: FiredEvent) {
        self.events.push(event);
    }

    /// Blends the pose and attributes toward `other` by `weight`, and weights both frames' events to match.
    pub fn blend(&mut self, other: &AnimationFrame, weight: f32) {
        self.pose.blend(&other.pose, weight);
        self.attributes.blend(&other.attributes, weight);
        for event in &mut self.events {
            event.weight *= 1.0 - weight;
        }
        self.events
            .extend(other.events.iter().map(|event| FiredEvent {
                weight: event.weight * weight,
                ..*event
            }));
    }

    /// Overwrites this frame's joints, attributes and events with those of `other` (same skeleton / length).
    pub fn copy_from(&mut self, other: &AnimationFrame) {
        self.pose.copy_from(&other.pose);
        self.attributes.copy_from(&other.attributes);
        self.events.clone_from(&other.events);
    }
}

impl Blendable for AnimationFrame {
    fn interpolate(_from: Self, _to: Self, _t: f32) -> Self {
        todo!()
    }
}

/// A frame an input of a node evaluated, and the weight it carries.
pub struct EvaluatedFrame {
    pub frame: AnimationFrame,
    pub weight: f32,
}

/// One input's root motion in the root-motion pass and the weight its node carries.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeightedMotion {
    pub motion: RootMotion,
    pub weight: f32,
}

/// Recycled frames, handed out at the skeleton's rest pose.
pub struct FramePool {
    free_frames: Vec<AnimationFrame>,
    bone_count: usize,
    rest: Box<[JointPose]>,
}

impl FramePool {
    pub(crate) fn new(bone_count: usize) -> Self {
        Self {
            free_frames: Vec::new(),
            bone_count,
            rest: Box::default(),
        }
    }

    pub(crate) fn acquire(&mut self) -> AnimationFrame {
        let mut frame = self
            .free_frames
            .pop()
            .unwrap_or_else(|| AnimationFrame::identity(self.bone_count));
        if !self.rest.is_empty() {
            frame.pose.joints.clone_from_slice(&self.rest);
        }
        frame.attributes.clear();
        frame.events.clear();
        frame
    }

    pub(crate) fn set_rest(&mut self, rest: &[JointPose]) {
        if rest.len() != self.bone_count {
            self.bone_count = rest.len();
            self.free_frames.clear();
        }
        if *self.rest != *rest {
            self.rest = rest.into();
        }
    }

    pub(crate) fn release(&mut self, frame: AnimationFrame) {
        self.free_frames.push(frame);
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::PI;

    use glam::{Quat, Vec3};
    use proptest::prelude::*;

    use super::{JointPose, RootMotion, twist_about};

    fn joint(translation: Vec3, rotation: Quat) -> JointPose {
        JointPose {
            translation,
            rotation,
            scale: Vec3::ONE,
        }
    }

    struct TwistCase {
        name: &'static str,
        rotation: Quat,
        axis: Vec3,
        twist: Quat,
    }

    #[test]
    fn twist_about_keeps_the_part_of_a_turn_about_the_axis() {
        let axis = Vec3::new(1.0, 2.0, 2.0).normalize();
        let twist = Quat::from_axis_angle(axis, 0.9);
        let swing = Quat::from_axis_angle(axis.any_orthonormal_vector(), 0.4);
        let cases = [
            TwistCase {
                name: "a mix gives the twist it starts with",
                rotation: twist * swing,
                axis,
                twist,
            },
            TwistCase {
                name: "a half turn about the axis is all twist",
                rotation: Quat::from_rotation_y(PI),
                axis: Vec3::Y,
                twist: Quat::from_rotation_y(PI),
            },
            TwistCase {
                name: "a half turn across the axis has no twist to find",
                rotation: Quat::from_xyzw(1.0, 0.0, 0.0, 0.0),
                axis: Vec3::Y,
                twist: Quat::IDENTITY,
            },
        ];

        for case in cases {
            let found = twist_about(case.rotation, case.axis);

            assert!(
                found.abs_diff_eq(case.twist, 1e-5),
                "{}: {found} != {}",
                case.name,
                case.twist
            );
        }
    }

    fn any_stop() -> impl Strategy<Value = JointPose> {
        prop::array::uniform4(-5.0f32..5.0).prop_map(|[x, y, z, yaw]| {
            joint(
                Vec3::new(x, y, z),
                Quat::from_rotation_y(yaw * 0.6) * Quat::from_rotation_x(-1.2),
            )
        })
    }

    proptest! {
        #[test]
        fn two_stretches_compose_to_the_whole_journey(
            start in any_stop(),
            middle in any_stop(),
            end in any_stop(),
        ) {
            let rest = Quat::from_rotation_x(-1.2);

            let composed = RootMotion::between(&start, &middle, rest)
                .then(RootMotion::between(&middle, &end, rest));
            let whole = RootMotion::between(&start, &end, rest);

            prop_assert!(
                composed.translation.abs_diff_eq(whole.translation, 1e-4),
                "{} != {}",
                composed.translation,
                whole.translation
            );
            prop_assert!(
                composed.rotation.abs_diff_eq(whole.rotation, 1e-4)
                    || composed.rotation.abs_diff_eq(-whole.rotation, 1e-4),
                "{} != {}",
                composed.rotation,
                whole.rotation
            );
        }
    }
}
