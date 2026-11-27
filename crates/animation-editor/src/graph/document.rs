use std::{collections::HashMap, fmt, path::Path, sync::Arc};

use concerto_animation::{
    graph::AnimationGraph,
    node::{
        PlaceholderNode, ReflectAnimationNode, ResultNode,
        state_machine::{AnimationFSMTrigger, AnimationStateMachine, StateMachineEditError},
    },
};
use concerto_ecs::{
    Component, Entity, Event, Query, Res, World,
    command::{CommandQueue, EntityCommandQueue},
    events::{event_reader::EventReader, event_writer::EventWriter},
    reflect::AppTypeRegistry,
    signal::{On, Signal, lifecycle::Add},
};
use concerto_editor::{
    asset_editor::{AssetEditor, EditorDocument, PreviewSpec, Unsaved, finish_asset_request},
    content::{AssetCreation, CreateAsset},
    guard::DiscardEdits,
    hosted::{Hosted, HostedApp},
    inspector::{DocumentValue, InspectedValues, PropertyTarget, RefreshInspector, TargetWorld},
    preview::PreviewFailed,
    project::ProjectState,
    tools::edits::PropertyCommitted,
};
use concerto_foundation::assets::{
    Asset, AssetId,
    content::{decode_content_asset, read_content_asset},
    handle::{AssetHandle, LiveLeaves},
    registry::{AssetEntry, AssetRegistry},
};
use concerto_name::Name;
use concerto_reflect::{Reflect, ReflectDefault, ReflectedValue, TypeRegistration, TypeRegistry};

use super::{
    copy::{GraphCopy, GraphNodeId},
    params::{FoundParam, GraphParameters, find_params},
    preview::graph_preview,
    stage::{GraphLive, GraphPreview, ParametersValue, PreviewValue, SentPreview, hosted_types},
};

const GRAPH_EXTENSION: &str = "gasset";
const NODE: &str = "Node";
const STATE: &str = "State";
const TRANSITION: &str = "Transition";
const READ_ONLY: &str = "The preview is not running, so the graph is read-only";
const IN_DIAGRAM: &str = "A state machine is in view, which holds states and transitions, not nodes; graph_up goes back to its graph";
const NO_MACHINE: &str = "no state machine is in view";
const IN_FLOW: &str =
    "The graph in view is not a state machine; open a state machine node with graph_open first";

/// A transition of the state machine in view: its two states and its place among the transitions between that pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransitionRef {
    /// The state the transition leaves.
    pub from: Name,
    /// The state the transition enters.
    pub to: Name,
    /// The transition's index among those from `from` to `to`, in the order the machine tries
    /// them.
    pub index: usize,
}

impl fmt::Display for TransitionRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} → {} #{}", self.from, self.to, self.index)
    }
}

/// What a graph tab has selected: a node, the link that makes `child` an input, or in a state machine a state or a transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphSelection {
    /// A node of the graph in view.
    Node(GraphNodeId),
    /// The link into a node's parent.
    Link {
        /// The node the link makes an input.
        child: GraphNodeId,
    },
    /// A state of the machine in view, by name.
    State(Name),
    /// A transition of the machine in view.
    Transition(TransitionRef),
}

/// The selected state as the inspector edits it, the document value `State`.
#[derive(Clone, Default, Reflect)]
#[reflect(Default)]
pub struct StateValue {
    /// The state's name, unique in its machine.
    pub name: Name,
    /// The graph asset the state plays.
    pub graph: AssetHandle<AnimationGraph>,
    /// Whether the machine starts in this state.
    pub initial: bool,
}

/// The selected transition as the inspector edits it, the document value `Transition`.
#[derive(Clone, Default, Reflect)]
#[reflect(Default)]
pub struct TransitionValue {
    /// When the transition fires.
    pub trigger: AnimationFSMTrigger,
    /// The cross-fade time or inertialization halflife, in seconds.
    pub blend_time: f32,
}

/// One transition of a state machine as a graph tab lists it; `place` is its index among those out of its source state.
#[derive(Clone, Debug, PartialEq)]
pub struct ShownTransition {
    /// The transition by its two states and its index between them.
    pub named: TransitionRef,
    /// The transition's index among those out of its source state.
    pub place: usize,
    /// The trigger as a label, such as `Instant` or `crouch = true`.
    pub trigger: String,
    /// The cross-fade time or inertialization halflife, in seconds.
    pub blend_time: f32,
}

fn value_label(value: &ReflectedValue) -> String {
    match value {
        ReflectedValue::Leaf(bytes) => match bincode::deserialize::<String>(bytes) {
            Ok(text) => text,
            Err(_) => format!("{} bytes", bytes.len()),
        },
        ReflectedValue::Struct(fields) | ReflectedValue::Variant { fields, .. } => {
            let fields: Vec<String> = fields
                .iter()
                .map(|(name, value)| format!("{name}: {}", value_label(value)))
                .collect();
            format!("{{ {} }}", fields.join(", "))
        }
        ReflectedValue::List(items) => {
            let items: Vec<String> = items.iter().map(value_label).collect();
            format!("[{}]", items.join(", "))
        }
    }
}

fn trigger_label(trigger: &AnimationFSMTrigger) -> String {
    match trigger {
        AnimationFSMTrigger::Instant => "Instant".to_owned(),
        AnimationFSMTrigger::BoolEquals { param, value } => format!("{} = {value}", param.0),
        AnimationFSMTrigger::Vec2NonZero { param } => format!("{} ≠ 0", param.0),
        AnimationFSMTrigger::OnMessage { message, value } => {
            let short = message.path().rsplit("::").next().unwrap_or_default();
            match value {
                Some(value) => format!("on {short} {}", value_label(value)),
                None => format!("on {short}"),
            }
        }
    }
}

/// Every transition of `machine`, by source state in state order and then in the order the machine tries them.
pub fn shown_transitions(machine: &AnimationStateMachine) -> Vec<ShownTransition> {
    let mut shown = Vec::new();
    for state in machine.states() {
        let mut between: HashMap<Name, usize> = HashMap::new();
        for transition in machine.transitions(state.name).into_iter().flatten() {
            let index = between.entry(transition.target).or_default();
            shown.push(ShownTransition {
                named: TransitionRef {
                    from: state.name,
                    to: transition.target,
                    index: *index,
                },
                place: transition.index,
                trigger: trigger_label(transition.trigger),
                blend_time: transition.blend_time,
            });
            *index += 1;
        }
    }
    shown
}

