use concerto_foundation::assets::asset_store::AssetStore;
use uuid::Uuid;

use crate::{
    evaluation::AnimationGraphContext,
    graph::{AnimationGraph, AnimationGraphStates, GraphId},
    inertialization::{Inertializer, PoseVelocity},
    pose::{Pose, PosePool},
    transition::{AnimationTransitionBlender, blend_stack::BlendStack},
};

pub(crate) struct InertializationBlender {
    stack: BlendStack,
    inertializer: Option<Inertializer>,
    delta_time: f32,
    pending_halflife: Option<f32>,
    halflife: f32,
    last_output: Option<Pose>,
    previous_output: Option<Pose>,
    last_delta_time: f32,
}

impl InertializationBlender {
    pub(crate) fn new(initial_graph: GraphId) -> Self {
        Self {
            stack: BlendStack::new(initial_graph),
            inertializer: None,
            delta_time: 0.0,
            pending_halflife: None,
            halflife: 0.0,
            last_output: None,
            previous_output: None,
            last_delta_time: 0.0,
        }
    }

    fn drop_other_skeleton(&mut self, bone_count: usize) {
        self.inertializer
            .take_if(|inertializer| inertializer.bone_count() != bone_count);
        self.last_output.take_if(|pose| pose.len() != bone_count);
        self.previous_output
            .take_if(|pose| pose.len() != bone_count);
    }

    fn source_velocity(&self, bone_count: usize) -> PoseVelocity {
        match (&self.previous_output, &self.last_output) {
            (Some(previous), Some(last)) if self.last_delta_time > 0.0 => {
                PoseVelocity::between(previous, last, self.last_delta_time)
            }
            _ => PoseVelocity::zero(bone_count),
        }
    }

    fn store_output(&mut self, output: &Pose, delta_time: f32) {
        if delta_time > 0.0 {
            std::mem::swap(&mut self.previous_output, &mut self.last_output);
            self.last_delta_time = delta_time;
        }
        let reusable = self
            .last_output
            .as_ref()
            .is_some_and(|last| last.len() == output.len());
        if !reusable {
            self.last_output = Some(Pose::identity(output.len()));
        }
        if let Some(last) = self.last_output.as_mut() {
            last.copy_from(output);
        }
    }
}

impl AnimationTransitionBlender for InertializationBlender {
    fn current(&self) -> GraphId {
        self.stack.current()
    }

    fn sample(
        &mut self,
        bone_ids: &[Uuid],
        graph_states: &mut AnimationGraphStates,
        graphs: &AssetStore<AnimationGraph>,
        context: &AnimationGraphContext<'_>,
        pool: &mut PosePool,
        output: &mut Pose,
    ) {
        self.stack
            .sample(bone_ids, graph_states, graphs, context, pool, output);

        let bone_count = output.len();
        self.drop_other_skeleton(bone_count);
        let delta_time = std::mem::take(&mut self.delta_time);
        let zero_velocity = PoseVelocity::zero(bone_count);

        let started = match self.pending_halflife.take() {
            Some(halflife) if self.last_output.is_some() => {
                let source_velocity = self.source_velocity(bone_count);
                let source = self
                    .last_output
                    .as_ref()
                    .expect("last output checked above");
                self.inertializer
                    .get_or_insert_with(|| Inertializer::new(bone_count))
                    .transition(source, &source_velocity, output, &zero_velocity);
                self.halflife = halflife;
                true
            }
            _ => false,
        };

        let active = self
            .inertializer
            .as_mut()
            .filter(|inertializer| started || !inertializer.is_settled());
        if let Some(inertializer) = active {
            let mut destination = pool.acquire();
            destination.copy_from(output);
            let step = if started { 0.0 } else { delta_time };
            inertializer.update(&destination, &zero_velocity, self.halflife, step, output);
            pool.release(destination);
        }

        self.store_output(output, delta_time);
    }

    fn update(
        &mut self,
        delta_time: f32,
        graph_states: &mut AnimationGraphStates,
        graphs: &AssetStore<AnimationGraph>,
        context: &AnimationGraphContext<'_>,
    ) {
        self.delta_time += delta_time;
        self.stack.update(delta_time, graph_states, graphs, context);
    }

    fn transition(
        &mut self,
        next_graph: GraphId,
        graph_states: &mut AnimationGraphStates,
        blend_time: f32,
        graphs: &AssetStore<AnimationGraph>,
    ) {
        if *self.stack.current() == *next_graph {
            return;
        }

        self.stack.transition(next_graph, graph_states, 1.0, graphs);
        self.stack.collapse();
        self.pending_halflife = Some(blend_time);
    }
}
