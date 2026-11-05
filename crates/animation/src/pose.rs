use concerto_foundation::{assets::AssetId, blend::Blendable};
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

/// The part of `rotation` that turns about the unit vector `axis`, or identity for a half turn across it.
pub fn twist_about(rotation: Quat, axis: Vec3) -> Quat {
    let along = axis * rotation.xyz().dot(axis);
    let twist = Quat::from_xyzw(along.x, along.y, along.z, rotation.w);
    if twist.length_squared() < f32::EPSILON {
        return Quat::IDENTITY;
    }
    twist.normalize()
}

/// A clip event crossed this frame: the key on its clip, and the weight its source has in the pose.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FiredEvent {
    pub clip: AssetId,
    pub key: u32,
    pub time: f32,
    pub weight: f32,
    pub mirrored: bool,
}

pub struct Pose {
    joints: Box<[JointPose]>,
    root_motion: RootMotion,
    pub(crate) events: Vec<FiredEvent>,
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
            events: Vec::new(),
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

    /// The clip events this pose carries, each weighted by its source.
    pub fn events(&self) -> &[FiredEvent] {
        &self.events
    }

    /// Adds a clip event to this pose.
    pub fn push_event(&mut self, event: FiredEvent) {
        self.events.push(event);
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
        for event in &mut self.events {
            event.weight *= 1.0 - weight;
        }
        self.events
            .extend(other.events.iter().map(|event| FiredEvent {
                weight: event.weight * weight,
                ..*event
            }));
    }

    /// Overwrites this pose's joints, root motion and events with those of `other` (same skeleton / length).
    pub fn copy_from(&mut self, other: &Pose) {
        for (joint, other_joint) in self.joints.iter_mut().zip(other.joints.iter()) {
            *joint = other_joint.clone();
        }
        self.root_motion = other.root_motion;
        self.events.clone_from(&other.events);
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
    rest: Box<[JointPose]>,
}

impl PosePool {
    pub(crate) fn new(bone_count: usize) -> Self {
        Self {
            free_poses: Vec::new(),
            bone_count,
            rest: Box::default(),
        }
    }

    pub(crate) fn acquire(&mut self) -> Pose {
        let mut pose = self
            .free_poses
            .pop()
            .unwrap_or_else(|| Pose::identity(self.bone_count));
        if !self.rest.is_empty() {
            pose.joints.clone_from_slice(&self.rest);
        }
        pose.root_motion = RootMotion::IDENTITY;
        pose.events.clear();
        pose
    }

    pub(crate) fn set_rest(&mut self, rest: &[JointPose]) {
        if rest.len() != self.bone_count {
            self.bone_count = rest.len();
            self.free_poses.clear();
        }
        if *self.rest != *rest {
            self.rest = rest.into();
        }
    }

    pub(crate) fn release(&mut self, pose: Pose) {
        self.free_poses.push(pose);
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::PI;

    use glam::{Quat, Vec3};

    use concerto_foundation::assets::AssetId;

    use super::{FiredEvent, JointPose, Pose, PosePool, RootMotion, twist_about};

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
    fn a_turn_about_the_axis_is_all_twist() {
        let yaw = Quat::from_rotation_y(0.7);

        assert!(twist_about(yaw, Vec3::Y).abs_diff_eq(yaw, 1e-5));
    }

    #[test]
    fn a_turn_square_to_the_axis_has_no_twist() {
        let pitch = Quat::from_rotation_x(0.7);

        assert!(twist_about(pitch, Vec3::Y).abs_diff_eq(Quat::IDENTITY, 1e-5));
    }

    #[test]
    fn a_mix_gives_the_twist_it_starts_with() {
        let axis = Vec3::new(1.0, 2.0, 2.0).normalize();
        let twist = Quat::from_axis_angle(axis, 0.9);
        let swing = Quat::from_axis_angle(axis.any_orthonormal_vector(), 0.4);

        assert!(twist_about(twist * swing, axis).abs_diff_eq(twist, 1e-5));
    }

    #[test]
    fn a_half_turn_about_the_axis_is_all_twist() {
        let half_turn = Quat::from_rotation_y(PI);

        assert!(twist_about(half_turn, Vec3::Y).abs_diff_eq(half_turn, 1e-5));
    }

    #[test]
    fn a_half_turn_across_the_axis_has_no_twist_to_find() {
        assert_eq!(
            twist_about(Quat::from_xyzw(1.0, 0.0, 0.0, 0.0), Vec3::Y),
            Quat::IDENTITY
        );
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

    fn fired(key: u32, weight: f32) -> FiredEvent {
        FiredEvent {
            clip: AssetId::new(),
            key,
            time: key as f32,
            weight,
            mirrored: false,
        }
    }

    fn weights(pose: &Pose) -> Vec<(u32, f32)> {
        pose.events()
            .iter()
            .map(|event| (event.key, event.weight))
            .collect()
    }

    #[test]
    fn copy_from_carries_the_events_and_drops_its_own() {
        let mut source = Pose::identity(1);
        source.push_event(fired(1, 1.0));
        let mut copy = Pose::identity(1);
        copy.push_event(fired(2, 1.0));

        copy.copy_from(&source);

        assert_eq!(copy.events(), source.events());
    }

    #[test]
    fn blend_scales_its_own_events_by_the_rest_and_appends_the_others_scaled_by_the_weight() {
        let mut pose = Pose::identity(1);
        pose.push_event(fired(1, 1.0));
        pose.push_event(fired(2, 0.5));
        let mut other = Pose::identity(1);
        other.push_event(fired(3, 1.0));

        pose.blend(&other, 0.25);

        assert_eq!(weights(&pose), [(1, 0.75), (2, 0.375), (3, 0.25)]);
    }

    #[test]
    fn acquire_clears_the_events_of_a_recycled_pose() {
        let mut pool = PosePool::new(1);
        let mut recycled = pool.acquire();
        recycled.push_event(fired(1, 1.0));
        pool.release(recycled);

        assert!(pool.acquire().events().is_empty());
    }

    #[test]
    fn set_rest_with_another_length_resizes_the_pool() {
        let mut pool = PosePool::new(1);
        let shorter = pool.acquire();
        pool.release(shorter);
        let rest = [
            joint(Vec3::X, Quat::IDENTITY),
            joint(Vec3::Y, Quat::from_rotation_z(0.3)),
        ];

        pool.set_rest(&rest);

        assert_eq!(pool.acquire().joints(), rest);
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
