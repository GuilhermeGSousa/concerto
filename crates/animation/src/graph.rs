use std::{borrow::Cow, collections::HashMap, fmt, ops::Deref};

use concerto_foundation::assets::{
    Asset, LoadableAsset,
    asset_server::{AssetLoadContext, AssetServer},
    asset_store::AssetStore,
    handle::{AssetHandle, LiveLeaves},
};
use concerto_reflect::{EncodeError, ReflectedValue, TypeRegistry, ValueWarning, to_value};
use log::warn;
use petgraph::{
    Direction::Outgoing,
    graph::{DiGraph, Neighbors, NodeIndex},
    visit::{Dfs, DfsPostOrder, Walker},
};
use uuid::Uuid;

use crate::{
    blackboard::AnimationBlackboard,
    evaluation::{AnimationGraphContext, AnimationGraphEvaluator},
    node::{
        AnimationNode, ErasedAnimationNode, PlaceholderNode, ReflectAnimationNode, ResultNode,
        blend_space::BlendSpace2DBuilderContext,
    },
    player::ActiveNodeState,
    pose::{EvaluatedPose, Pose, PosePool},
};

type AnimationDirectedGraph = DiGraph<AnimationNodeData, ()>;

static PLACEHOLDER: PlaceholderNode = PlaceholderNode {};

/// One node as a graph holds it: `Stored` until built, `Live` once built, or a `Placeholder` that plays as [`PlaceholderNode`] and keeps the value it could not build.
pub enum AnimationNodeData {
    Stored {
        type_name: String,
        value: ReflectedValue,
    },
    Live {
        type_name: String,
        node: Box<dyn ErasedAnimationNode>,
    },
    Placeholder {
        type_name: String,
        value: ReflectedValue,
    },
}

impl AnimationNodeData {
    pub(crate) fn of<T: AnimationNode>(node: &T) -> Self {
        Self::Stored {
            type_name: std::any::type_name::<T>().to_owned(),
            value: to_value(node).expect("an animation node encodes through reflection"),
        }
    }

    /// The registry key of the node's type.
    pub fn type_name(&self) -> &str {
        match self {
            Self::Stored { type_name, .. }
            | Self::Live { type_name, .. }
            | Self::Placeholder { type_name, .. } => type_name,
        }
    }

    /// The reflected value a save writes.
    pub fn value(&self) -> Result<Cow<'_, ReflectedValue>, EncodeError> {
        match self {
            Self::Stored { value, .. } | Self::Placeholder { value, .. } => {
                Ok(Cow::Borrowed(value))
            }
            Self::Live { node, .. } => to_value(node.as_reflect()).map(Cow::Owned),
        }
    }

    /// The node that plays: the live one, the placeholder, or none before the graph is made live.
    pub fn live(&self) -> Option<&dyn ErasedAnimationNode> {
        match self {
            Self::Stored { .. } => None,
            Self::Live { node, .. } => Some(node.as_ref()),
            Self::Placeholder { .. } => Some(&PLACEHOLDER),
        }
    }
}

#[derive(serde::Serialize)]
struct StoredRef<'a> {
    type_name: &'a str,
    value: Cow<'a, ReflectedValue>,
}

#[derive(serde::Deserialize)]
struct Stored {
    type_name: String,
    value: ReflectedValue,
}

impl serde::Serialize for AnimationNodeData {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        StoredRef {
            type_name: self.type_name(),
            value: self.value().map_err(serde::ser::Error::custom)?,
        }
        .serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for AnimationNodeData {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let Stored { type_name, value } = Stored::deserialize(deserializer)?;
        Ok(Self::Stored { type_name, value })
    }
}

impl fmt::Debug for AnimationNodeData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let variant = match self {
            Self::Stored { .. } => "Stored",
            Self::Live { .. } => "Live",
            Self::Placeholder { .. } => "Placeholder",
        };
        f.debug_struct(variant)
            .field("type_name", &self.type_name())
            .field("value", &self.value())
            .finish()
    }
}

impl PartialEq for AnimationNodeData {
    fn eq(&self, other: &Self) -> bool {
        self.type_name() == other.type_name()
            && matches!((self.value(), other.value()), (Ok(mine), Ok(theirs)) if mine == theirs)
    }
}

#[derive(Clone, Copy, Eq, Hash, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AnimationNodeIndex(NodeIndex);

