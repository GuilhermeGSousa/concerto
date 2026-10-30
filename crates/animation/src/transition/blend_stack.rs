use uuid::Uuid;

use crate::{
    evaluation::AnimationGraphContext,
    graph::{AnimationGraphInstances, GraphId},
    pose::{Pose, PosePool},
    transition::AnimationTransitionBlender,
};

/// One active graph in the cross-fade. The stack is ordered oldest → newest; only the newest
/// entry (the state being transitioned into) fades in.
struct BlendStackEntry {
    graph_id: GraphId,
    fade_speed: f32,
    weight: f32,
}

pub(crate) struct BlendStack {
    entries: Vec<BlendStackEntry>,
}

impl BlendStack {
    pub(crate) fn new(initial_graph: GraphId) -> Self {
        Self {
            entries: vec![BlendStackEntry {
                graph_id: initial_graph,
                fade_speed: 0.0,
                weight: 1.0,
            }],
        }
    }

    pub(crate) fn collapse(&mut self) {
        if let Some(top) = self.entries.last_mut() {
            top.weight = 1.0;
        }
        let last = self.entries.len().saturating_sub(1);
        self.entries.drain(..last);
    }
}

impl AnimationTransitionBlender for BlendStack {
    /// The state currently being transitioned into — the single source of truth the FSM reads.
    fn current(&self) -> GraphId {
        self.entries
            .last()
            .expect("blend stack always retains the current graph")
            .graph_id
    }

    fn sample(
        &mut self,
        bone_ids: &[Uuid],
        graph_instances: &mut AnimationGraphInstances,
        context: &AnimationGraphContext<'_>,
        pool: &mut PosePool,
        output: &mut Pose,
    ) {
        let Some((base, fading_in)) = self.entries.split_first() else {
            return;
        };

        // The settled base pose goes straight into the output...
        if let Some(graph_instance) = graph_instances.get_mut(base.graph_id) {
            graph_instance.evaluate(context, bone_ids, pool, output);
        }

        // ...then each newer graph is cross-faded on top by its current weight.
        for entry in fading_in {
            let mut layer_pose = pool.acquire();

            if let Some(graph_instance) = graph_instances.get_mut(entry.graph_id) {
                graph_instance.evaluate(context, bone_ids, pool, &mut layer_pose);
            }

            output.blend(&layer_pose, entry.weight);
            pool.release(layer_pose);
        }
    }

    fn update(
        &mut self,
        delta_time: f32,
        graph_instances: &mut AnimationGraphInstances,
        context: &AnimationGraphContext<'_>,
    ) {
        for entry in &self.entries {
            if let Some(graph_instance) = graph_instances.get_mut(entry.graph_id) {
                graph_instance.update(delta_time, context);
            }
        }

        let Some(top) = self.entries.last_mut() else {
            return;
        };
        top.weight = (top.weight + top.fade_speed * delta_time).min(1.0);

        if top.weight >= 1.0 && self.entries.len() > 1 {
            let last = self.entries.len() - 1;
            self.entries.drain(..last);
        }
    }

    fn transition(
        &mut self,
        next_graph: GraphId,
        graph_instances: &mut AnimationGraphInstances,
        transition_time: f32,
        context: &AnimationGraphContext<'_>,
    ) {
        if *self.current() == *next_graph {
            return;
        }

        // Re-entering a state still fading lower in the stack: drop the stale occurrence so its
        // single instance is not advanced twice and the reset below is unambiguous (invariant 2).
        self.entries.retain(|entry| *entry.graph_id != *next_graph);

        // Anchor the target's clip (and OnAnimationEnd) to the moment of entry (invariant 3).
        if let Some(graph_instance) = graph_instances.get_mut(next_graph) {
            graph_instance.reset(context);
        }

        self.entries.push(BlendStackEntry {
            graph_id: next_graph,
            fade_speed: 1.0 / transition_time,
            weight: 0.0,
        });
    }
}
