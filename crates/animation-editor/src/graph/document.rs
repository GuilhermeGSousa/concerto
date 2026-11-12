use std::{collections::HashMap, fmt, path::Path, sync::Arc};

use concerto_animation::{
    graph::AnimationGraph,
    node::{
        PlaceholderNode, ReflectAnimationNode, ResultNode,
        state_machine::{AnimationFSMTrigger, AnimationStateMachine, StateMachineEditError},
    },
};
use concerto_app::App;
use concerto_ecs::{
    Component, Entity, Event, Query, Res,
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
    inspector::{DocumentValue, InspectedValues, PropertyTarget, RefreshInspector},
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

use super::copy::{GraphCopy, GraphNodeId};

const GRAPH_EXTENSION: &str = "gasset";
const NODE: &str = "Node";
const STATE: &str = "State";
const TRANSITION: &str = "Transition";
const READ_ONLY: &str = "The preview is not running, so the graph is read-only";
const IN_DIAGRAM: &str = "A state machine is in view, which holds states and transitions, not nodes; graph_up goes back to its graph";
const IN_FLOW: &str =
    "The graph in view is not a state machine; open a state machine node with graph_open first";

/// A transition of the state machine in view: its two states and its place among the transitions between that pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransitionRef {
    pub from: Name,
    pub to: Name,
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
    Node(GraphNodeId),
    Link { child: GraphNodeId },
    State(Name),
    Transition(TransitionRef),
}

/// The selected state as the inspector edits it, the document value `State`.
#[derive(Clone, Default, Reflect)]
#[reflect(Default)]
pub struct StateValue {
    pub name: Name,
    pub graph: AssetHandle<AnimationGraph>,
    pub initial: bool,
}

/// The selected transition as the inspector edits it, the document value `Transition`.
#[derive(Clone, Default, Reflect)]
#[reflect(Default)]
pub struct TransitionValue {
    pub trigger: AnimationFSMTrigger,
    pub blend_time: f32,
}

/// One transition of a state machine as a graph tab lists it; `place` is its index among those out of its source state.
#[derive(Clone, Debug, PartialEq)]
pub struct ShownTransition {
    pub named: TransitionRef,
    pub place: usize,
    pub trigger: String,
    pub blend_time: f32,
}

fn trigger_label(trigger: &AnimationFSMTrigger) -> String {
    match trigger {
        AnimationFSMTrigger::Instant => "Instant".to_owned(),
        AnimationFSMTrigger::OnAnimationEnd => "OnAnimationEnd".to_owned(),
        AnimationFSMTrigger::BoolEquals { param, value } => format!("{} = {value}", param.0),
        AnimationFSMTrigger::Vec2NonZero { param } => format!("{} ≠ 0", param.0),
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
    pub asset: AssetEntry,
    pub copy: GraphCopy,
    pub changed: bool,
}

/// One level of a graph tab's stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphLevel {
    /// A graph asset, shown as a pose graph; `state` names the state of the machine above that plays it.
    Graph { asset: AssetId, state: Option<Name> },
    /// A state machine node of the graph above, shown as a state diagram and edited in that graph's working copy.
    Machine { node: GraphNodeId },
}

struct CreatingState {
    address: String,
    name: Name,
}

struct WriteBack {
    applied: bool,
    refused: Option<String>,
}

/// What a graph's tab holds: the stack from its asset down to what is in view, a working copy per graph, and the selection.
#[derive(Component, Default)]
pub struct GraphDocument {
    graphs: Vec<HeldGraph>,
    levels: Vec<GraphLevel>,
    pub selection: Option<GraphSelection>,
    state: Option<StateValue>,
    transition: Option<TransitionValue>,
    creating: Vec<CreatingState>,
}

impl GraphDocument {
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

    fn shown_asset(&self) -> Option<AssetId> {
        self.levels.iter().rev().find_map(|level| match level {
            GraphLevel::Graph { asset, .. } => Some(*asset),
            GraphLevel::Machine { .. } => None,
        })
    }

