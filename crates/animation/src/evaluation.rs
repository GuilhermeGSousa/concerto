use concerto_foundation::assets::asset_store::AssetStore;

use crate::{
    blackboard::AnimationBlackboard,
    clip::AnimationClip,
    graph::AnimationGraph,
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

/// The bone root motion is read from: its index in the pose and its rest joint.
#[derive(Clone, Debug, PartialEq)]
pub struct RootBone {
    pub index: usize,
    pub rest: JointPose,
}

pub struct AnimationGraphContext<'a> {
    pub(crate) animation_clips: &'a AssetStore<AnimationClip>,
    pub(crate) animation_graphs: &'a AssetStore<AnimationGraph>,
    pub(crate) blackboard: &'a AnimationBlackboard,
    pub(crate) root: Option<RootBone>,
}

impl<'a> AnimationGraphContext<'a> {
    pub fn animation_clips(&self) -> &AssetStore<AnimationClip> {
        self.animation_clips
    }

    pub fn animation_graphs(&self) -> &AssetStore<AnimationGraph> {
        self.animation_graphs
    }

    pub fn blackboard(&self) -> &AnimationBlackboard {
        self.blackboard
    }

    /// The root bone of the player being evaluated, if it has one.
    pub fn root(&self) -> Option<&RootBone> {
        self.root.as_ref()
    }
}