fn sentence(error: StateMachineEditError) -> String {
    let text = error.to_string();
    let mut letters = text.chars();
    match letters.next() {
        Some(first) => first.to_uppercase().chain(letters).collect(),
        None => text,
    }
}

fn has_state(machine: &AnimationStateMachine, name: Name) -> Result<(), String> {
    if machine.states().any(|state| state.name == name) {
        Ok(())
    } else {
        Err(format!(
            "There is no state {name}; graph_inspect lists the states"
        ))
    }
}

fn place_of(machine: &AnimationStateMachine, named: TransitionRef) -> Result<usize, String> {
    shown_transitions(machine)
        .into_iter()
        .find(|shown| shown.named == named)
        .map(|shown| shown.place)
        .ok_or_else(|| {
            format!("There is no transition {named}; graph_inspect lists the transitions")
        })
}

fn state_value(machine: &AnimationStateMachine, name: Name) -> Option<StateValue> {
    let state = machine.states().find(|state| state.name == name)?;
    Some(StateValue {
        name,
        graph: state.graph.clone(),
        initial: machine.initial_state() == Some(name),
    })
}

fn transition_value(
    machine: &AnimationStateMachine,
    named: TransitionRef,
) -> Option<TransitionValue> {
    let place = place_of(machine, named).ok()?;
    let transition = machine
        .transitions(named.from)
        .ok()?
        .find(|transition| transition.index == place)?;
    Some(TransitionValue {
        trigger: transition.trigger.clone(),
        blend_time: transition.blend_time,
    })
}

/// The working copy of one graph asset a tab has drilled through, kept until the tab reloads.
pub struct HeldGraph {
    /// The graph asset the copy was read from, which a save writes over.
    pub asset: AssetEntry,
    /// The graph as edited so far.
    pub copy: GraphCopy,
    /// Whether the copy has edits since it was read or saved; a save writes only these.
    pub changed: bool,
}

/// One level of a graph tab's stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphLevel {
    /// A graph asset, shown as a pose graph; `state` names the state of the machine above that plays it.
    Graph {
        /// The graph asset.
        asset: AssetId,
        /// The state of the machine above that plays the graph, if it was reached through one.
        state: Option<Name>,
    },
    /// A state machine node of the graph above, shown as a state diagram and edited in that graph's working copy.
    Machine {
        /// The state machine node in the graph above.
        node: GraphNodeId,
    },
}

struct CreatingState {
    address: String,
    name: Name,
}

/// What a graph's tab holds: the stack from its asset down to what is in view, a working copy per graph, and the selection.
#[derive(Component, Default)]
pub struct GraphDocument {
    graphs: Vec<HeldGraph>,
    levels: Vec<GraphLevel>,
    /// What is selected in the graph in view, if anything.
    pub selection: Option<GraphSelection>,
    /// How the preview shows and plays the graph, the document value `Preview`.
    pub preview: GraphPreview,
    /// The parameters the graph in view names, the document value `Parameters`.
    pub parameters: GraphParameters,
    /// The scene the preview shows, or none for an empty stage.
    pub shown_scene: Option<AssetId>,
    /// Why the preview's scene could not be read.
    pub scene_error: Option<String>,
    creating: Vec<CreatingState>,
    fixed: Vec<Arc<dyn PropertyTarget>>,
}

impl GraphDocument {
    /// The document of the tab `document`, publishing its `Preview` and `Parameters` values.
    pub fn of(document: Entity) -> Self {
        Self {
            fixed: vec![
                Arc::new(PreviewValue { document }),
                Arc::new(ParametersValue { document }),
            ],
            ..Default::default()
        }
    }

    /// The `Preview` value, as `WriteValue` and `set_property` address it.
    pub fn preview_value(&self) -> Option<Arc<dyn PropertyTarget>> {
        self.fixed.first().cloned()
    }

    /// The `Parameters` value, as `WriteValue` and `set_property` address it.
    pub fn parameters_value(&self) -> Option<Arc<dyn PropertyTarget>> {
        self.fixed.get(1).cloned()
    }

    /// Why `node` cannot be previewed from, or none when it can.
    pub fn preview_refusal(&self, node: GraphNodeId) -> Option<String> {
        if self.shown_machine().is_some() {
            return Some(format!(
                "A state machine is in view, so there is no node {node} to preview from; graph_up goes back to its graph"
            ));
        }
        let copy = self.graph()?;
        if copy.node(node).is_none() {
            return Some(format!(
                "There is no node {node} to preview from; graph_inspect lists the nodes"
            ));
        }
        (node == copy.result())
            .then(|| format!("Node {node} is the result node, which plays the whole graph already"))
    }

    /// Every parameter the graph in view names, the graphs its nodes play inside themselves included, by name and then by type.
    pub fn found_params(&self, sources: &Sources) -> Vec<FoundParam> {
        let mut found = Vec::new();
        let (Some(types), Some(shown)) = (sources.types, self.shown_asset()) else {
            return found;
        };
        let registry = types.read();
        let mut seen = vec![shown];
        let mut open = vec![shown];
        while let Some(asset) = open.pop() {
            let read;
            let copy = match self.graphs.iter().find(|held| held.asset.id == asset) {
                Some(held) => &held.copy,
                None => {
                    let nested = sources
                        .root
                        .zip(sources.assets.get(asset))
                        .and_then(|(root, entry)| read_copy(root, entry, sources.types).ok());
                    let Some(nested) = nested else {
                        continue;
                    };
                    read = nested;
                    &read
                }
            };
            for node in copy.nodes() {
                let Some(live) = node.live() else {
                    continue;
                };
                find_params(live.as_reflect(), &registry, &mut found);
                live.inner_graphs(&mut |handle| {
                    let inner = handle.id();
                    if !inner.is_nil() && !seen.contains(&inner) {
                        seen.push(inner);
                        open.push(inner);
                    }
                });
            }
        }
        found.sort_by(|a, b| {
            a.name
                .as_str()
                .cmp(b.name.as_str())
                .then_with(|| a.data.value_type_name().cmp(b.data.value_type_name()))
        });
        found
    }