    fn shown_machine(&self) -> Option<GraphNodeId> {
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
                    .map_or_else(String::new, |held| held.asset.name().to_owned()),
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

    fn mirror(&mut self) {
        let machine = self.machine();
        let state = match (self.selection, machine) {
            (Some(GraphSelection::State(name)), Some(machine)) => state_value(machine, name),
            _ => None,
        };
        let transition = match (self.selection, machine) {
            (Some(GraphSelection::Transition(named)), Some(machine)) => {
                transition_value(machine, named)
            }
            _ => None,
        };
        self.state = state;
        self.transition = transition;
    }

    fn show(&mut self, root: HeldGraph) {
        self.levels = vec![GraphLevel::Graph {
            asset: root.asset.id,
            state: None,
        }];
        self.graphs = vec![root];
        self.selection = None;
        self.creating.clear();
    }

    fn up(&mut self, count: usize) -> Result<GraphChange, String> {
        if self.levels.len() < 2 {
            return Err("The tab's own graph is in view, so there is no level above it".to_owned());
        }
        let kept = self.levels.len().saturating_sub(count).max(1);
        self.levels.truncate(kept);
        self.selection = None;
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

    fn write_back(&mut self) -> WriteBack {
        let mut outcome = WriteBack {
            applied: false,
            refused: None,
        };
        let (selection, state, transition) =
            (self.selection, self.state.clone(), self.transition.clone());
        let Some(machine) = self.machine_mut() else {
            return outcome;
        };
        match selection {
            Some(GraphSelection::State(old)) => {
                let Some(value) = state else {
                    return outcome;
                };
                let mut name = old;
                if value.name != old {
                    let renamed = if value.name.as_str().is_empty() {
                        Err("Give the state a name".to_owned())
                    } else {
                        machine.rename_state(old, value.name).map_err(sentence)
                    };
                    match renamed {
                        Ok(()) => {
                            name = value.name;
                            outcome.applied = true;
                        }
                        Err(reason) => outcome.refused = Some(reason),
                    }
                }
                let held = machine
                    .states()
                    .find(|state| state.name == name)
                    .map(|state| state.graph.id());
                if held.is_some_and(|held| held != value.graph.id())
                    && machine.set_state_graph(name, value.graph.clone()).is_ok()
                {
                    outcome.applied = true;
                }
                if value.initial
                    && machine.initial_state() != Some(name)
                    && machine.set_initial(name).is_ok()
                {
                    outcome.applied = true;
                }
                self.selection = Some(GraphSelection::State(name));
            }
            Some(GraphSelection::Transition(named)) => {
                let Some(value) = transition else {
                    return outcome;
                };
                let written = place_of(machine, named).and_then(|place| {
                    machine
                        .set_transition(named.from, place, value.trigger, value.blend_time)
                        .map_err(sentence)
                });
                match written {
                    Ok(()) => outcome.applied = true,
                    Err(reason) => outcome.refused = Some(reason),
                }
            }
            _ => return outcome,
        }
        self.mirror();
        if outcome.applied
            && let Some(held) = self.held_mut()
        {
            held.changed = true;
        }
        outcome
    }
}

struct Opened {
    asset: AssetId,
    state: Option<Name>,
    unset: String,
}

struct Sources<'a> {
    types: Option<&'a AppTypeRegistry>,
    assets: &'a AssetRegistry,
    root: Option<&'a Path>,
}

/// What a graph tab drills into: a node that plays a graph inside itself, or a state of the machine in view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenTarget {
    Node(GraphNodeId),
    State(Name),
}

/// One edit of what a graph tab has in view, or a move through its stack.
#[derive(Clone, Debug, PartialEq)]
pub enum GraphEdit {
    /// Adds a node of the type named by its short name or full path, loose or as an input of `parent`.
    AddNode {
        type_name: String,
        parent: Option<GraphNodeId>,
        slot: Option<usize>,
    },
    /// Adds a node of the type named by its short name or full path and makes `child` its first input.
    AddParent {
        type_name: String,
        child: GraphNodeId,
    },
    /// Makes `child` an input of `parent`, at `slot` or at its first empty slot.
    Connect {
        child: GraphNodeId,
        parent: GraphNodeId,
        slot: Option<usize>,
    },
    /// Removes a node, leaving its inputs loose.
    RemoveNode(GraphNodeId),
    /// Takes `child` out of its parent's inputs.
    RemoveLink { child: GraphNodeId },
    /// Adds a state playing `graph`, or without one a new graph asset named as the state, created beside the graph in view.
    AddState { name: Name, graph: Option<AssetId> },
    /// Adds a transition with an `Instant` trigger and no blend time, and selects it; `from` may equal `to`.
    AddTransition { from: Name, to: Name },
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
pub struct EditGraph {
    pub document: Entity,
    pub edit: GraphEdit,
}

impl Signal for EditGraph {}

/// What an applied [`GraphEdit`] changed: the node it added, the slot a connection took, the input it displaced, the transition it added, and the graph a new state plays.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GraphChange {
    pub node: Option<GraphNodeId>,
    pub slot: Option<usize>,
    pub loose: Option<GraphNodeId>,
    pub transition: Option<TransitionRef>,
    pub graph: Option<AssetId>,
}

