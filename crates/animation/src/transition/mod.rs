use uuid::Uuid;

use crate::{
    evaluation::AnimationGraphContext,
    graph::{AnimationGraphInstances, GraphId},
    pose::{Pose, PosePool},
};

pub(crate) mod blend_stack;
pub(crate) mod inertialization_blender;

/// How a state machine moves from one state to the next.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum TransitionBlend {
    CrossFade { transition_time: f32 },
    Inertialize { halflife: f32 },
}

impl TransitionBlend {
    /// A cross-fade lasting `transition_time` seconds.
    pub fn cross_fade(transition_time: f32) -> Self {
        Self::CrossFade { transition_time }
    }

    /// An inertialization whose offset halves every `halflife` seconds.
    pub fn inertialize(halflife: f32) -> Self {
        Self::Inertialize { halflife }
    }
}

impl Default for TransitionBlend {
    fn default() -> Self {
        Self::cross_fade(0.1)
    }
}

pub(crate) trait AnimationTransitionBlender {
    fn sample(
        &mut self,
        bone_ids: &[Uuid],
        graph_instances: &mut AnimationGraphInstances,
        context: &AnimationGraphContext<'_>,
        pool: &mut PosePool,
        output: &mut Pose,
    );

    fn update(
        &mut self,
        delta_time: f32,
        graph_instances: &mut AnimationGraphInstances,
        context: &AnimationGraphContext<'_>,
    );

    fn transition(
        &mut self,
        next_graph: GraphId,
        graph_instances: &mut AnimationGraphInstances,
        blend: &TransitionBlend,
        context: &AnimationGraphContext<'_>,
    );
}