    /// The stack, from the tab's asset down to the level in view.
    pub fn levels(&self) -> &[GraphLevel] {
        &self.levels
    }

    /// The working copy of every graph asset the tab has read.
    pub fn graphs(&self) -> &[HeldGraph] {
        &self.graphs
    }

    /// The working copies, to mark saved.
    pub fn graphs_mut(&mut self) -> &mut [HeldGraph] {
        &mut self.graphs
    }

    /// The graph asset in view, or under a state diagram the one that holds the machine.
    pub fn shown_asset(&self) -> Option<AssetId> {
        self.levels.iter().rev().find_map(|level| match level {
            GraphLevel::Graph { asset, .. } => Some(*asset),
            GraphLevel::Machine { .. } => None,
        })
    }

    /// The state machine node whose diagram is in view, or none while a pose graph is.
    pub fn shown_machine(&self) -> Option<GraphNodeId> {
        match self.levels.last()? {
            GraphLevel::Machine { node } => Some(*node),
            GraphLevel::Graph { .. } => None,
        }
    }

    /// The graph asset in view, or under a state diagram the one whose node the diagram shows.
    pub fn held(&self) -> Option<&HeldGraph> {
        let asset = self.shown_asset()?;
        self.graphs.iter().find(|held| held.asset.id == asset)
    }

    fn held_mut(&mut self) -> Option<&mut HeldGraph> {
        let asset = self.shown_asset()?;
        self.graphs.iter_mut().find(|held| held.asset.id == asset)
    }

    /// The working copy of [`held`](Self::held).
    pub fn graph(&self) -> Option<&GraphCopy> {
        self.held().map(|held| &held.copy)
    }

    /// The state machine in view, or none while a pose graph is.
    pub fn machine(&self) -> Option<&AnimationStateMachine> {
        let node = self.shown_machine()?;
        self.graph()?
            .node(node)?
            .live()?
            .as_reflect()
            .downcast_ref()
    }

    /// Whether the node `id` of the pose graph in view is a state machine, which its diagram edits and no `Node` value does.
    pub fn is_state_machine(&self, id: GraphNodeId) -> bool {
        self.graph()
            .and_then(|copy| copy.node(id))
            .and_then(|node| node.live())
            .is_some_and(|live| {
                live.as_reflect()
                    .downcast_ref::<AnimationStateMachine>()
                    .is_some()
            })
    }

    fn machine_mut(&mut self) -> Option<&mut AnimationStateMachine> {
        let node = self.shown_machine()?;
        self.held_mut()?
            .copy
            .node_mut(node)?
            .live_mut()?
            .as_reflect_mut()
            .downcast_mut()
    }

    /// The label of each level, as the breadcrumb shows them.
    pub fn breadcrumb(&self) -> Vec<String> {
        self.levels
            .iter()
            .map(|level| match level {
                GraphLevel::Graph {
                    state: Some(state), ..
                } => format!("{state} (state)"),
                GraphLevel::Graph { asset, state: None } => self
                    .graphs
                    .iter()
                    .find(|held| held.asset.id == *asset)
                    .map_or_else(String::new, |held| held.asset.name().into_owned()),
                GraphLevel::Machine { .. } => "States".to_owned(),
            })
            .collect()
    }

    /// The line a placeholder node shows as the status while it is selected.
    pub fn selected_warning(&self) -> Option<String> {
        let Some(GraphSelection::Node(id)) = self.selection else {
            return None;
        };
        if self.shown_machine().is_some() {
            return None;
        }
        let copy = self.graph()?;
        let node = copy.node(id)?;
        if !copy.editable() || node.live().is_some() {
            return None;
        }
        Some(node.warning.clone().unwrap_or_else(|| {
            format!("{} is not registered as an animation node", node.type_name)
        }))
    }

    fn selectable(&self, selection: GraphSelection) -> Result<(), String> {
        match (selection, self.machine()) {
            (GraphSelection::State(name), Some(machine)) => has_state(machine, name),
            (GraphSelection::Transition(named), Some(machine)) => {
                place_of(machine, named).map(|_| ())
            }
            (GraphSelection::State(_) | GraphSelection::Transition(_), None) => {
                Err(IN_FLOW.to_owned())
            }
            (GraphSelection::Node(_) | GraphSelection::Link { .. }, _)
                if self.shown_machine().is_some() =>
            {
                Err(IN_DIAGRAM.to_owned())
            }
            (GraphSelection::Node(id), _) => match self.graph() {
                Some(copy) if copy.node(id).is_some() => Ok(()),
                _ => Err(format!(
                    "There is no node {id}; graph_inspect lists the nodes"
                )),
            },
            (GraphSelection::Link { child }, _) => match self.graph() {
                Some(copy) if copy.link_of(child).is_some() => Ok(()),
                _ => Err(format!("Node {child} is not an input of any node")),
            },
        }
    }

    fn keep_selection(&mut self) {
        if self
            .selection
            .is_some_and(|selection| self.selectable(selection).is_err())
        {
            self.selection = None;
        }
    }

    fn selected_state(&self) -> Option<StateValue> {
        match self.selection {
            Some(GraphSelection::State(name)) => state_value(self.machine()?, name),
            _ => None,
        }
    }

    fn selected_transition(&self) -> Option<TransitionValue> {
        match self.selection {
            Some(GraphSelection::Transition(named)) => transition_value(self.machine()?, named),
            _ => None,
        }
    }

    fn write_state(&mut self, before: &StateValue, value: StateValue) -> Result<(), String> {
        if value.name.as_str().is_empty() {
            return Err("a state needs a name".to_owned());
        }
        if before.initial && !value.initial {
            return Err(
                "a machine always starts in a state; set initial on the state it should start in"
                    .to_owned(),
            );
        }
        let machine = self.machine_mut().ok_or(NO_MACHINE)?;
        let refused = |error: StateMachineEditError| error.to_string();
        machine
            .rename_state(before.name, value.name)
            .map_err(refused)?;
        if value.graph.id() != before.graph.id() {
            machine
                .set_state_graph(value.name, value.graph)
                .map_err(refused)?;
        }
        if value.initial && !before.initial {
            machine.set_initial(value.name).map_err(refused)?;
        }
        self.selection = Some(GraphSelection::State(value.name));
        Ok(())
    }