impl Deref for AnimationNodeIndex {
    type Target = NodeIndex;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<NodeIndex> for AnimationNodeIndex {
    fn from(value: NodeIndex) -> Self {
        AnimationNodeIndex(value)
    }
}

#[derive(Asset, serde::Serialize, serde::Deserialize)]
pub struct AnimationGraph {
    nodes: AnimationDirectedGraph,
    result_node: AnimationNodeIndex,
    #[serde(skip)]
    load_warnings: Vec<ValueWarning>,
}

impl LoadableAsset for AnimationGraph {
    fn on_load(&mut self, context: &AssetLoadContext) -> anyhow::Result<()> {
        let registry = context.type_registry().read();
        self.make_live(&registry, Some(context.asset_server()))?;
        for warning in &self.load_warnings {
            warn!("animation graph {:?}: {warning}", context.asset_id());
        }
        Ok(())
    }
}

impl AnimationGraph {
    pub fn new() -> Self {
        let mut nodes = AnimationDirectedGraph::new();
        let result_node = nodes.add_node(AnimationNodeData::of(&ResultNode {}));

        Self {
            nodes,
            result_node: result_node.into(),
            load_warnings: Vec::new(),
        }
    }

    /// Builds every node not yet live from its stored value, loading handles through `server`; a node that cannot be built becomes a placeholder and leaves a warning.
    pub fn make_live(
        &mut self,
        registry: &TypeRegistry,
        server: Option<&AssetServer>,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.nodes.node_weight(*self.result_node).is_some(),
            "animation graph result node {} is not a node of the graph",
            self.result_node.index()
        );

        let mut leaves = LiveLeaves::from_parts(registry, server);
        let mut warnings = Vec::new();

        for (index, node) in self.nodes.node_weights_mut().enumerate() {
            let (AnimationNodeData::Stored { type_name, value }
            | AnimationNodeData::Placeholder { type_name, value }) = node
            else {
                continue;
            };
            let failure = match registry
                .get_by_name(type_name)
                .and_then(|registration| registration.data::<ReflectAnimationNode>().copied())
            {
                Some(data) => {
                    let (built, node_warnings) = (data.from_value)(value, &mut leaves);
                    if node_warnings.is_empty() {
                        *node = AnimationNodeData::Live {
                            type_name: std::mem::take(type_name),
                            node: built,
                        };
                        continue;
                    }
                    node_warnings
                        .into_iter()
                        .map(|warning| format!("node {index} replaced by a placeholder: {warning}"))
                        .collect()
                }
                None => vec![format!(
                    "node {index} replaced by a placeholder: type is not registered as an animation node"
                )],
            };
            warnings.extend(failure.into_iter().map(|reason| ValueWarning {
                path: type_name.clone(),
                reason,
            }));
            *node = AnimationNodeData::Placeholder {
                type_name: std::mem::take(type_name),
                value: std::mem::replace(value, ReflectedValue::List(Vec::new())),
            };
        }

        self.load_warnings = warnings;
        Ok(())
    }

    /// Whether every node is live or a placeholder; the player skips a graph that is not.
    pub fn is_live(&self) -> bool {
        self.nodes.node_weights().all(|node| node.live().is_some())
    }

    /// Every node, in node order.
    pub fn nodes(&self) -> impl Iterator<Item = &AnimationNodeData> {
        self.nodes.node_weights()
    }

    /// The nodes that play, in node order; a node not yet made live is left out.
    pub fn live_nodes(&self) -> impl Iterator<Item = &dyn ErasedAnimationNode> {
        self.nodes
            .node_weights()
            .filter_map(AnimationNodeData::live)
    }

    /// What the last [`AnimationGraph::make_live`] could not apply.
    pub fn load_warnings(&self) -> &[ValueWarning] {
        &self.load_warnings
    }

    pub fn from_node<T: AnimationNode>(node: T) -> Self {
        let mut graph = Self::new();
        let result_node_index = graph.result_node().index();
        graph.add_node(node, result_node_index);
        graph
    }

    pub fn add_node<T: AnimationNode>(
        &mut self,
        node: T,
        output_node: AnimationNodeIndex,
    ) -> AnimationNodeContext<'_> {
        self.add_stored_node(AnimationNodeData::of(&node), output_node)
    }

    pub(crate) fn add_stored_node(
        &mut self,
        node: AnimationNodeData,
        output_node: AnimationNodeIndex,
    ) -> AnimationNodeContext<'_> {
        let added_node = self.nodes.add_node(node);
        self.nodes.add_edge(*output_node, added_node, ());

        AnimationNodeContext {
            graph: self,
            node_index: added_node.into(),
        }
    }

    pub fn result_node(&mut self) -> AnimationNodeContext<'_> {
        let result_node = self.result_node;
        AnimationNodeContext {
            graph: self,
            node_index: result_node,
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = AnimationNodeIndex> + '_ {
        Dfs::new(&self.nodes, *self.result_node)
            .iter(&self.nodes)
            .map(|node_index| node_index.into())
    }

    pub fn iter_post_order(&self) -> impl Iterator<Item = AnimationNodeIndex> + '_ {
        DfsPostOrder::new(&self.nodes, *self.result_node)
            .iter(&self.nodes)
            .map(|node_index| node_index.into())
    }

    pub fn live_node(&self, node_index: AnimationNodeIndex) -> Option<&dyn ErasedAnimationNode> {
        self.nodes
            .node_weight(*node_index)
            .and_then(AnimationNodeData::live)
    }

    pub fn get_node_inputs(&self, node_index: AnimationNodeIndex) -> Neighbors<'_, (), u32> {
        self.nodes.neighbors_directed(*node_index, Outgoing)
    }
}

