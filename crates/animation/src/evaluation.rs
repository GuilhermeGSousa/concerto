use concerto_ecs::system::input::Inputs;
use concerto_mesh::Skeleton;
use glam::{Affine3A, Vec3};

use crate::{
    blackboard::AnimationBlackboard,
    pose::{EvaluatedPose, JointPose},
};

pub struct AnimationGraphEvaluator {
    pub(crate) evaluation_stack: Vec<EvaluatedPose>,
}

impl AnimationGraphEvaluator {
    pub fn new() -> Self {
        Self {
            evaluation_stack: Vec::new(),
        }
    }

    pub fn push_evaluation(&mut self, evaluated_pose: EvaluatedPose) {
        self.evaluation_stack.push(evaluated_pose);
    }

    pub fn pop_evaluation(&mut self) -> Option<EvaluatedPose> {
        self.evaluation_stack.pop()
    }

    pub fn stack_len(&self) -> usize {
        self.evaluation_stack.len()
    }

    pub fn view_stack(&self, start: usize) -> &[EvaluatedPose] {
        &self.evaluation_stack[start..]
    }
}

impl Default for AnimationGraphEvaluator {
    fn default() -> Self {
        Self::new()
    }
}

/// The bone root motion is read from: its index in the pose, its rest joint and the unit up of its parent's space.
#[derive(Clone, Debug, PartialEq)]
pub struct RootBone {
    pub index: usize,
    pub rest: JointPose,
    pub up: Vec3,
}

/// What every node of one player's graph is walked with: the player's own data and the walk's node inputs.
pub struct AnimationGraphContext<'a> {
    pub(crate) blackboard: &'a AnimationBlackboard,
    pub(crate) root: Option<RootBone>,
    pub(crate) skeleton: Option<&'a Skeleton>,
    pub(crate) model_to_world: Option<Affine3A>,
    pub(crate) inputs: &'a Inputs<'a, 'a>,
}

impl<'a> AnimationGraphContext<'a> {
    pub fn blackboard(&self) -> &AnimationBlackboard {
        self.blackboard
    }

    /// The root bone of the player being evaluated, if it has one.
    pub fn root(&self) -> Option<&RootBone> {
        self.root.as_ref()
    }

    /// The skeleton of the player being walked, once its asset is loaded.
    pub fn skeleton(&self) -> Option<&'a Skeleton> {
        self.skeleton
    }

    /// In `evaluate`, the world transform of the skeleton's model space: the `GlobalTransform` of bone 0's parent, or identity when it has none.
    pub fn model_to_world(&self) -> Option<Affine3A> {
        self.model_to_world
    }
}