    fn write_transition(&mut self, value: TransitionValue) -> Result<(), String> {
        let Some(GraphSelection::Transition(named)) = self.selection else {
            return Err(NO_MACHINE.to_owned());
        };
        let machine = self.machine_mut().ok_or(NO_MACHINE)?;
        let place = place_of(machine, named)?;
        machine
            .set_transition(named.from, place, value.trigger, value.blend_time)
            .map_err(|error| error.to_string())
    }

    fn show(&mut self, root: HeldGraph) {
        self.levels = vec![GraphLevel::Graph {
            asset: root.asset.id,
            state: None,
        }];
        self.graphs = vec![root];
        self.selection = None;
        self.preview.from = None;
        self.creating.clear();
    }

    fn up(&mut self, count: usize) -> Result<GraphChange, String> {
        if self.levels.len() < 2 {
            return Err("The tab's own graph is in view, so there is no level above it".to_owned());
        }
        let kept = self.levels.len().saturating_sub(count).max(1);
        self.levels.truncate(kept);
        self.selection = None;
        self.preview.from = None;
        Ok(GraphChange::default())
    }

    fn open(&mut self, target: &OpenTarget, sources: &Sources) -> Result<GraphChange, String> {
        let opened = match *target {
            OpenTarget::Node(id) => {
                if self.shown_machine().is_some() {
                    return Err(IN_DIAGRAM.to_owned());
                }
                let copy = self.graph().ok_or("No graph is open")?;
                let node = copy.node(id).ok_or_else(|| {
                    format!("There is no node {id}; graph_inspect lists the nodes")
                })?;
                let live = node.live().ok_or_else(|| {
                    format!("Node {id} could not be built, so it cannot be opened")
                })?;
                if live
                    .as_reflect()
                    .downcast_ref::<AnimationStateMachine>()
                    .is_some()
                {
                    self.levels.push(GraphLevel::Machine { node: id });
                    self.selection = None;
                    self.preview.from = None;
                    return Ok(GraphChange::default());
                }
                let mut inner = None;
                live.inner_graphs(&mut |handle| {
                    inner.get_or_insert(handle.id());
                });
                Opened {
                    asset: inner.ok_or_else(|| {
                        format!(
                            "Node {id}, {}, plays no graph inside itself, so there is nothing to open",
                            node.described()
                        )
                    })?,
                    state: None,
                    unset: format!("Node {id} names no graph; select it and set its graph"),
                }
            }
            OpenTarget::State(name) => {
                let machine = self.machine().ok_or(IN_FLOW)?;
                let state = machine
                    .states()
                    .find(|state| state.name == name)
                    .ok_or_else(|| {
                        format!("There is no state {name}; graph_inspect lists the states")
                    })?;
                Opened {
                    asset: state.graph.id(),
                    state: Some(name),
                    unset: format!("State {name} names no graph; select it and set its graph"),
                }
            }
        };
        if opened.asset.is_nil() {
            return Err(opened.unset);
        }
        let entry = sources
            .assets
            .get(opened.asset)
            .filter(|entry| entry.is::<AnimationGraph>())
            .ok_or_else(|| {
                format!(
                    "The graph {} is not an AnimationGraph of this project; {}",
                    opened.asset.simple_hex(),
                    opened.unset.split_once("; ").map_or("", |(_, fix)| fix)
                )
            })?;
        let in_stack = self.levels.iter().any(
            |level| matches!(level, GraphLevel::Graph { asset, .. } if *asset == opened.asset),
        );
        if in_stack {
            return Err(format!(
                "{} is already open in this tab, above what is in view; a graph cannot be opened inside itself",
                entry.name()
            ));
        }
        if !self.graphs.iter().any(|held| held.asset.id == opened.asset) {
            let root = sources.root.ok_or("Open a project first")?;
            let copy =
                read_copy(root, entry, sources.types).map_err(|error| format!("{error:#}"))?;
            self.graphs.push(HeldGraph {
                asset: entry.clone(),
                copy,
                changed: false,
            });
        }
        self.levels.push(GraphLevel::Graph {
            asset: opened.asset,
            state: opened.state,
        });
        self.selection = None;
        self.preview.from = None;
        Ok(GraphChange::default())
    }

    fn new_state_folder(&self, name: Name) -> Result<String, String> {
        let machine = self.machine().ok_or(IN_FLOW)?;
        let held = self.held().ok_or("No graph is open")?;
        if !held.copy.editable() {
            return Err(READ_ONLY.to_owned());
        }
        if name.as_str().is_empty() {
            return Err("Give the state a name".to_owned());
        }
        if machine.states().any(|state| state.name == name) {
            return Err(sentence(StateMachineEditError::DuplicateState { name }));
        }
        Ok(held.asset.folder().to_owned())
    }

    fn add_state(&mut self, name: Name, graph: AssetId) -> Result<GraphChange, String> {
        if name.as_str().is_empty() {
            return Err("Give the state a name".to_owned());
        }
        let machine = self.machine_mut().ok_or(IN_FLOW)?;
        machine
            .add_state(name, AssetHandle::weak(graph))
            .map_err(sentence)?;
        Ok(GraphChange {
            graph: Some(graph),
            ..Default::default()
        })
    }

