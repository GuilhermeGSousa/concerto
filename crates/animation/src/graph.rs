use std::{borrow::Cow, collections::HashMap, fmt, ops::Deref};

use concerto_foundation::assets::{
    Asset, CreatableAsset, LoadableAsset,
    asset_server::{AssetLoadContext, AssetServer},
    asset_store::AssetStore,
    handle::{AssetHandle, LiveLeaves},
};
use concerto_name::Name;
use concerto_reflect::{EncodeError, ReflectedValue, TypeRegistry, ValueWarning, to_value};
use glam::Vec2;
use log::warn;
use petgraph::{
    Direction::{Incoming, Outgoing},
    algo::has_path_connecting,
    graph::{DiGraph, Neighbors, NodeIndex},
    visit::{Dfs, DfsPostOrder, Walker},
};
use uuid::Uuid;

use crate::{
    blackboard::AnimationBlackboard,
    evaluation::{AnimationGraphContext, AnimationGraphEvaluator},
    node::{
        AnimationNode, ErasedAnimationNode, LayerNode, NodeReport, PlaceholderNode,
        ReflectAnimationNode, ResultNode, blend_space::BlendSpace2DBuilderContext,
    },
    param::Param,
    player::ActiveNodeState,
    pose::{AnimationFrame, EvaluatedFrame, FramePool},
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
    /// The stored data of a typed node.
    pub fn of<T: AnimationNode>(node: &T) -> Self {
        Self::Stored {
            type_name: std::any::type_name::<T>().to_owned(),
            value: to_value(node).expect("an animation node encodes through reflection"),
        }
    }

    /// The data of a node that is already built, which plays without [`AnimationGraph::make_live`].
    pub fn from_live(node: Box<dyn ErasedAnimationNode>) -> Self {
        Self::Live {
            type_name: node.as_reflect().type_name().to_owned(),
            node,
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

    /// The live node to edit in place; none for a stored node or a placeholder.
    pub fn live_mut(&mut self) -> Option<&mut dyn ErasedAnimationNode> {
        match self {
            Self::Live { node, .. } => Some(node.as_mut()),
            Self::Stored { .. } | Self::Placeholder { .. } => None,
        }
    }
}

#[derive(serde::Serialize)]
pub(crate) struct StoredRef<'a> {
    pub(crate) type_name: &'a str,
    pub(crate) value: Cow<'a, ReflectedValue>,
}

#[derive(serde::Deserialize)]
pub(crate) struct Stored {
    pub(crate) type_name: String,
    pub(crate) value: ReflectedValue,
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

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Serialize, serde::Deserialize)]
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

/// Why [`AnimationGraph::connect`] refused a connection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectError {
    NotANode { index: AnimationNodeIndex },
    ChildIsResult,
    ChildHasParent { child: AnimationNodeIndex },
    Cycle,
}

impl fmt::Display for ConnectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConnectError::NotANode { index } => {
                write!(f, "{} is not a node of the graph", index.index())
            }
            ConnectError::ChildIsResult => {
                write!(f, "the result node cannot be an input of another node")
            }
            ConnectError::ChildHasParent { child } => write!(
                f,
                "node {} is already an input of another node",
                child.index()
            ),
            ConnectError::Cycle => write!(f, "the connection would close a cycle"),
        }
    }
}

impl std::error::Error for ConnectError {}

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

impl CreatableAsset for AnimationGraph {
    fn new_asset() -> Self {
        Self::new()
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

    pub(crate) fn get_node_inputs(&self, node_index: AnimationNodeIndex) -> Neighbors<'_, (), u32> {
        self.nodes.neighbors_directed(*node_index, Outgoing)
    }

