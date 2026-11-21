use concerto_animation::clip::AnimationClip;
use concerto_foundation::transform::Transform;
use concerto_mesh::{JointPose, Skeleton};
use concerto_motion_matching::{
    BodyMotion, FrameContext, MotionDatabase, TrajectorySample, bake::model_pose,
};

/// A database's local pose of `frame`, sampled from its clips; `None` when they can't be sampled.
pub fn local_pose(
    database: &MotionDatabase,
    skeleton: &Skeleton,
    clips: &[&AnimationClip],
    frame: usize,
) -> Option<Vec<JointPose>> {
    database.pose(frame, skeleton, clips).ok()
}

/// A local pose with its root joints moved from the simulation bone's space to where `simulation` puts them.
pub fn placed_pose(
    skeleton: &Skeleton,
    simulation: &Transform,
    mut pose: Vec<JointPose>,
) -> Vec<JointPose> {
    for (joint, parent) in pose.iter_mut().zip(skeleton.parents.iter()) {
        if parent.is_none() {
            joint.translation = simulation.rotation * joint.translation + simulation.translation;
            joint.rotation = simulation.rotation * joint.rotation;
        }
    }
    pose
}

/// The owned pose, simulation transform and trajectory of a frame, which a `FrameContext` borrows.
pub struct BakedFrame {
    local_pose: Vec<JointPose>,
    model_pose: Vec<Transform>,
    simulation: Transform,
    trajectory: Vec<TrajectorySample>,
}

impl BakedFrame {
    /// `frame` of the database, as it was baked; `None` when its clips can't be sampled or it is not derived.
    pub fn new(
        database: &MotionDatabase,
        skeleton: &Skeleton,
        clips: &[&AnimationClip],
        frame: usize,
    ) -> Option<Self> {
        Some(Self::placed(
            skeleton,
            local_pose(database, skeleton, clips, frame)?,
            database.simulation_bone(frame)?,
            database.trajectory_samples(frame)?,
        ))
    }

    pub(crate) fn placed(
        skeleton: &Skeleton,
        local_pose: Vec<JointPose>,
        simulation: Transform,
        trajectory: Vec<TrajectorySample>,
    ) -> Self {
        let model_pose = model_pose(
            skeleton,
            &placed_pose(skeleton, &simulation, local_pose.clone()),
        );
        Self {
            local_pose,
            model_pose,
            simulation,
            trajectory,
        }
    }

    /// The context a channel previews this frame's features with.
    pub fn context<'a>(&'a self, skeleton: &'a Skeleton) -> FrameContext<'a> {
        FrameContext {
            skeleton,
            local_pose: &self.local_pose,
            model_pose: &self.model_pose,
            model_velocities: &[],
            simulation: self.simulation.clone(),
            body: BodyMotion::default(),
            trajectory: &self.trajectory,
        }
    }
}