    fn edit_machine(&mut self, edit: &GraphEdit, sources: &Sources) -> Result<GraphChange, String> {
        match *edit {
            GraphEdit::AddState { name, graph } => {
                let graph = graph.ok_or("The state has no graph")?;
                let known = sources
                    .assets
                    .get(graph)
                    .is_some_and(|entry| entry.is::<AnimationGraph>());
                if !known {
                    return Err(format!(
                        "{} is not an AnimationGraph of this project; list_assets kind=AnimationGraph lists them",
                        graph.simple_hex()
                    ));
                }
                self.add_state(name, graph)
            }
            GraphEdit::AddTransition { from, to } => {
                let machine = self.machine_mut().ok_or(IN_FLOW)?;
                let place = machine
                    .add_transition(from, to, AnimationFSMTrigger::Instant, 0.0)
                    .map_err(sentence)?;
                let named = shown_transitions(machine)
                    .into_iter()
                    .find(|shown| shown.named.from == from && shown.place == place)
                    .map(|shown| shown.named);
                self.selection = named.map(GraphSelection::Transition);
                Ok(GraphChange {
                    transition: named,
                    ..Default::default()
                })
            }
            GraphEdit::RemoveState(name) => {
                let machine = self.machine_mut().ok_or(IN_FLOW)?;
                machine.remove_state(name).map_err(sentence)?;
                Ok(GraphChange::default())
            }
            GraphEdit::RemoveTransition(named) => {
                let machine = self.machine_mut().ok_or(IN_FLOW)?;
                let place = place_of(machine, named)?;
                machine
                    .remove_transition(named.from, place)
                    .map_err(sentence)?;
                Ok(GraphChange::default())
            }
            _ => Ok(GraphChange::default()),
        }
    }

    fn edit_flow(&mut self, edit: &GraphEdit, sources: &Sources) -> Result<GraphChange, String> {
        if self.shown_machine().is_some() {
            return Err(IN_DIAGRAM.to_owned());
        }
        let copy = &mut self.held_mut().ok_or("No graph is open")?.copy;
        match edit {
            GraphEdit::AddNode {
                type_name,
                parent,
                slot,
            } => {
                let types = sources.types.ok_or(READ_ONLY)?;
                add_node(copy, &types.read(), type_name, *parent, *slot)
            }
            GraphEdit::AddParent { type_name, child } => {
                let types = sources.types.ok_or(READ_ONLY)?;
                let added = add_node(copy, &types.read(), type_name, None, None)?;
                let Some(node) = added.node else {
                    return Ok(added);
                };
                match copy.connect(*child, node, None) {
                    Ok(connected) => Ok(GraphChange {
                        node: Some(node),
                        slot: Some(connected.slot),
                        ..Default::default()
                    }),
                    Err(reason) => {
                        copy.remove_node(node)?;
                        Err(reason)
                    }
                }
            }
            GraphEdit::Connect {
                child,
                parent,
                slot,
            } => {
                let connected = copy.connect(*child, *parent, *slot)?;
                Ok(GraphChange {
                    slot: Some(connected.slot),
                    loose: connected.loose,
                    ..Default::default()
                })
            }
            GraphEdit::RemoveNode(id) => {
                copy.remove_node(*id)?;
                Ok(GraphChange::default())
            }
            GraphEdit::RemoveLink { child } => {
                copy.remove_link(*child)?;
                Ok(GraphChange::default())
            }
            _ => Ok(GraphChange::default()),
        }
    }
}

struct Opened {
    asset: AssetId,
    state: Option<Name>,
    unset: String,
}

/// What reading a graph's nested graphs needs: the preview's types, the project's assets and its root.
pub struct Sources<'a> {
    /// The preview's type registry, or `None` while the preview is not running.
    pub types: Option<&'a AppTypeRegistry>,
    /// The project's asset registry.
    pub assets: &'a AssetRegistry,
    /// The project's root, or `None` with no project open.
    pub root: Option<&'a Path>,
}

/// What a graph tab drills into: a node that plays a graph inside itself, or a state of the machine in view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenTarget {
    /// A node of the graph in view: a state machine, or a node that plays a graph asset.
    Node(GraphNodeId),
    /// A state of the machine in view, whose graph opens.
    State(Name),
}

/// One edit of what a graph tab has in view, or a move through its stack.
#[derive(Clone, Debug, PartialEq)]
pub enum GraphEdit {
    /// Adds a node of the type named by its short name or full path, loose or as an input of `parent`.
    AddNode {
        /// The node type's short name or full path.
        type_name: String,
        /// The node the new one becomes an input of, or `None` to leave it loose.
        parent: Option<GraphNodeId>,
        /// The parent's slot to fill, or `None` for its first empty slot.
        slot: Option<usize>,
    },
    /// Adds a node of the type named by its short name or full path and makes `child` its first input.
    AddParent {
        /// The node type's short name or full path.
        type_name: String,
        /// The node that becomes the new node's first input.
        child: GraphNodeId,
    },
    /// Makes `child` an input of `parent`, at `slot` or at its first empty slot.
    Connect {
        /// The node that becomes an input.
        child: GraphNodeId,
        /// The node that takes it.
        parent: GraphNodeId,
        /// The parent's slot to fill, or `None` for its first empty slot.
        slot: Option<usize>,
    },
    /// Removes a node, leaving its inputs loose.
    RemoveNode(GraphNodeId),
    /// Takes `child` out of its parent's inputs.
    RemoveLink {
        /// The node to take out, which stays as a loose node.
        child: GraphNodeId,
    },
    /// Adds a state playing `graph`, or without one a new graph asset named as the state, created beside the graph in view.
    AddState {
        /// The new state's name.
        name: Name,
        /// The graph asset the state plays, or `None` to create one.
        graph: Option<AssetId>,
    },
    /// Adds a transition with an `Instant` trigger and no blend time, and selects it; `from` may equal `to`.
    AddTransition {
        /// The state the transition leaves.
        from: Name,
        /// The state the transition enters.
        to: Name,
    },
    /// Removes a state with every transition to or from it; its graph asset stays.
    RemoveState(Name),
    /// Removes a transition.
    RemoveTransition(TransitionRef),
    /// Drills into a node's graph, a state machine node's diagram, or a state's graph.
    Open(OpenTarget),
    /// Goes up this many levels, stopping at the tab's own graph.
    Up(usize),
    /// Selects a node, a link, a state or a transition, or nothing.
    Select(Option<GraphSelection>),
}

impl GraphEdit {
    fn changes_graph(&self) -> bool {
        !matches!(
            self,
            GraphEdit::Open(_) | GraphEdit::Up(_) | GraphEdit::Select(_)
        )
    }

