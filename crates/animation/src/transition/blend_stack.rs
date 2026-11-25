use concerto_foundation::assets::asset_store::AssetStore;
use uuid::Uuid;

use crate::{
    evaluation::{AnimationGraphContext, DriveContext},
    graph::{AnimationGraph, AnimationGraphState, AnimationGraphStates, GraphId},
    pose::{AnimationFrame, FramePool, RootMotion},
    transition::{AnimationTransitionBlender, BlendInFlight},
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

    fn mix(
        &self,
        graph_states: &mut AnimationGraphStates,
        pool: &mut FramePool,
        output: &mut AnimationFrame,
        mut play: impl FnMut(&mut AnimationGraphState, &mut FramePool, &mut AnimationFrame),
        blend: impl Fn(&mut AnimationFrame, &AnimationFrame, f32),
    ) {
        let Some((base, fading_in)) = self.entries.split_first() else {
            return;
        };

        if let Some(graph_state) = graph_states.get_mut(base.graph_id) {
            play(graph_state, pool, output);
        }

        for entry in fading_in {
            let mut layer = pool.acquire();

            if let Some(graph_state) = graph_states.get_mut(entry.graph_id) {
                play(graph_state, pool, &mut layer);
            }

            blend(output, &layer, entry.weight);
            pool.release(layer);
        }
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

    fn for_each_playing(&self, visit: &mut dyn FnMut(GraphId)) {
        for entry in &self.entries {
            visit(entry.graph_id);
        }
    }

    fn in_flight(&self) -> Option<BlendInFlight> {
        let [.., from, to] = &self.entries[..] else {
            return None;
        };
        Some(BlendInFlight {
            from: from.graph_id,
            to: to.graph_id,
            progress: to.weight,
        })
    }

    fn sample(
        &mut self,
        bone_ids: &[Uuid],
        graph_states: &mut AnimationGraphStates,
        graphs: &AssetStore<AnimationGraph>,
        context: &AnimationGraphContext<'_>,
        pool: &mut FramePool,
        output: &mut AnimationFrame,
    ) {
        self.mix(
            graph_states,
            pool,
            output,
            |graph_state, pool, frame| {
                graph_state.evaluate(graphs, context, bone_ids, pool, frame);
            },
            |output, layer, weight| output.blend(layer, weight),
        );
    }

    fn drive(
        &self,
        bone_ids: &[Uuid],
        graph_states: &mut AnimationGraphStates,
        graphs: &AssetStore<AnimationGraph>,
        context: &DriveContext<'_>,
        pool: &mut FramePool,
        output: &mut AnimationFrame,
    ) {
        self.mix(
            graph_states,
            pool,
            output,
            |graph_state, pool, frame| {
                graph_state.drive(graphs, context, bone_ids, pool, frame);
            },
            |output, layer, weight| {
                output.attributes_mut().blend(layer.attributes(), weight);
            },
        );
    }

    fn root_motion(
        &mut self,
        bone_ids: &[Uuid],
        graph_states: &mut AnimationGraphStates,
        graphs: &AssetStore<AnimationGraph>,
        context: &AnimationGraphContext<'_>,
    ) -> RootMotion {
        let Some((base, fading_in)) = self.entries.split_first() else {
            return RootMotion::IDENTITY;
        };

        let mut motion = graph_states
            .get_mut(base.graph_id)
            .map_or(RootMotion::IDENTITY, |graph_state| {
                graph_state.root_motion(graphs, context, bone_ids)
            });

        for entry in fading_in {
            let layer = graph_states
                .get_mut(entry.graph_id)
                .map_or(RootMotion::IDENTITY, |graph_state| {
                    graph_state.root_motion(graphs, context, bone_ids)
                });
            motion = motion.blend(layer, entry.weight);
        }
        motion
    }

    fn update(
        &mut self,
        delta_time: f32,
        graph_states: &mut AnimationGraphStates,
        graphs: &AssetStore<AnimationGraph>,
        context: &AnimationGraphContext<'_>,
    ) {
        for entry in &self.entries {
            if let Some(graph_state) = graph_states.get_mut(entry.graph_id) {
                graph_state.update(delta_time, graphs, context);
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
        graph_states: &mut AnimationGraphStates,
        transition_time: f32,
        graphs: &AssetStore<AnimationGraph>,
    ) {
        if *self.current() == *next_graph {
            return;
        }

        // Re-entering a state still fading lower in the stack: drop the stale occurrence so its
        // single graph state is not advanced twice and the reset below is unambiguous (invariant 2).
        self.entries.retain(|entry| *entry.graph_id != *next_graph);

        // Anchor the target's clip (and OnAnimationEnd) to the moment of entry (invariant 3).
        if let Some(graph_state) = graph_states.get_mut(next_graph) {
            graph_state.reset(graphs);
        }

        self.entries.push(BlendStackEntry {
            graph_id: next_graph,
            fade_speed: 1.0 / transition_time,
            weight: 0.0,
        });
    }
}
