use uuid::Uuid;

use crate::{
    evaluation::AnimationGraphContext,
    graph::{AnimationGraphInstances, GraphId},
    pose::{Pose, PosePool},
};

pub(crate) mod blend_stack;
pub(crate) mod inertialization_blender;

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
        transition_time: f32,
        context: &AnimationGraphContext<'_>,
    );
}