    fn on_machine(&self) -> bool {
        matches!(
            self,
            GraphEdit::AddState { .. }
                | GraphEdit::AddTransition { .. }
                | GraphEdit::RemoveState(_)
                | GraphEdit::RemoveTransition(_)
        )
    }
}

/// Applies `edit` to what the tab `document` has in view; the outcome is a [`GraphEdited`].
///
/// The graph tab's canvas and the graph MCP tools send it.
pub struct EditGraph {
    /// The graph tab's document.
    pub document: Entity,
    /// The edit to apply.
    pub edit: GraphEdit,
}

impl Signal for EditGraph {}

/// What an applied [`GraphEdit`] changed: the node it added, the slot a connection took, the input it displaced, the transition it added, and the graph a new state plays.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GraphChange {
    /// The node the edit added.
    pub node: Option<GraphNodeId>,
    /// The slot a connection took.
    pub slot: Option<usize>,
    /// The input a connection displaced, now loose.
    pub loose: Option<GraphNodeId>,
    /// The transition the edit added.
    pub transition: Option<TransitionRef>,
    /// The graph asset a new state plays.
    pub graph: Option<AssetId>,
}

/// The outcome of one [`EditGraph`]; a refusal carries the reason the tab's status shows.
#[derive(Event, Clone, Debug, PartialEq)]
pub struct GraphEdited {
    /// The graph tab's document.
    pub document: Entity,
    /// The edit that was asked for.
    pub edit: GraphEdit,
    /// What the edit changed, or why it was refused.
    pub result: Result<GraphChange, String>,
}

/// Opens an `AnimationGraph` in a tab that shows its nodes on a canvas.
#[derive(Default)]
pub struct AnimationGraphEditor;

impl AssetEditor for AnimationGraphEditor {
    fn build(&self, editor: &mut EntityCommandQueue) {
        let state = GraphDocument::of(editor.entity());
        let values = InspectedValues(state.fixed.clone());
        editor.insert((state, values, SentPreview::default(), GraphLive::default()));
    }

    fn preview(&self) -> Option<PreviewSpec> {
        Some(PreviewSpec {
            build: graph_preview,
        })
    }

    fn bottom_host(&self) -> bool {
        true
    }
}

/// The node types a graph tab can add, in short-name order: every registered animation node but the result node and the placeholder.
pub fn node_types(registry: &TypeRegistry) -> Vec<Arc<TypeRegistration>> {
    let hidden = [
        std::any::type_name::<ResultNode>(),
        std::any::type_name::<PlaceholderNode>(),
    ];
    let mut types: Vec<_> = registry
        .iter()
        .filter(|registration| {
            registration.data::<ReflectAnimationNode>().is_some()
                && !hidden.contains(&registration.type_path())
        })
        .collect();
    types.sort_by(|a, b| {
        a.short_name()
            .cmp(b.short_name())
            .then_with(|| a.type_path().cmp(b.type_path()))
    });
    types
}

fn node_type(registry: &TypeRegistry, name: &str) -> Result<Arc<TypeRegistration>, String> {
    let types = node_types(registry);
    if let Some(found) = types
        .iter()
        .find(|registration| registration.type_path() == name)
    {
        return Ok(found.clone());
    }
    let named: Vec<_> = types
        .iter()
        .filter(|registration| registration.short_name() == name)
        .collect();
    match named[..] {
        [found] => Ok(found.clone()),
        [] => {
            let names: Vec<&str> = types
                .iter()
                .map(|registration| registration.short_name())
                .collect();
            Err(format!(
                "{name} is not a node type that can be added; the types are {}",
                names.join(", ")
            ))
        }
        _ => {
            let paths: Vec<&str> = named
                .iter()
                .map(|registration| registration.type_path())
                .collect();
            Err(format!(
                "{name} names several node types; use a full path: {}",
                paths.join(", ")
            ))
        }
    }
}

fn add_node(
    copy: &mut GraphCopy,
    registry: &TypeRegistry,
    type_name: &str,
    parent: Option<GraphNodeId>,
    slot: Option<usize>,
) -> Result<GraphChange, String> {
    let registration = node_type(registry, type_name)?;
    let data = registration
        .data::<ReflectAnimationNode>()
        .ok_or_else(|| format!("{type_name} is not an animation node"))?;
    let mut leaves = LiveLeaves::from_parts(registry, None);
    let (built, _) = (data.from_value)(&ReflectedValue::Struct(Vec::new()), &mut leaves);
    let node = copy.add(registration.type_path().to_owned(), built);
    let Some(parent) = parent else {
        return Ok(GraphChange {
            node: Some(node),
            ..Default::default()
        });
    };
    match copy.connect(node, parent, slot) {
        Ok(connected) => Ok(GraphChange {
            node: Some(node),
            slot: Some(connected.slot),
            loose: connected.loose,
            ..Default::default()
        }),
        Err(reason) => {
            copy.remove_node(node)?;
            Err(reason)
        }
    }
}

fn apply(
    state: &mut GraphDocument,
    edit: &GraphEdit,
    sources: &Sources,
) -> Result<GraphChange, String> {
    if state.levels.is_empty() {
        return Err("No graph is open".to_owned());
    }
    match edit {
        GraphEdit::Select(selection) => {
            if let Some(selection) = selection {
                state.selectable(*selection)?;
            }
            state.selection = *selection;
            Ok(GraphChange::default())
        }
        GraphEdit::Up(count) => state.up(*count),
        GraphEdit::Open(target) => state.open(target, sources),
        _ => {
            if !state.graph().is_some_and(GraphCopy::editable) {
                return Err(READ_ONLY.to_owned());
            }
            let change = if edit.on_machine() {
                state.edit_machine(edit, sources)?
            } else {
                state.edit_flow(edit, sources)?
            };
            if let Some(held) = state.held_mut() {
                held.changed = true;
            }
            state.keep_selection();
            if let (GraphEdit::AddNode { .. } | GraphEdit::AddParent { .. }, Some(node)) =
                (edit, change.node)
            {
                state.selection = Some(GraphSelection::Node(node));
            }
            Ok(change)
        }
    }
}

