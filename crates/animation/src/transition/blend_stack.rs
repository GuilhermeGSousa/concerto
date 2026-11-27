use concerto_foundation::assets::asset_store::AssetStore;
use uuid::Uuid;

use crate::{
    attribute::PoseAttributes,
    evaluation::{AnimationGraphContext, DriveContext, RaisedMessage},
    graph::{AnimationGraph, AnimationGraphStates, GraphId},
    pose::{AnimationFrame, FramePool, RootMotion},
    transition::{AnimationTransitionBlender, BlendInFlight},
};

/// One active graph in the cross-fade. The stack is ordered oldest → newest; only the newest
/// entry (the state being transitioned into) fades in.
struct BlendStackEntry {
    source: Source,
    fade_speed: f32,
    weight: f32,
}

#[derive(Clone, Copy)]
enum Source {
    Graph(GraphId),
    Snapshot(GraphId),
}

impl Source {
    fn graph_id(self) -> GraphId {
        match self {
            Source::Graph(graph_id) | Source::Snapshot(graph_id) => graph_id,
        }
    }

    fn playing(self) -> Option<GraphId> {
        match self {
            Source::Graph(graph_id) => Some(graph_id),
            Source::Snapshot(_) => None,
        }
    }
}

pub(crate) struct BlendStack {
    entries: Vec<BlendStackEntry>,
    last_pose: AnimationFrame,
    last_pose_blended: bool,
    last_drive: PoseAttributes,
    snapshot_pose: AnimationFrame,
    snapshot_drive: PoseAttributes,
}

impl BlendStack {
    pub(crate) fn new(initial_graph: GraphId) -> Self {
        Self {
            entries: vec![BlendStackEntry {
                source: Source::Graph(initial_graph),
                fade_speed: 0.0,
                weight: 1.0,
            }],
            last_pose: AnimationFrame::identity(0),
            last_pose_blended: false,
            last_drive: PoseAttributes::default(),
            snapshot_pose: AnimationFrame::identity(0),
            snapshot_drive: PoseAttributes::default(),
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
        pool: &mut FramePool,
        output: &mut AnimationFrame,
        mut play: impl FnMut(Source, f32, &mut FramePool, &mut AnimationFrame),
        blend: impl Fn(&mut AnimationFrame, &AnimationFrame, f32),
    ) {
        let Some((base, fading_in)) = self.entries.split_first() else {
            return;
        };

        play(base.source, self.share(0), pool, output);

        for (index, entry) in fading_in.iter().enumerate() {
            let mut layer = pool.acquire();
            play(entry.source, self.share(index + 1), pool, &mut layer);
            blend(output, &layer, entry.weight);
            pool.release(layer);
        }
    }

    fn share(&self, index: usize) -> f32 {
        let own = match index {
            0 => 1.0,
            _ => self.entries[index].weight,
        };
        self.entries[index + 1..]
            .iter()
            .fold(own, |share, later| share * (1.0 - later.weight))
    }

    fn holds(&self, graph: GraphId) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.source.playing().is_some_and(|held| *held == *graph))
    }

    fn freeze(&mut self) {
        let from = self.current();
        if self.snapshot_pose.len() != self.last_pose.len() {
            self.snapshot_pose = AnimationFrame::identity(self.last_pose.len());
        }
        self.snapshot_pose.copy_from(&self.last_pose);
        self.snapshot_pose.events.clear();
        self.snapshot_pose.attributes_mut().freeze();
        self.snapshot_drive.copy_from(&self.last_drive);
        self.snapshot_drive.freeze();
        self.entries.clear();
        self.entries.push(BlendStackEntry {
            source: Source::Snapshot(from),
            fade_speed: 0.0,
            weight: 1.0,
        });
    }
}

impl AnimationTransitionBlender for BlendStack {
    /// The state currently being transitioned into — the single source of truth the FSM reads.
    fn current(&self) -> GraphId {
        self.entries
            .last()
            .expect("blend stack always retains the current graph")
            .source
            .graph_id()
    }

