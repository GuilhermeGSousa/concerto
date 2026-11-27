use concerto_ecs::system::input::Inputs;
use concerto_mesh::Skeleton;
use glam::{Affine3A, Vec3};

use crate::{
    blackboard::AnimationBlackboard,
    pose::{EvaluatedFrame, JointPose},
};

/// The stack of frames a graph evaluation passes from each node to the node that consumes it.
///
/// Nodes are evaluated in post order, so a node's inputs are the top entries of the stack, first
/// input first.
pub struct AnimationGraphEvaluator {
    pub(crate) evaluation_stack: Vec<EvaluatedFrame>,
}

impl AnimationGraphEvaluator {
    /// An empty stack.
    pub fn new() -> Self {
        Self {
            evaluation_stack: Vec::new(),
        }
    }

    /// Pushes a node's evaluated frame.
    pub fn push_evaluation(&mut self, evaluated_frame: EvaluatedFrame) {
        self.evaluation_stack.push(evaluated_frame);
    }

    /// Pops the frame on top, or `None` if the stack is empty.
    pub fn pop_evaluation(&mut self) -> Option<EvaluatedFrame> {
        self.evaluation_stack.pop()
    }

    /// The number of frames on the stack.
    pub fn stack_len(&self) -> usize {
        self.evaluation_stack.len()
    }

    /// The frames from index `start` to the top, oldest first.
    ///
    /// # Panics
    ///
    /// Panics if `start` is greater than [`Self::stack_len`].
    pub fn view_stack(&self, start: usize) -> &[EvaluatedFrame] {
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
    /// The bone's index in the skeleton's bone list, which is also its index in the pose.
    pub index: usize,
    /// The bone's rest pose, local to its parent.
    pub rest: JointPose,
    /// The unit up direction of the bone's parent space.
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
    /// The blackboard of the player being walked.
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

    /// In `evaluate`, the world transform of the skeleton's model space as of this frame.
    ///
    /// It is taken after the character moved by its root motion: the world transform of bone
    /// 0's parent, or identity when bone 0 has none.
    pub fn model_to_world(&self) -> Option<Affine3A> {
        self.model_to_world
    }
}
