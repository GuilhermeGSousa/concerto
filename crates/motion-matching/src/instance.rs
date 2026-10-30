use std::collections::HashMap;

use concerto_animation::{
    inertialization::{Inertializer, PoseVelocity},
    pose::Pose,
};
use glam::{Quat, Vec3};
use uuid::Uuid;

use crate::{channel::BodyMotion, query::FeatureQuery};

/// The per-player state of a motion matching node.
pub struct MotionMatchingInstance {
    pub initialised: bool,
    pub failed: bool,
    pub active: bool,
    pub delta_time: f32,
    pub frame: usize,
    pub sub_frame_time: f32,
    pub search_timer: f32,
    pub forced_search_timer: f32,
    pub previous_desired_velocity: Vec3,
    pub desired_velocity: Vec3,
    pub previous_desired_rotation: Quat,
    pub desired_rotation: Quat,
    pub velocity_change: f32,
    pub rotation_change: f32,
    pub pose: Pose,
    pub velocity: PoseVelocity,
    pub blended: Pose,
    pub previous_blended: Pose,
    pub last_delta: f32,
    pub inertializer: Option<Inertializer>,
    pub output: Pose,
    pub query: Option<FeatureQuery>,
    pub bone_indices: HashMap<Uuid, usize>,
    pub root_motion: BodyMotion,
}

impl MotionMatchingInstance {
    pub(crate) fn new(search_period: f32) -> Self {
        Self {
            initialised: false,
            failed: false,
            active: false,
            delta_time: 0.0,
            frame: 0,
            sub_frame_time: 0.0,
            search_timer: search_period,
            forced_search_timer: 0.0,
            previous_desired_velocity: Vec3::ZERO,
            desired_velocity: Vec3::ZERO,
            previous_desired_rotation: Quat::IDENTITY,
            desired_rotation: Quat::IDENTITY,
            velocity_change: 0.0,
            rotation_change: 0.0,
            pose: Pose::identity(0),
            velocity: PoseVelocity::zero(0),
            blended: Pose::identity(0),
            previous_blended: Pose::identity(0),
            last_delta: 0.0,
            inertializer: None,
            output: Pose::identity(0),
            query: None,
            bone_indices: HashMap::new(),
            root_motion: BodyMotion::default(),
        }
    }

    /// Marks the instance to be advanced by the runtime this frame by `delta_time` seconds.
    pub fn activate(&mut self, delta_time: f32) {
        self.active = true;
        self.delta_time = delta_time;
    }

    /// The simulation bone's velocity and angular velocity at the current frame.
    pub fn root_motion(&self) -> BodyMotion {
        self.root_motion
    }

    /// Copies the instance's pose into `output` and returns true, or returns false and leaves `output` untouched until the instance is initialised.
    pub fn write_output(&self, output: &mut Pose) -> bool {
        if !self.initialised || self.output.len() != output.len() {
            return false;
        }
        output.copy_from(&self.output);
        true
    }
}

#[cfg(test)]
mod tests {
    use concerto_animation::pose::Pose;
    use concerto_mesh::JointPose;
    use glam::Vec3;

    use super::MotionMatchingInstance;

    fn moved(x: f32) -> JointPose {
        JointPose {
            translation: Vec3::new(x, 0.0, 0.0),
            ..JointPose::IDENTITY
        }
    }

    #[test]
    fn an_uninitialised_instance_leaves_the_output_untouched() {
        let mut instance = MotionMatchingInstance::new(0.1);
        instance.output = Pose::from_joints(vec![moved(5.0)]);
        let mut output = Pose::from_joints(vec![moved(1.0)]);

        assert!(!instance.write_output(&mut output));
        assert_eq!(output.joints()[0].translation.x, 1.0);
    }

    #[test]
    fn an_initialised_instance_copies_its_pose_to_the_output() {
        let mut instance = MotionMatchingInstance::new(0.1);
        instance.output = Pose::from_joints(vec![moved(5.0)]);
        instance.initialised = true;
        let mut output = Pose::from_joints(vec![moved(1.0)]);

        assert!(instance.write_output(&mut output));
        assert_eq!(output.joints()[0].translation.x, 5.0);
    }

    #[test]
    fn a_new_instance_is_inactive_and_activate_records_the_delta_time() {
        let mut instance = MotionMatchingInstance::new(0.1);
        assert!(!instance.active);

        instance.activate(0.25);

        assert!(instance.active);
        assert_eq!(instance.delta_time, 0.25);
        assert_eq!(instance.root_motion().velocity, Vec3::ZERO);
    }
}
