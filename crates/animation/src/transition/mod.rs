use uuid::Uuid;

use crate::{
    evaluation::AnimationGraphContext,
    graph::{AnimationGraphInstances, GraphId},
    pose::{Pose, PosePool},
};

pub(crate) mod blend_stack;
pub(crate) mod inertialization_blender;

/// How a state machine moves between states: `blend_time` is the cross-fade duration or the inertialization halflife.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TransitionBlender {
    #[default]
    CrossFade,
    Inertialization,
}

pub(crate) trait AnimationTransitionBlender: Send + Sync {
    fn current(&self) -> GraphId;

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
        blend_time: f32,
        context: &AnimationGraphContext<'_>,
    );
}