fn node_value(document: Entity, id: GraphNodeId) -> Arc<dyn PropertyTarget> {
    Arc::new(DocumentValue::<GraphDocument> {
        name: NODE.to_owned(),
        document,
        read: Box::new(move |state| {
            let node = state.graph()?.node(id)?.live()?;
            Some(node.as_reflect())
        }),
        edit: Box::new(move |state| {
            let node = state.held_mut()?.copy.node_mut(id)?.live_mut()?;
            Some(node.as_reflect_mut() as &mut dyn Reflect)
        }),
    })
}

struct StateTarget {
    document: Entity,
}

impl PropertyTarget for StateTarget {
    fn name(&self) -> String {
        STATE.to_owned()
    }

    fn world(&self) -> TargetWorld {
        TargetWorld::Editor
    }

    fn read(&self, world: &World, read: &mut dyn FnMut(&dyn Reflect)) -> bool {
        let value = world
            .get_component_for_entity::<GraphDocument>(self.document)
            .and_then(GraphDocument::selected_state);
        let Some(value) = value else {
            return false;
        };
        read(&value);
        true
    }

    fn edit(&self, world: &mut World, edit: &mut dyn FnMut(&mut dyn Reflect)) -> bool {
        self.try_edit(world, edit).unwrap_or(false)
    }

    fn try_edit(
        &self,
        world: &mut World,
        edit: &mut dyn FnMut(&mut dyn Reflect),
    ) -> Result<bool, String> {
        let before = world
            .get_component_for_entity::<GraphDocument>(self.document)
            .and_then(GraphDocument::selected_state);
        let Some(before) = before else {
            return Ok(false);
        };
        let mut value = before.clone();
        edit(&mut value);
        let Some(held) = world.get_component_for_entity_mut::<GraphDocument>(self.document) else {
            return Ok(false);
        };
        held.write_state(&before, value).map(|()| true)
    }

    fn types(&self, world: &World) -> Option<AppTypeRegistry> {
        hosted_types(world, self.document)
    }

    fn document(&self) -> Option<Entity> {
        Some(self.document)
    }
}

struct TransitionTarget {
    document: Entity,
}

impl PropertyTarget for TransitionTarget {
    fn name(&self) -> String {
        TRANSITION.to_owned()
    }

    fn world(&self) -> TargetWorld {
        TargetWorld::Editor
    }

    fn read(&self, world: &World, read: &mut dyn FnMut(&dyn Reflect)) -> bool {
        let value = world
            .get_component_for_entity::<GraphDocument>(self.document)
            .and_then(GraphDocument::selected_transition);
        let Some(value) = value else {
            return false;
        };
        read(&value);
        true
    }

    fn edit(&self, world: &mut World, edit: &mut dyn FnMut(&mut dyn Reflect)) -> bool {
        self.try_edit(world, edit).unwrap_or(false)
    }

    fn try_edit(
        &self,
        world: &mut World,
        edit: &mut dyn FnMut(&mut dyn Reflect),
    ) -> Result<bool, String> {
        let value = world
            .get_component_for_entity::<GraphDocument>(self.document)
            .and_then(GraphDocument::selected_transition);
        let Some(mut value) = value else {
            return Ok(false);
        };
        edit(&mut value);
        let Some(held) = world.get_component_for_entity_mut::<GraphDocument>(self.document) else {
            return Ok(false);
        };
        held.write_transition(value).map(|()| true)
    }

    fn types(&self, world: &World) -> Option<AppTypeRegistry> {
        hosted_types(world, self.document)
    }

    fn document(&self) -> Option<Entity> {
        Some(self.document)
    }
}

fn publish_selection(document: Entity, state: &mut GraphDocument, cmd: &mut CommandQueue) {
    let value: Option<Arc<dyn PropertyTarget>> = match state.selection {
        Some(GraphSelection::Node(id)) => state
            .graph()
            .and_then(|copy| copy.node(id))
            .and_then(|node| node.live())
            .filter(|_| !state.is_state_machine(id))
            .map(|_| node_value(document, id)),
        Some(GraphSelection::State(_)) => state
            .selected_state()
            .map(|_| Arc::new(StateTarget { document }) as Arc<dyn PropertyTarget>),
        Some(GraphSelection::Transition(_)) => state
            .selected_transition()
            .map(|_| Arc::new(TransitionTarget { document }) as Arc<dyn PropertyTarget>),
        Some(GraphSelection::Link { .. }) | None => None,
    };
    let values = value.into_iter().chain(state.fixed.iter().cloned());
    cmd.insert(InspectedValues(values.collect()), document);
    cmd.trigger(RefreshInspector);
}

struct Shown {
    levels: Vec<GraphLevel>,
    selection: Option<GraphSelection>,
}

impl Shown {
    fn of(state: &GraphDocument) -> Self {
        Self {
            levels: state.levels.clone(),
            selection: state.selection,
        }
    }
}

fn settle(
    document: Entity,
    editor: &mut EditorDocument,
    state: &mut GraphDocument,
    before: Shown,
    edit: &GraphEdit,
    result: Result<GraphChange, String>,
    edited: &mut EventWriter<GraphEdited>,
    cmd: &mut CommandQueue,
) {
    let status = match &result {
        Ok(_) => state.selected_warning().unwrap_or_default(),
        Err(reason) => reason.clone(),
    };
    if editor.status != status {
        editor.status = status;
    }
    if result.is_ok() && edit.changes_graph() {
        cmd.insert(Unsaved, document);
    }
    if state.selection != before.selection || state.levels != before.levels {
        publish_selection(document, state, cmd);
    }
    edited.write(GraphEdited {
        document,
        edit: edit.clone(),
        result,
    });
}