    /// Every node's index, in node order, whether or not the result node reaches it.
    pub fn node_indices(&self) -> impl Iterator<Item = AnimationNodeIndex> + '_ {
        self.nodes.node_indices().map(AnimationNodeIndex::from)
    }

    /// The node at `index`.
    pub fn node(&self, index: AnimationNodeIndex) -> Option<&AnimationNodeData> {
        self.nodes.node_weight(*index)
    }

    /// The node at `index`, to edit in place.
    pub fn node_mut(&mut self, index: AnimationNodeIndex) -> Option<&mut AnimationNodeData> {
        self.nodes.node_weight_mut(*index)
    }

    /// The index of the result node.
    pub fn result_index(&self) -> AnimationNodeIndex {
        self.result_node
    }

    /// The inputs of the node at `index` in slot order, the order they were added and the order the node evaluates them in.
    pub fn inputs(&self, index: AnimationNodeIndex) -> Vec<AnimationNodeIndex> {
        let mut inputs: Vec<AnimationNodeIndex> = self
            .get_node_inputs(index)
            .map(AnimationNodeIndex::from)
            .collect();
        inputs.reverse();
        inputs
    }

    /// Adds a node that is no node's input yet.
    pub fn insert_node(&mut self, data: AnimationNodeData) -> AnimationNodeIndex {
        self.nodes.add_node(data).into()
    }

    /// Appends `child` as the next input of `parent`.
    pub fn connect(
        &mut self,
        parent: AnimationNodeIndex,
        child: AnimationNodeIndex,
    ) -> Result<(), ConnectError> {
        for index in [parent, child] {
            if self.nodes.node_weight(*index).is_none() {
                return Err(ConnectError::NotANode { index });
            }
        }
        if child == self.result_node {
            return Err(ConnectError::ChildIsResult);
        }
        if self
            .nodes
            .neighbors_directed(*child, Incoming)
            .next()
            .is_some()
        {
            return Err(ConnectError::ChildHasParent { child });
        }
        if has_path_connecting(&self.nodes, *child, *parent, None) {
            return Err(ConnectError::Cycle);
        }
        self.nodes.add_edge(*parent, *child, ());
        Ok(())
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

    /// Adds `node` as an input and returns the input's context, so a chain of single inputs reads flat.
    pub fn input<T: AnimationNode>(self, node: T) -> AnimationNodeContext<'a> {
        self.graph.add_node(node, self.node_index)
    }

    /// Adds `node` as an input with no inputs of its own.
    pub fn add_input<T: AnimationNode>(&mut self, node: T) -> &mut Self {
        self.add_input_with(node, |_| {})
    }

    /// Adds `node` as an input and hands its context to `f`.
    pub fn add_input_with<T: AnimationNode>(
        &mut self,
        node: T,
        f: impl FnOnce(AnimationNodeContext<'_>),
    ) -> &mut Self {
        f(self.graph.add_node(node, self.node_index));
        self
    }

    /// Adds a 2D blend space driven by `param` as an input, its points and their inputs added through `f`.
    pub fn add_blend_space_2d_input_with(
        &mut self,
        param: impl Into<Param<Vec2>>,
        f: impl FnOnce(&mut BlendSpace2DBuilderContext<'_>),
    ) -> &mut Self {
        let mut builder_context = BlendSpace2DBuilderContext {
            graph: self.graph,
            output_node_index: self.node_index,
            points: Vec::new(),
            nodes: Vec::new(),
            input: crate::node::blend_space::BlendInput {
                param: param.into(),
            },
        };

        f(&mut builder_context);

        builder_context.build();

        self
    }

    /// Adds `node` as an input, with `base` and then `overlay` as its two inputs.
    pub fn add_layer_input<B: AnimationNode, O: AnimationNode>(
        &mut self,
        node: LayerNode,
        base: B,
        overlay: O,
    ) -> &mut Self {
        self.add_layer_input_with(
            node,
            |mut layer| {
                layer.add_input(base);
            },
            |mut layer| {
                layer.add_input(overlay);
            },
        )
    }

    /// Adds `node` as an input, its base through `base` and then its overlay through `overlay`; panics unless each closure adds exactly one input.
    pub fn add_layer_input_with(
        &mut self,
        node: LayerNode,
        base: impl FnOnce(AnimationNodeContext<'_>),
        overlay: impl FnOnce(AnimationNodeContext<'_>),
    ) -> &mut Self {
        let layer = self.graph.add_node(node, self.node_index).index();
        self.add_layer_side(layer, "base", base);
        self.add_layer_side(layer, "overlay", overlay);
        self
    }

    fn add_layer_side(
        &mut self,
        layer: AnimationNodeIndex,
        closure: &str,
        f: impl FnOnce(AnimationNodeContext<'_>),
    ) {
        let before = self.graph.get_node_inputs(layer).count();
        f(AnimationNodeContext {
            graph: self.graph,
            node_index: layer,
        });
        let added = self.graph.get_node_inputs(layer).count() - before;
        assert!(
            added == 1,
            "add_layer_input_with: the {closure} closure added {added} inputs to the layer node, and must add exactly one"
        );
    }
}

/// One step of the path to a playing node from a player's graph: a node of the graph the step is in and, when the next step is in a state's graph, that state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodePathStep {
    pub node: AnimationNodeIndex,
    pub state: Option<Name>,
}

/// One playing node as [`AnimationGraphState::for_each_node`] visits it; the last step of `path` is the node itself.
pub struct NodeVisit<'a> {
    pub path: &'a [NodePathStep],
    pub weight: f32,
    pub report: NodeReport,
}

/// The per-player state of one graph: its handle and its nodes' states.
#[derive(Default)]
pub struct AnimationGraphState {
    graph_handle: Option<AssetHandle<AnimationGraph>>,
    graph_state: HashMap<AnimationNodeIndex, ActiveNodeState>,
    warned_stateless: bool,
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

    pub(crate) fn reset_node(
        &mut self,
        node_index: &AnimationNodeIndex,
        graphs: &AssetStore<AnimationGraph>,
    ) {
        let Some(node) = self
            .get_animation_graph(graphs)
            .and_then(|graph| graph.live_node(*node_index))
        else {
            return;
        };
        if let Some(active) = self.graph_state.get_mut(node_index) {
            node.reset(&mut active.node_state);
        }
    }

    pub(crate) fn for_each_state<N: AnimationNode>(
        &self,
        graphs: &AssetStore<AnimationGraph>,
        blackboard: &AnimationBlackboard,
        mut f: impl FnMut(&N, &N::State, &AnimationBlackboard),
    ) {
        self.visit_states(graphs, blackboard, &mut f);
    }

    fn visit_states<N: AnimationNode>(
        &self,
        graphs: &AssetStore<AnimationGraph>,
        blackboard: &AnimationBlackboard,
        f: &mut dyn FnMut(&N, &N::State, &AnimationBlackboard),
    ) {
        let Some(graph) = self
            .graph_handle
            .as_ref()
            .and_then(|handle| graphs.get(handle))
        else {
            return;
        };

        for (node_index, node_state) in self.graph_state.iter() {
            let Some(live) = graph.live_node(*node_index) else {
                continue;
            };
            live.inner_states(&node_state.node_state, &mut |_, inner| {
                inner.visit_states(graphs, blackboard, f)
            });
            let Some(node) = live.as_reflect().downcast_ref::<N>() else {
                continue;
            };
            let Some(state) = node_state.node_state.downcast_ref::<N::State>() else {
                continue;
            };
            f(node, state, blackboard);
        }
    }

    /// Calls `f` with each playing node in node order, the nodes of the graphs it plays inside itself after it: its path from this graph, its weight and what it reports.
    pub fn for_each_node(
        &self,
        graphs: &AssetStore<AnimationGraph>,
        mut f: impl FnMut(&NodeVisit<'_>),
    ) {
        self.visit_nodes(graphs, &mut Vec::new(), &mut f);
    }

    fn visit_nodes(
        &self,
        graphs: &AssetStore<AnimationGraph>,
        path: &mut Vec<NodePathStep>,
        f: &mut dyn FnMut(&NodeVisit<'_>),
    ) {
        let Some(graph) = self.get_animation_graph(graphs) else {
            return;
        };
        for node in graph.node_indices() {
            let (Some(live), Some(active)) = (graph.live_node(node), self.graph_state.get(&node))
            else {
                continue;
            };
            path.push(NodePathStep { node, state: None });
            f(&NodeVisit {
                path,
                weight: active.weight,
                report: live.report(&active.node_state),
            });
            live.inner_states(&active.node_state, &mut |state, inner| {
                if let Some(step) = path.last_mut() {
                    step.state = state;
                }
                inner.visit_nodes(graphs, path, f);
            });
            path.pop();
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn for_each_state_mut<N: AnimationNode>(
        &mut self,
        graphs: &AssetStore<AnimationGraph>,
        blackboard: &AnimationBlackboard,
        mut f: impl FnMut(&N, &mut N::State, &AnimationBlackboard),
    ) {
        self.visit_states_mut(graphs, blackboard, &mut f);
    }

    #[cfg(any(test, feature = "test-support"))]
    fn visit_states_mut<N: AnimationNode>(
        &mut self,
        graphs: &AssetStore<AnimationGraph>,
        blackboard: &AnimationBlackboard,
        f: &mut dyn FnMut(&N, &mut N::State, &AnimationBlackboard),
    ) {
        let Some(graph) = self
            .graph_handle
            .as_ref()
            .and_then(|handle| graphs.get(handle))
        else {
            return;
        };

        for (node_index, node_state) in self.graph_state.iter_mut() {
            let Some(live) = graph.live_node(*node_index) else {
                continue;
            };
            live.inner_states_mut(&mut node_state.node_state, &mut |inner| {
                inner.visit_states_mut(graphs, blackboard, f)
            });
            let Some(node) = live.as_reflect().downcast_ref::<N>() else {
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
        self.warned_stateless = false;
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

    pub(crate) fn stateless_node<'a>(
        &self,
        graphs: &'a AssetStore<AnimationGraph>,
    ) -> Option<&'a str> {
        let graph = self.get_animation_graph(graphs)?;
        graph.iter().find_map(|node_index| {
            let node = graph.nodes.node_weight(*node_index)?;
            let live = node.live()?;
            let Some(node_state) = self.graph_state.get(&node_index) else {
                return Some(node.type_name());
            };
            let mut found = None;
            live.inner_states(&node_state.node_state, &mut |_, inner| {
                if found.is_none() {
                    found = inner.stateless_node(graphs);
                }
            });
            found
        })
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
        pool: &mut FramePool,
        output_frame: &mut AnimationFrame,
    ) -> bool {
        let Some(graph) = self.get_animation_graph(graphs) else {
            return false;
        };

        let mut graph_evaluator = AnimationGraphEvaluator::new();

        let mut stateless = false;

        for node_index in graph.iter_post_order() {
            let input_count = graph.get_node_inputs(node_index).count();
            let stack_start = graph_evaluator.stack_len().saturating_sub(input_count);
            let mut node_output = pool.acquire();
            let mut weight = 1.0;

            if let Some(node) = graph.live_node(node_index) {
                if let Some(node_state) = self.get_active_node_state_mut(&node_index) {
                    node.evaluate(
                        &mut node_state.node_state,
                        context,
                        bone_ids,
                        graph_evaluator.view_stack(stack_start),
                        pool,
                        &mut node_output,
                    );
                    weight = node_state.weight;
                } else {
                    stateless = true;
                    if !self.warned_stateless {
                        warn!(
                            "No node state found for node {} ({}), which plays the rest pose; make sure the animation player has been correctly initialized",
                            node_index.index(),
                            graph
                                .node(node_index)
                                .map_or("unknown", AnimationNodeData::type_name)
                        );
                    }
                }
            }

            for evaluated in graph_evaluator.evaluation_stack.drain(stack_start..) {
                pool.release(evaluated.frame);
            }

            graph_evaluator.push_evaluation(EvaluatedFrame {
                frame: node_output,
                weight,
            });
        }

        self.warned_stateless |= stateless;

        let mut result = graph_evaluator
            .pop_evaluation()
            .map(|evaluated| evaluated.frame)
            .unwrap_or_else(|| pool.acquire());

        std::mem::swap(output_frame, &mut result);
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

    pub(crate) fn iter(&self) -> impl Iterator<Item = &AnimationGraphState> {
        self.states.iter()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn iter_mut(&mut self) -> impl Iterator<Item = &mut AnimationGraphState> {
        self.states.iter_mut()
    }
}

#[cfg(test)]
mod tests {
    use concerto_foundation::assets::{AssetId, asset_store::AssetStore, handle::AssetHandle};
    use concerto_mesh::Skeleton;

    use glam::Mat4;
    use uuid::Uuid;

    use super::{AnimationGraph, AnimationGraphState};
    use crate::{
        blackboard::AnimationBlackboard,
        clip::{AnimationChanelOutput, AnimationChannel, AnimationClip},
        mask::{BoneMask, BoneMaskBranch},
        node::{AnimationClipNode, LayerNode, LayerSpace},
        pose::{FramePool, JointPose},
        testing::{Walks, made_live},
    };

    const HIP: Uuid = Uuid::from_u128(1);
    const ARM: Uuid = Uuid::from_u128(2);
    const BASE_X: f32 = 1.0;
    const OVERLAY_X: f32 = 2.0;

    fn skeleton() -> Skeleton {
        Skeleton::new(
            vec![Mat4::IDENTITY; 2],
            vec![None, Some(0)],
            vec!["hip".into(), "arm".into()],
            vec![JointPose::IDENTITY; 2],
            vec![HIP, ARM],
        )
        .unwrap()
    }

    fn held_at(x: f32) -> AnimationClip {
        let mut clip = AnimationClip::default();
        for bone in [HIP, ARM] {
            clip.add_channel(
                bone,
                AnimationChannel::new(
                    vec![0.0, 1.0],
                    AnimationChanelOutput::from_translation([[x, 0.0, 0.0]; 2].into_iter()),
                ),
            );
        }
        clip
    }

    struct Layered {
        walks: Walks,
        base: AssetHandle<AnimationClip>,
        overlay: AssetHandle<AnimationClip>,
        mask: AssetHandle<BoneMask>,
    }

    impl Layered {
        fn new() -> Self {
            let mut walks = Walks::new(AssetStore::new(), AssetStore::new());
            let base = AssetId::from_path("layer_order_base");
            let overlay = AssetId::from_path("layer_order_overlay");
            let mask = AssetId::from_path("layer_order_arm");
            walks.clips_mut().insert(base, held_at(BASE_X));
            walks.clips_mut().insert(overlay, held_at(OVERLAY_X));
            walks.masks_mut().insert(
                mask,
                BoneMask::new().with_branch(BoneMaskBranch::new("arm")),
            );
            Self {
                walks,
                base: AssetHandle::weak(base),
                overlay: AssetHandle::weak(overlay),
                mask: AssetHandle::weak(mask),
            }
        }

        fn graph(&self) -> AnimationGraph {
            let mut graph = AnimationGraph::new();
            graph.result_node().add_layer_input(
                LayerNode::new(self.mask.clone()).with_space(LayerSpace::Local),
                AnimationClipNode::new(self.base.clone()),
                AnimationClipNode::new(self.overlay.clone()),
            );
            graph
        }

        fn evaluate(&mut self, graph: AnimationGraph) -> Vec<f32> {
            let id = AssetId::from_path("layer_order_graph");
            self.walks.graphs_mut().insert(id, made_live(graph));
            let walks = &self.walks;
            let blackboard = AnimationBlackboard::default();
            let skeleton = skeleton();
            let mut state = AnimationGraphState::default();
            walks.create(&blackboard, |context| {
                state.initialize(AssetHandle::weak(id), walks.graphs(), context)
            });
            let mut pool = FramePool::new(skeleton.bone_count());
            pool.set_rest(&skeleton.rest_pose);
            let mut output = pool.acquire();
            let evaluated = walks.evaluate_with(&blackboard, None, Some(&skeleton), |context| {
                state.evaluate(
                    walks.graphs(),
                    context,
                    &skeleton.bone_ids,
                    &mut pool,
                    &mut output,
                )
            });
            assert!(evaluated, "the graph evaluated");
            output
                .joints()
                .iter()
                .map(|joint| joint.translation.x)
                .collect()
        }
    }

    fn assert_base_then_overlay(x: &[f32]) {
        assert_eq!(x.len(), 2);
        assert!(
            (x[0] - BASE_X).abs() < 1e-6,
            "the unmasked hip is the base's, got {}",
            x[0]
        );
        assert!(
            (x[1] - OVERLAY_X).abs() < 1e-6,
            "the masked arm is the overlay's, got {}",
            x[1]
        );
    }

    #[test]
    fn add_layer_input_hands_the_layer_its_base_first_and_its_overlay_second_and_a_load_keeps_them()
    {
        let mut layered = Layered::new();
        let built = layered.graph();
        let bytes = bincode::serialize(&layered.graph()).unwrap();
        let loaded: AnimationGraph = bincode::deserialize(&bytes).unwrap();

        assert_base_then_overlay(&layered.evaluate(built));
        assert_base_then_overlay(&layered.evaluate(loaded));
    }
}