impl Default for AnimationGraph {
    fn default() -> Self {
        Self::new()
    }
}

pub struct AnimationNodeContext<'a> {
    pub(crate) graph: &'a mut AnimationGraph,
    pub(crate) node_index: AnimationNodeIndex,
}

impl<'a> AnimationNodeContext<'a> {
    pub fn index(&self) -> AnimationNodeIndex {
        self.node_index
    }

    pub fn with_input<T: AnimationNode>(
        &mut self,
        node: T,
        f: impl FnOnce(AnimationNodeContext<'_>),
    ) -> &mut Self {
        f(self.graph.add_node(node, self.node_index));
        self
    }

    pub fn with_blend_space_2d_input(
        &mut self,
        param: &str,
        f: impl FnOnce(&mut BlendSpace2DBuilderContext<'_>),
    ) -> &mut Self {
        let mut builder_context = BlendSpace2DBuilderContext {
            graph: self.graph,
            output_node_index: self.node_index,
            points: Vec::new(),
            nodes: Vec::new(),
            input: crate::node::blend_space::BlendInput {
                param: param.to_string(),
            },
        };

        f(&mut builder_context);

        builder_context.build();

        self
    }
}

#[derive(Default)]
pub(crate) struct AnimationGraphState {
    graph_handle: Option<AssetHandle<AnimationGraph>>,
    graph_state: HashMap<AnimationNodeIndex, ActiveNodeState>,
}

impl AnimationGraphState {
    pub(crate) fn set_node_weight(&mut self, node_index: &AnimationNodeIndex, weight: f32) {
        if let Some(active_anim) = self.graph_state.get_mut(node_index) {
            active_anim.weight = weight;
        }
    }

    pub(crate) fn get_active_node_state_mut(
        &mut self,
        node_index: &AnimationNodeIndex,
    ) -> Option<&mut ActiveNodeState> {
        self.graph_state.get_mut(node_index)
    }

    pub(crate) fn get_state<T: 'static>(&self, node_index: &AnimationNodeIndex) -> Option<&T> {
        self.graph_state
            .get(node_index)
            .and_then(|node_state| node_state.node_state.downcast_ref::<T>())
    }

    pub(crate) fn get_state_mut<T: 'static>(
        &mut self,
        node_index: &AnimationNodeIndex,
    ) -> Option<&mut T> {
        self.graph_state
            .get_mut(node_index)
            .and_then(|node_state| node_state.node_state.downcast_mut::<T>())
    }

    pub(crate) fn for_each_state_mut<N: AnimationNode>(
        &mut self,
        graphs: &AssetStore<AnimationGraph>,
        blackboard: &AnimationBlackboard,
        mut f: impl FnMut(&N, &mut N::State, &AnimationBlackboard),
    ) {
        let Some(graph) = self
            .graph_handle
            .as_ref()
            .and_then(|handle| graphs.get(handle))
        else {
            return;
        };

        for (node_index, node_state) in self.graph_state.iter_mut() {
            let Some(node) = graph
                .live_node(*node_index)
                .and_then(|node| node.as_reflect().downcast_ref::<N>())
            else {
                continue;
            };
            let Some(state) = node_state.node_state.downcast_mut::<N::State>() else {
                continue;
            };
            f(node, state, blackboard);
        }
    }

    pub(crate) fn initialize(
        &mut self,
        animation_graph: AssetHandle<AnimationGraph>,
        graphs: &AssetStore<AnimationGraph>,
        creation_context: &AnimationGraphContext,
    ) {
        self.graph_state.clear();
        let Some(graph) = graphs.get(&animation_graph) else {
            self.graph_handle = None;
            return;
        };

        if !graph.is_live() {
            self.graph_handle = None;
            return;
        }

        self.graph_handle = Some(animation_graph);

        for node_index in graph.iter() {
            let Some(anim_node) = graph.live_node(node_index) else {
                continue;
            };

            let Some(node_state) = anim_node.create_state(creation_context) else {
                continue;
            };
            self.graph_state.insert(
                node_index,
                ActiveNodeState {
                    node_state,
                    weight: 1.0,
                },
            );
        }
    }