    fn in_flight(&self) -> Option<BlendInFlight> {
        let [.., from, to] = &self.entries[..] else {
            return None;
        };
        Some(BlendInFlight {
            from: from.source.graph_id(),
            to: to.source.graph_id(),
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
        let snapshot = &self.snapshot_pose;
        self.mix(
            pool,
            output,
            |source, _, pool, frame| match source {
                Source::Graph(graph_id) => {
                    if let Some(graph_state) = graph_states.get_mut(graph_id) {
                        graph_state.evaluate(graphs, context, bone_ids, pool, frame);
                    }
                }
                Source::Snapshot(_) => frame.copy_from(snapshot),
            },
            |output, layer, weight| output.blend(layer, weight),
        );
        self.last_pose_blended = self.entries.len() > 1;
        if !self.last_pose_blended {
            return;
        }
        if self.last_pose.len() != output.len() {
            self.last_pose = AnimationFrame::identity(output.len());
        }
        self.last_pose.copy_from(output);
    }

    fn drive(
        &mut self,
        bone_ids: &[Uuid],
        graph_states: &mut AnimationGraphStates,
        graphs: &AssetStore<AnimationGraph>,
        context: &mut DriveContext<'_>,
        pool: &mut FramePool,
        output: &mut AnimationFrame,
    ) {
        let snapshot = &self.snapshot_drive;
        self.mix(
            pool,
            output,
            |source, share, pool, frame| match source {
                Source::Graph(graph_id) => {
                    if let Some(graph_state) = graph_states.get_mut(graph_id) {
                        let mut walk = context.scaled(share);
                        graph_state.drive(graphs, &mut walk, bone_ids, pool, frame);
                        context.pass_up(walk);
                    }
                }
                Source::Snapshot(_) => frame.attributes_mut().copy_from(snapshot),
            },
            |output, layer, weight| {
                output.attributes_mut().blend(layer.attributes(), weight);
            },
        );
        self.last_drive.copy_from(output.attributes());
    }

    fn root_motion(
        &mut self,
        bone_ids: &[Uuid],
        graph_states: &mut AnimationGraphStates,
        graphs: &AssetStore<AnimationGraph>,
        context: &AnimationGraphContext<'_>,
    ) -> RootMotion {
        let mut motion_of = |source: Source| {
            source
                .playing()
                .and_then(|graph_id| graph_states.get_mut(graph_id))
                .map_or(RootMotion::IDENTITY, |graph_state| {
                    graph_state.root_motion(graphs, context, bone_ids)
                })
        };
        let Some((base, fading_in)) = self.entries.split_first() else {
            return RootMotion::IDENTITY;
        };
        let mut motion = motion_of(base.source);
        for entry in fading_in {
            motion = motion.blend(motion_of(entry.source), entry.weight);
        }
        motion
    }

    fn update(
        &mut self,
        delta_time: f32,
        graph_states: &mut AnimationGraphStates,
        graphs: &AssetStore<AnimationGraph>,
        context: &mut AnimationGraphContext<'_>,
    ) -> Vec<RaisedMessage> {
        let newest = self.entries.len().saturating_sub(1);
        let mut raised = Vec::new();
        for (index, entry) in self.entries.iter().enumerate() {
            let Some(graph_state) = entry
                .source
                .playing()
                .and_then(|graph_id| graph_states.get_mut(graph_id))
            else {
                continue;
            };
            if index == newest {
                let mut walk = context.child();
                graph_state.update(delta_time, graphs, &mut walk);
                raised = walk.into_raised();
            } else {
                graph_state.update(delta_time, graphs, context);
            }
        }

        let Some(top) = self.entries.last_mut() else {
            return raised;
        };
        top.weight = (top.weight + top.fade_speed * delta_time).min(1.0);

        if top.weight >= 1.0 && self.entries.len() > 1 {
            let last = self.entries.len() - 1;
            self.entries.drain(..last);
        }
        raised
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

        if self.holds(next_graph) {
            if self.last_pose_blended {
                self.freeze();
            } else {
                self.entries.retain(|entry| {
                    entry
                        .source
                        .playing()
                        .is_none_or(|held| *held != *next_graph)
                });
            }
        }

        // Anchor the target's clip (and its ClipEnded) to the moment of entry (invariant 3).
        if let Some(graph_state) = graph_states.get_mut(next_graph) {
            graph_state.reset(graphs);
        }

        self.entries.push(BlendStackEntry {
            source: Source::Graph(next_graph),
            fade_speed: 1.0 / transition_time,
            weight: 0.0,
        });
    }
}