pub(crate) fn apply_graph_edit(
    on: On<EditGraph>,
    mut documents: Query<(&mut EditorDocument, &mut GraphDocument, Option<&HostedApp>)>,
    assets: Res<AssetRegistry>,
    project: Res<ProjectState>,
    mut edited: EventWriter<GraphEdited>,
    mut cmd: CommandQueue,
) {
    let EditGraph { document, edit } = on.signal();
    let Some((mut editor, mut state, hosted)) = documents.get_entity_mut(*document) else {
        return;
    };
    let before = Shown::of(&state);
    let result = if let GraphEdit::AddState { name, graph: None } = edit {
        match state.new_state_folder(*name) {
            Ok(folder) => {
                state.creating.push(CreatingState {
                    address: format!("{folder}/{name}.{GRAPH_EXTENSION}"),
                    name: *name,
                });
                cmd.trigger(CreateAsset {
                    kind: AnimationGraph::name().to_owned(),
                    folder: Some(folder),
                    name: name.to_string(),
                    open: false,
                });
                return;
            }
            Err(reason) => Err(reason),
        }
    } else {
        let types = hosted.and_then(HostedApp::type_registry);
        let sources = Sources {
            types: types.as_ref(),
            assets: &assets,
            root: project
                .project
                .as_ref()
                .map(|project| project.root.as_path()),
        };
        apply(&mut state, edit, &sources)
    };
    settle(
        *document,
        &mut editor,
        &mut state,
        before,
        edit,
        result,
        &mut edited,
        &mut cmd,
    );
}

pub(crate) fn finish_state_creation(
    mut created: EventReader<AssetCreation>,
    mut documents: Query<(Entity, &mut EditorDocument, &mut GraphDocument)>,
    mut edited: EventWriter<GraphEdited>,
    mut cmd: CommandQueue,
) {
    for creation in created.read() {
        for (document, mut editor, mut state) in documents.iter_mut() {
            let Some(at) = state
                .creating
                .iter()
                .position(|creating| creating.address == creation.address)
            else {
                continue;
            };
            let creating = state.creating.remove(at);
            let before = Shown::of(&state);
            let result = creation
                .result
                .clone()
                .and_then(|graph| state.add_state(creating.name, graph));
            if result.is_ok()
                && let Some(held) = state.held_mut()
            {
                held.changed = true;
            }
            settle(
                document,
                &mut editor,
                &mut state,
                before,
                &GraphEdit::AddState {
                    name: creating.name,
                    graph: None,
                },
                result,
                &mut edited,
                &mut cmd,
            );
        }
    }
}

/// Reads the graph `asset` names under `root`, checking its id and its kind.
///
/// # Errors
///
/// Returns an error when the file cannot be read or decoded, or when its header names
/// another asset or another kind.
pub fn read_graph(root: &Path, asset: &AssetEntry) -> anyhow::Result<AnimationGraph> {
    let bytes = std::fs::read(root.join(&asset.address))?;
    let (header, _) = read_content_asset(&bytes)?;
    anyhow::ensure!(
        header.asset_id == asset.id && header.kind == AnimationGraph::name(),
        "Asset identity or type mismatch"
    );
    Ok(decode_content_asset::<AnimationGraph>(&bytes)?.1)
}

fn read_copy(
    root: &Path,
    asset: &AssetEntry,
    types: Option<&AppTypeRegistry>,
) -> anyhow::Result<GraphCopy> {
    let mut graph = read_graph(root, asset)?;
    if let Some(types) = types {
        graph.make_live(&types.read(), None)?;
    }
    Ok(GraphCopy::read(graph, types.is_some()))
}

fn hold(
    document: Entity,
    state: &mut GraphDocument,
    asset: &AssetEntry,
    copy: GraphCopy,
    cmd: &mut CommandQueue,
) {
    state.show(HeldGraph {
        asset: asset.clone(),
        copy,
        changed: false,
    });
    publish_selection(document, state, cmd);
}

pub(crate) fn load_graph(
    mut documents: Query<(
        Entity,
        &mut EditorDocument,
        &mut GraphDocument,
        Option<&HostedApp>,
        Option<&PreviewFailed>,
    )>,
    project: Res<ProjectState>,
    mut cmd: CommandQueue,
) {
    let Some(root) = project.project.as_ref().map(|project| project.root.clone()) else {
        return;
    };
    for (entity, mut document, mut state, hosted, failed) in documents.iter_mut() {
        if hosted.is_none() && failed.is_none() {
            continue;
        }
        let Some(asset) = document.pending.clone() else {
            continue;
        };
        let types = hosted.and_then(HostedApp::type_registry);
        match read_copy(&root, &asset, types.as_ref()) {
            Ok(copy) => {
                hold(entity, &mut state, &asset, copy, &mut cmd);
                finish_asset_request(&mut document, &asset, Ok(()));
            }
            Err(error) => {
                finish_asset_request(&mut document, &asset, Err(error));
            }
        }
    }
}

pub(crate) fn restore_discarded_graph(
    on: On<DiscardEdits>,
    mut documents: Query<(&EditorDocument, &mut GraphDocument, Option<&HostedApp>)>,
    project: Res<ProjectState>,
    mut cmd: CommandQueue,
) {
    let document = on.signal().0;
    let Some((editor, mut state, hosted)) = documents.get_entity_mut(document) else {
        return;
    };
    let (Some(project), Some(asset)) = (&project.project, &editor.current) else {
        return;
    };
    let types = hosted.and_then(HostedApp::type_registry);
    match read_copy(&project.root, asset, types.as_ref()) {
        Ok(copy) => hold(document, &mut state, asset, copy, &mut cmd),
        Err(error) => {
            log::warn!("Discarded edits stay: {error:#}");
            cmd.insert(Unsaved, document);
        }
    }
}

pub(crate) fn follow_value_edits(
    mut committed: EventReader<Hosted<PropertyCommitted>>,
    mut documents: Query<&mut GraphDocument>,
    mut cmd: CommandQueue,
) {
    for hosted in committed.read() {
        if hosted.event.result.is_err() || !hosted.event.address.target.saved() {
            continue;
        }
        let Some(mut state) = documents.get_entity_mut(hosted.document) else {
            continue;
        };
        if let Some(held) = state.held_mut() {
            held.changed = true;
        }
        cmd.insert(Unsaved, hosted.document);
    }
}

pub(crate) fn clear_status_on_edit(
    on: On<Add<Unsaved>>,
    mut documents: Query<(&mut EditorDocument, &GraphDocument)>,
) {
    if let Some((mut document, state)) = documents.get_entity_mut(on.signal().entity) {
        let status = state.selected_warning().unwrap_or_default();
        if document.status != status {
            document.status = status;
        }
    }
}