    pub(crate) fn reset(&mut self, graphs: &AssetStore<AnimationGraph>) {
        let Some(graph) = self.get_animation_graph(graphs) else {
            return;
        };

        self.graph_state
            .iter_mut()
            .for_each(|(node_index, node_state)| {
                if let Some(node) = graph.live_node(*node_index) {
                    node.reset(&mut node_state.node_state);
                }
            });
    }

    pub(crate) fn update(
        &mut self,
        delta_time: f32,
        graphs: &AssetStore<AnimationGraph>,
        context: &AnimationGraphContext<'_>,
    ) {
        let Some(graph) = self.get_animation_graph(graphs) else {
            return;
        };

        self.graph_state
            .iter_mut()
            .for_each(|(node_index, node_state)| {
                let Some(node) = graph.live_node(*node_index) else {
                    return;
                };

                node_state.update(node, delta_time, context);
            });
    }

    pub(crate) fn evaluate(
        &mut self,
        graphs: &AssetStore<AnimationGraph>,
        context: &AnimationGraphContext<'_>,
        bone_ids: &[Uuid],
        pool: &mut PosePool,
        output_pose: &mut Pose,
    ) -> bool {
        let Some(graph) = self.get_animation_graph(graphs) else {
            return false;
        };

        let mut graph_evaluator = AnimationGraphEvaluator::new();

        for node_index in graph.iter_post_order() {
            let Some(node) = graph.live_node(node_index) else {
                continue;
            };

            let Some(node_state) = self.get_active_node_state_mut(&node_index) else {
                warn!(
                    "No node state found for node, make sure the animation player has been correctly initialized"
                );
                continue;
            };

            let input_count = graph.get_node_inputs(node_index).count();

            let stack_start = graph_evaluator.stack_len() - input_count;

            let mut node_output_pose = pool.acquire();

            node.evaluate(
                &mut node_state.node_state,
                context,
                bone_ids,
                graph_evaluator.view_stack(stack_start),
                pool,
                &mut node_output_pose,
            );

            // Consume inputs and add them back to the free list
            for evaluated_pose in graph_evaluator.evaluation_stack.drain(stack_start..) {
                pool.release(evaluated_pose.pose);
            }

            graph_evaluator.push_evaluation(EvaluatedPose {
                pose: node_output_pose,
                weight: node_state.weight,
            });
        }

        let mut result = graph_evaluator
            .pop_evaluation()
            .map(|evaluated_pose| evaluated_pose.pose)
            .unwrap_or_else(|| pool.acquire());

        std::mem::swap(output_pose, &mut result);
        pool.release(result);
        true
    }

    pub(crate) fn get_animation_graph<'a>(
        &self,
        graphs: &'a AssetStore<AnimationGraph>,
    ) -> Option<&'a AnimationGraph> {
        self.graph_handle
            .as_ref()
            .and_then(move |handle| graphs.get(handle))
    }

    pub(crate) fn is_finished(&self, graphs: &AssetStore<AnimationGraph>) -> bool {
        let Some(graph) = self.get_animation_graph(graphs) else {
            return true;
        };

        self.graph_state.iter().all(|(node_index, node_state)| {
            graph
                .live_node(*node_index)
                .is_none_or(|node| node.is_finished(&node_state.node_state))
        })
    }
}

#[derive(Clone, Copy)]
pub(crate) struct GraphId(usize);

impl Deref for GraphId {
    type Target = usize;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<usize> for GraphId {
    fn from(value: usize) -> Self {
        Self(value)
    }
}

pub(crate) struct AnimationGraphStates {
    states: Vec<AnimationGraphState>,
}

impl AnimationGraphStates {
    pub(crate) fn new(states: Vec<AnimationGraphState>) -> Self {
        Self { states }
    }

    pub(crate) fn get(&self, id: GraphId) -> Option<&AnimationGraphState> {
        self.states.get(*id)
    }

    pub(crate) fn get_mut(&mut self, id: GraphId) -> Option<&mut AnimationGraphState> {
        self.states.get_mut(*id)
    }
}

#[cfg(test)]
pub(crate) fn made_live(mut graph: AnimationGraph) -> AnimationGraph {
    let mut registry = TypeRegistry::default();
    crate::plugin::register_animation_types(&mut registry);
    graph.make_live(&registry, None).unwrap();
    graph
}