/// The outcome of one [`EditGraph`]; a refusal carries the reason the tab's status shows.
#[derive(Event, Clone, Debug, PartialEq)]
pub struct GraphEdited {
    pub document: Entity,
    pub edit: GraphEdit,
    pub result: Result<GraphChange, String>,
}

/// Opens an `AnimationGraph` in a tab that shows its nodes on a canvas.
#[derive(Default)]
pub struct AnimationGraphEditor;

impl AssetEditor for AnimationGraphEditor {
    fn build(&self, editor: &mut EntityCommandQueue) {
        editor.insert((GraphDocument::default(), InspectedValues::default()));
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

/// What a graph's preview adds to the game's recipe: the types of the `State` and `Transition` values.
pub fn graph_preview(app: &mut App) {
    app.register_type::<StateValue>()
        .register_type::<TransitionValue>()
        .register_type::<AnimationFSMTrigger>();
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

fn mirrored_state(document: Entity) -> Arc<dyn PropertyTarget> {
    Arc::new(DocumentValue::<GraphDocument> {
        name: STATE.to_owned(),
        document,
        read: Box::new(|state| state.state.as_ref().map(|value| value as &dyn Reflect)),
        edit: Box::new(|state| state.state.as_mut().map(|value| value as &mut dyn Reflect)),
    })
}

fn mirrored_transition(document: Entity) -> Arc<dyn PropertyTarget> {
    Arc::new(DocumentValue::<GraphDocument> {
        name: TRANSITION.to_owned(),
        document,
        read: Box::new(|state| state.transition.as_ref().map(|value| value as &dyn Reflect)),
        edit: Box::new(|state| {
            state
                .transition
                .as_mut()
                .map(|value| value as &mut dyn Reflect)
        }),
    })
}

fn publish_selection(document: Entity, state: &mut GraphDocument, cmd: &mut CommandQueue) {
    state.mirror();
    let value = match state.selection {
        Some(GraphSelection::Node(id)) => state
            .graph()
            .and_then(|copy| copy.node(id))
            .and_then(|node| node.live())
            .map(|_| node_value(document, id)),
        Some(GraphSelection::State(_)) => state.state.as_ref().map(|_| mirrored_state(document)),
        Some(GraphSelection::Transition(_)) => state
            .transition
            .as_ref()
            .map(|_| mirrored_transition(document)),
        Some(GraphSelection::Link { .. }) | None => None,
    };
    cmd.insert(InspectedValues(value.into_iter().collect()), document);
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
    documents: Query<(&mut EditorDocument, &mut GraphDocument, Option<&HostedApp>)>,
    assets: Res<AssetRegistry>,
    project: Res<ProjectState>,
    mut edited: EventWriter<GraphEdited>,
    mut cmd: CommandQueue,
) {
    let EditGraph { document, edit } = on.signal();
    let Some((mut editor, mut state, hosted)) = documents.get_entity(*document) else {
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
    documents: Query<(Entity, &mut EditorDocument, &mut GraphDocument)>,
    mut edited: EventWriter<GraphEdited>,
    mut cmd: CommandQueue,
) {
    for creation in created.read() {
        for (document, mut editor, mut state) in documents.iter() {
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
    documents: Query<(
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
    for (entity, mut document, mut state, hosted, failed) in documents.iter() {
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
    documents: Query<(&EditorDocument, &mut GraphDocument, Option<&HostedApp>)>,
    project: Res<ProjectState>,
    mut cmd: CommandQueue,
) {
    let document = on.signal().0;
    let Some((editor, mut state, hosted)) = documents.get_entity(document) else {
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
    documents: Query<(&mut EditorDocument, &mut GraphDocument)>,
    mut cmd: CommandQueue,
) {
    for hosted in committed.read() {
        if hosted.event.result.is_err() {
            continue;
        }
        let Some((mut editor, mut state)) = documents.get_entity(hosted.document) else {
            continue;
        };
        let applied = match state.selection {
            Some(GraphSelection::State(_) | GraphSelection::Transition(_)) => {
                let outcome = state.write_back();
                if let Some(reason) = outcome.refused {
                    editor.status = reason;
                    cmd.trigger(RefreshInspector);
                }
                outcome.applied
            }
            _ => {
                if let Some(held) = state.held_mut() {
                    held.changed = true;
                }
                true
            }
        };
        if applied {
            cmd.insert(Unsaved, hosted.document);
        }
    }
}

pub(crate) fn clear_status_on_edit(
    on: On<Add<Unsaved>>,
    documents: Query<(&mut EditorDocument, &GraphDocument)>,
) {
    if let Some((mut document, state)) = documents.get_entity(on.signal().entity) {
        let status = state.selected_warning().unwrap_or_default();
        if document.status != status {
            document.status = status;
        }
    }
}
