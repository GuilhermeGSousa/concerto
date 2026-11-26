use std::time::Duration;

use concerto_animation::{
    graph::AnimationGraph,
    message::{SendReflectedMessage, animation_message_types, decode_animation_message},
    node::state_machine::AnimationStateMachine,
    player::AnimationPlayer,
};
use concerto_app::{App, schedule_groups::LateUpdate};
use concerto_ecs::{
    Component, Entity, IntoSystemConfig, Query, Res, With, command::CommandQueue,
    events::event_reader::EventReader, reflect::AppTypeRegistry,
};
use concerto_editor::{
    asset_editor::{ActiveEditor, EditorDocument},
    hosted::{EditTarget, EditWorld, EditWorldSignals, Hosted, HostedApp, PlayState},
    inspector::WriteValue,
    preview::PreviewSignals,
    project::ProjectState,
    tools::edits::PropertyCommitted,
};
use concerto_foundation::assets::{AssetId, asset_store::AssetStore, registry::AssetRegistry};
use concerto_mcp::{
    AppMcpExt, McpServe, McpTool, Needs, PendingCall, Reply, ToolCalls, ToolError, ToolRegistry,
    answer_unanswered,
};
use concerto_name::Name;
use concerto_reflect::{Reflect, ReflectedValue, TypeRegistry, to_json};
use schemars::JsonSchema;
use serde::Deserialize;

use super::{
    copy::{GraphCopy, GraphNodeId},
    document::{
        EditGraph, GraphChange, GraphDocument, GraphEdit, GraphEdited, GraphSelection, OpenTarget,
        Sources, TransitionRef, shown_transitions,
    },
    panels::{subtitle, time_text, weight_text},
    params::{FoundParam, short_type},
    preview::{GraphPreviewState, MachineView, PreviewNode, SendGraphMessage},
    stage::{GraphLive, LiveView, SentPreview},
};

/// Reads what the front animation graph tab shows: the breadcrumb, then for a pose graph one line per node, `id · type · parent#slot · subtitle`, with the result node and loose nodes marked, and for a state machine one line per state, `name · initial · graph`, then one per transition, `from → to #index · trigger · blend`. When the graph names parameters, a `parameters:` line follows, then one line per parameter, `name · type · value`. Once the preview has reported what it plays, a `live:` line follows: for a pose graph one line per playing node, `id · weight · time` for a clip and `in state · from → to progress` for a state machine, and for a state machine that one part alone. A node that describes itself, such as a slot with its playing entries, has its indented `describe` line under its own.
#[derive(Deserialize, JsonSchema)]
pub struct GraphInspect {}

/// Adds a node to the pose graph the front tab shows and answers its id.
#[derive(Deserialize, JsonSchema)]
pub struct GraphAddNode {
    /// The node's type, by short name such as LayerNode, or by full path when two types share one.
    pub r#type: String,
    /// The node to make it an input of, by its id in graph_inspect; loose without one.
    pub parent: Option<u32>,
    /// The parent's input slot, from 0; its first empty slot without one.
    pub slot: Option<usize>,
    /// A node to make the new node's first input, as dropping its link on empty space does; give it instead of parent and slot. A type that takes no inputs is refused and adds nothing.
    pub child: Option<u32>,
}

/// Adds a state to the state machine the front tab shows; the first state added is the initial one.
#[derive(Deserialize, JsonSchema)]
pub struct GraphAddState {
    /// The state's name. Without `graph`, a new AnimationGraph of this name is created beside the graph that holds the machine.
    pub name: String,
    /// An existing AnimationGraph for the state to play, by address or id; none is created then.
    pub graph: Option<String>,
}

/// In a pose graph, makes node `child` an input of node `parent`, moving it if it already is one. In a state machine, adds a transition `from` one state `to` another, or to itself, with an Instant trigger and no blend time, and selects it.
#[derive(Deserialize, JsonSchema)]
pub struct GraphConnect {
    /// The node that becomes an input, by its id in graph_inspect.
    pub child: Option<u32>,
    /// The node that takes it.
    pub parent: Option<u32>,
    /// The parent's input slot, from 0: a taken slot of a node with named slots is replaced, and its input left loose. The first empty slot without one.
    pub slot: Option<usize>,
    /// The state a transition leaves, by name.
    pub from: Option<String>,
    /// The state a transition enters, by name; the same as `from` for a transition to itself.
    pub to: Option<String>,
}

/// Removes one thing from what the front animation graph tab shows: a node, leaving its inputs loose, or a link, or in a state machine a state with its transitions, whose graph asset stays, or a transition.
#[derive(Deserialize, JsonSchema)]
pub struct GraphRemove {
    /// The node to remove, by its id in graph_inspect.
    pub node: Option<u32>,
    /// The link to remove, named by the node it makes an input.
    pub link: Option<u32>,
    /// The state to remove, by name.
    pub state: Option<String>,
    /// The state the transition to remove leaves.
    pub from: Option<String>,
    /// The state the transition to remove enters.
    pub to: Option<String>,
    /// The transition's index among those between that pair, as graph_inspect shows after `#`; 0 without one.
    pub index: Option<usize>,
}

/// Selects a node, a state or a transition of what the front animation graph tab shows, or nothing, publishing it as the value Node, State or Transition that inspect, set_property, list_push and list_remove take as target. A state machine node has no Node value: graph_open shows its states and transitions.
#[derive(Deserialize, JsonSchema)]
pub struct GraphSelect {
    /// The node to select, by its id in graph_inspect.
    pub node: Option<u32>,
    /// The state to select, by name.
    pub state: Option<String>,
    /// The state the transition to select leaves.
    pub from: Option<String>,
    /// The state the transition to select enters.
    pub to: Option<String>,
    /// The transition's index among those between that pair; 0 without one.
    pub index: Option<usize>,
}

/// Drills into a node of the pose graph the front tab shows, a state machine into its states and any other node into the graph it plays, or into the graph of a state of the state machine in view.
#[derive(Deserialize, JsonSchema)]
pub struct GraphOpen {
    /// The node to open, by its id in graph_inspect.
    pub node: Option<u32>,
    /// The state to open, by name.
    pub state: Option<String>,
}

/// Goes up one level in the front animation graph tab, from a state's graph to its state machine, or from a state machine or a nested graph to the graph that holds it.
#[derive(Deserialize, JsonSchema)]
pub struct GraphUp {}

/// Sets a parameter of the graph the front animation graph tab shows, on the blackboard of the preview's players, and answers with the `live:` block of graph_inspect as the preview next reports it, or says that no preview is playing. graph_inspect lists the parameters with their types and values. Never saved.
#[derive(Deserialize, JsonSchema)]
pub struct GraphSetParam {
    /// The parameter's name, as graph_inspect lists it.
    pub name: String,
    /// The value, as JSON of the parameter's type: true, 0.5, [1.0, 0.0].
    pub value: serde_json::Value,
    /// The parameter's type, as graph_inspect shows it, when the graph reads the name as several types.
    pub r#type: Option<String>,
}

/// Sends a message to a character of the playing game, or without `entity` to the characters the front animation graph tab's preview plays, as the game's code does with `SendAnimationMessage`, and answers after the next frame with the states the graphs are in and what each slot plays. A type that is not registered with `register_animation_message` is refused with the registered ones. Never saved.
#[derive(Deserialize, JsonSchema)]
pub struct SendCharacterMessage {
    /// The character of the playing game to send to, by id as scene_tree shows it, such as 42v3; it carries the `AnimationPlayer`. Without it the message goes to the characters the front graph tab's preview plays.
    pub entity: Option<String>,
    /// The message: `type` is its full type path as the refusal lists them, `value` its fields as the inspector shows them, such as { "strength": 1.0 }.
    pub message: MessageArguments,
}

/// A message by its type and value.
#[derive(Deserialize, JsonSchema)]
pub struct MessageArguments {
    pub r#type: String,
    #[serde(default = "no_fields")]
    pub value: serde_json::Value,
}

fn no_fields() -> serde_json::Value {
    serde_json::json!({})
}

#[derive(Component)]
struct SendingToCharacter {
    character: Entity,
    type_path: String,
    runs: usize,
}

#[derive(Component)]
struct SendingMessage {
    document: Entity,
    type_path: String,
    reports: usize,
}

const MESSAGE_PATIENCE: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, PartialEq)]
enum ParamStep {
    Committing { waited: bool },
    Sending,
    Playing,
}

#[derive(Component)]
struct SettingParam {
    document: Entity,
    label: &'static str,
    answer: String,
    step: ParamStep,
}

const PAUSED: &str = "The preview is paused, so no live state follows; set playing on the Preview value to see the graph play.";
const NOT_PLAYING: &str = "No preview is playing the graph, so no live state follows; set scene on the Preview value to a scene with a skeleton.";

#[derive(Component)]
struct EditingGraph {
    document: Entity,
    edit: GraphEdit,
}

pub(super) fn build(app: &mut App) {
    if app.get_resource::<ToolRegistry>().is_none() {
        return;
    }
    app.add_tool::<GraphInspect, _>(
        McpTool::read_only("graph_inspect").needs(Needs::Project),
        graph_inspect,
    )
    .add_tool::<GraphAddNode, _>(
        McpTool::new("graph_add_node").needs(Needs::Project),
        graph_add_node,
    )
    .add_tool::<GraphConnect, _>(
        McpTool::new("graph_connect").needs(Needs::Project),
        graph_connect,
    )
    .add_tool::<GraphRemove, _>(
        McpTool::new("graph_remove").needs(Needs::Project),
        graph_remove,
    )
    .add_tool::<GraphSelect, _>(
        McpTool::new("graph_select").needs(Needs::Project),
        graph_select,
    )
    .add_tool::<GraphAddState, _>(
        McpTool::new("graph_add_state").needs(Needs::Project),
        graph_add_state,
    )
    .add_tool::<GraphOpen, _>(McpTool::new("graph_open").needs(Needs::Project), graph_open)
    .add_tool::<GraphUp, _>(McpTool::new("graph_up").needs(Needs::Project), graph_up)
    .add_tool::<GraphSetParam, _>(
        McpTool::new("graph_set_param").needs(Needs::Project),
        graph_set_param,
    )
    .add_tool::<SendCharacterMessage, _>(
        McpTool::new("send_animation_message").needs(Needs::Project),
        send_animation_message,
    )
    .add_system(LateUpdate, answer_graph_edits)
    .add_system(LateUpdate, answer_param_sets)
    .add_system(LateUpdate, answer_message_sends)
    .add_system(McpServe, send_to_character.before(answer_unanswered))
    .add_system(McpServe, refuse_without_game.before(answer_unanswered))
    .add_system(LateUpdate, answer_character_sends);
}

fn refused(reason: impl Into<String>) -> ToolError {
    ToolError::new("refused", reason)
}

fn no_graph_tab() -> ToolError {
    refused(
        "The front document is not an animation graph. Open one with open_asset; list_assets kind=AnimationGraph lists them, and create_asset makes one.",
    )
}

fn front_graph(active: &ActiveEditor, graphs: &Query<&GraphDocument>) -> Option<Entity> {
    active
        .0
        .filter(|document| graphs.get_entity(*document).is_some())
}

fn start(
    reply: &Reply,
    edit: GraphEdit,
    active: &ActiveEditor,
    graphs: &Query<&GraphDocument>,
    cmd: &mut CommandQueue,
) {
    let Some(document) = front_graph(active, graphs) else {
        reply.err(no_graph_tab());
        return;
    };
    cmd.trigger(EditGraph {
        document,
        edit: edit.clone(),
    });
    cmd.spawn((
        PendingCall::new(reply, "the graph edit"),
        EditingGraph { document, edit },
    ));
}

fn graph_add_node(
    mut calls: ToolCalls<GraphAddNode>,
    active: Res<ActiveEditor>,
    graphs: Query<&GraphDocument>,
    mut cmd: CommandQueue,
) {
    for (arguments, reply) in calls.read() {
        let edit = match arguments.child {
            Some(_) if arguments.parent.is_some() || arguments.slot.is_some() => {
                reply.err(invalid(
                    "Give child, the node that becomes the new node's first input, or parent and slot, not both.",
                ));
                continue;
            }
            Some(child) => GraphEdit::AddParent {
                type_name: arguments.r#type,
                child: GraphNodeId(child),
            },
            None => GraphEdit::AddNode {
                type_name: arguments.r#type,
                parent: arguments.parent.map(GraphNodeId),
                slot: arguments.slot,
            },
        };
        start(reply, edit, &active, &graphs, &mut cmd);
    }
}

fn invalid(reason: &str) -> ToolError {
    ToolError::new("invalid_arguments", reason)
}

struct Named {
    node: Option<u32>,
    link: Option<u32>,
    state: Option<String>,
    from: Option<String>,
    to: Option<String>,
    index: Option<usize>,
}

enum Target {
    Node(GraphNodeId),
    Link(GraphNodeId),
    State(Name),
    Transition(TransitionRef),
    Nothing,
}

fn target(named: Named) -> Result<Target, ToolError> {
    let transition = named.from.is_some() || named.to.is_some() || named.index.is_some();
    let given = [
        named.node.is_some(),
        named.link.is_some(),
        named.state.is_some(),
        transition,
    ];
    if given.iter().filter(|given| **given).count() > 1 {
        return Err(invalid(
            "Name one thing: a node, a link, a state, or a transition by from and to.",
        ));
    }
    if let Some(node) = named.node {
        return Ok(Target::Node(GraphNodeId(node)));
    }
    if let Some(link) = named.link {
        return Ok(Target::Link(GraphNodeId(link)));
    }
    if let Some(state) = named.state {
        return Ok(Target::State(Name::new(&state)));
    }
    match (named.from, named.to) {
        (Some(from), Some(to)) => Ok(Target::Transition(TransitionRef {
            from: Name::new(&from),
            to: Name::new(&to),
            index: named.index.unwrap_or(0),
        })),
        (None, None) if !transition => Ok(Target::Nothing),
        _ => Err(invalid(
            "A transition is named by both from and to, and index when several join that pair.",
        )),
    }
}

fn graph_connect(
    mut calls: ToolCalls<GraphConnect>,
    active: Res<ActiveEditor>,
    graphs: Query<&GraphDocument>,
    mut cmd: CommandQueue,
) {
    for (arguments, reply) in calls.read() {
        let edit = match (
            arguments.child,
            arguments.parent,
            arguments.from,
            arguments.to,
        ) {
            (Some(child), Some(parent), None, None) => GraphEdit::Connect {
                child: GraphNodeId(child),
                parent: GraphNodeId(parent),
                slot: arguments.slot,
            },
            (None, None, Some(from), Some(to)) => GraphEdit::AddTransition {
                from: Name::new(&from),
                to: Name::new(&to),
            },
            _ => {
                reply.err(invalid(
                    "Give child and parent to wire a pose graph, or from and to to add a transition to a state machine.",
                ));
                continue;
            }
        };
        start(reply, edit, &active, &graphs, &mut cmd);
    }
}

fn graph_remove(
    mut calls: ToolCalls<GraphRemove>,
    active: Res<ActiveEditor>,
    graphs: Query<&GraphDocument>,
    mut cmd: CommandQueue,
) {
    for (arguments, reply) in calls.read() {
        let named = Named {
            node: arguments.node,
            link: arguments.link,
            state: arguments.state,
            from: arguments.from,
            to: arguments.to,
            index: arguments.index,
        };
        let edit = match target(named) {
            Ok(Target::Node(node)) => GraphEdit::RemoveNode(node),
            Ok(Target::Link(child)) => GraphEdit::RemoveLink { child },
            Ok(Target::State(state)) => GraphEdit::RemoveState(state),
            Ok(Target::Transition(transition)) => GraphEdit::RemoveTransition(transition),
            Ok(Target::Nothing) => {
                reply.err(invalid(
                    "Give node, or link, the node the link makes an input, or state, or from and to for a transition.",
                ));
                continue;
            }
            Err(error) => {
                reply.err(error);
                continue;
            }
        };
        start(reply, edit, &active, &graphs, &mut cmd);
    }
}

fn graph_select(
    mut calls: ToolCalls<GraphSelect>,
    active: Res<ActiveEditor>,
    graphs: Query<&GraphDocument>,
    mut cmd: CommandQueue,
) {
    for (arguments, reply) in calls.read() {
        let named = Named {
            node: arguments.node,
            link: None,
            state: arguments.state,
            from: arguments.from,
            to: arguments.to,
            index: arguments.index,
        };
        let selection = match target(named) {
            Ok(Target::Node(node)) => Some(GraphSelection::Node(node)),
            Ok(Target::Link(child)) => Some(GraphSelection::Link { child }),
            Ok(Target::State(state)) => Some(GraphSelection::State(state)),
            Ok(Target::Transition(transition)) => Some(GraphSelection::Transition(transition)),
            Ok(Target::Nothing) => None,
            Err(error) => {
                reply.err(error);
                continue;
            }
        };
        start(
            reply,
            GraphEdit::Select(selection),
            &active,
            &graphs,
            &mut cmd,
        );
    }
}

fn graph_add_state(
    mut calls: ToolCalls<GraphAddState>,
    active: Res<ActiveEditor>,
    graphs: Query<&GraphDocument>,
    assets: Res<AssetRegistry>,
    mut cmd: CommandQueue,
) {
    for (arguments, reply) in calls.read() {
        let graph = match arguments.graph.as_deref() {
            None => None,
            Some(named) => {
                let found = assets
                    .id_for_address(named)
                    .or_else(|| AssetId::from_simple_hex(named).ok());
                let Some(found) = found else {
                    reply.err(ToolError::new(
                        "unknown_asset",
                        format!(
                            "No asset `{named}` in the project; list_assets kind=AnimationGraph shows addresses and ids."
                        ),
                    ));
                    continue;
                };
                Some(found)
            }
        };
        let edit = GraphEdit::AddState {
            name: Name::new(&arguments.name),
            graph,
        };
        start(reply, edit, &active, &graphs, &mut cmd);
    }
}

fn graph_open(
    mut calls: ToolCalls<GraphOpen>,
    active: Res<ActiveEditor>,
    graphs: Query<&GraphDocument>,
    mut cmd: CommandQueue,
) {
    for (arguments, reply) in calls.read() {
        let target = match (arguments.node, arguments.state) {
            (Some(node), None) => OpenTarget::Node(GraphNodeId(node)),
            (None, Some(state)) => OpenTarget::State(Name::new(&state)),
            _ => {
                reply.err(invalid(
                    "Give node, a node of the pose graph in view, or state, a state of the state machine in view, and not both.",
                ));
                continue;
            }
        };
        start(reply, GraphEdit::Open(target), &active, &graphs, &mut cmd);
    }
}

fn graph_up(
    mut calls: ToolCalls<GraphUp>,
    active: Res<ActiveEditor>,
    graphs: Query<&GraphDocument>,
    mut cmd: CommandQueue,
) {
    for (_, reply) in calls.read() {
        start(reply, GraphEdit::Up(1), &active, &graphs, &mut cmd);
    }
}

fn kind(copy: Option<&GraphCopy>, id: GraphNodeId) -> String {
    copy.and_then(|copy| copy.node(id))
        .map_or_else(|| "a node".to_owned(), |node| node.described())
}

fn answer(
    edit: &GraphEdit,
    change: &GraphChange,
    state: Option<&GraphDocument>,
    assets: &AssetRegistry,
) -> String {
    let copy = state.and_then(GraphDocument::graph);
    let displaced = change.loose.map_or_else(String::new, |loose| {
        format!("; node {loose}, which was there, is loose")
    });
    match edit {
        GraphEdit::AddNode { parent, .. } => {
            let node = change.node.unwrap_or(GraphNodeId(0));
            let place = match (parent, change.slot) {
                (Some(parent), Some(slot)) => format!("as input {slot} of node {parent}"),
                _ => "loose".to_owned(),
            };
            format!(
                "Added node {node}, {}, {place}{displaced}.",
                kind(copy, node)
            )
        }
        GraphEdit::AddParent { child, .. } => {
            let node = change.node.unwrap_or(GraphNodeId(0));
            format!(
                "Added node {node}, {}, with node {child} as input {}.",
                kind(copy, node),
                change.slot.unwrap_or(0)
            )
        }
        GraphEdit::Connect { child, parent, .. } => format!(
            "Node {child} is input {} of node {parent}{displaced}.",
            change.slot.unwrap_or(0)
        ),
        GraphEdit::RemoveNode(node) => format!("Removed node {node}."),
        GraphEdit::RemoveLink { child } => format!("Node {child} is loose."),
        GraphEdit::AddState { name, .. } => {
            let initial = state
                .and_then(GraphDocument::machine)
                .is_some_and(|machine| machine.initial_state() == Some(*name));
            let graph = change
                .graph
                .and_then(|graph| assets.get(graph))
                .map_or("a graph", |entry| entry.address.as_str());
            let initial = if initial { ", the initial state" } else { "" };
            format!("Added state {name}{initial}, playing {graph}.")
        }
        GraphEdit::AddTransition { from, to } => match change.transition {
            Some(transition) => format!("Added transition {transition}."),
            None => format!("Added a transition {from} → {to}."),
        },
        GraphEdit::RemoveState(name) => format!("Removed state {name} and its transitions."),
        GraphEdit::RemoveTransition(transition) => format!("Removed transition {transition}."),
        GraphEdit::Open(_) | GraphEdit::Up(_) => {
            let shown = state.map_or_else(Vec::new, GraphDocument::breadcrumb);
            format!("Showing {}.", shown.join(" › "))
        }
        GraphEdit::Select(Some(GraphSelection::Node(node))) => {
            let live = copy
                .and_then(|copy| copy.node(*node))
                .is_some_and(|node| node.live().is_some());
            if state.is_some_and(|state| state.is_state_machine(*node)) {
                format!(
                    "Selected node {node}, {}; its states and transitions are edited in its diagram, which graph_open node={node} shows, so it has no Node value.",
                    kind(copy, *node)
                )
            } else if live {
                format!(
                    "Selected node {node}, {}; inspect and set_property take it as target Node.",
                    kind(copy, *node)
                )
            } else {
                format!("Selected node {node}; it could not be built, so it has no value to edit.")
            }
        }
        GraphEdit::Select(Some(GraphSelection::Link { child })) => {
            format!("Selected the link of node {child}.")
        }
        GraphEdit::Select(Some(GraphSelection::State(name))) => {
            format!("Selected state {name}; inspect and set_property take it as target State.")
        }
        GraphEdit::Select(Some(GraphSelection::Transition(transition))) => format!(
            "Selected transition {transition}; inspect and set_property take it as target Transition."
        ),
        GraphEdit::Select(None) => "Nothing is selected.".to_owned(),
    }
}

fn answer_graph_edits(
    mut edited: EventReader<GraphEdited>,
    waiting: Query<(Entity, &PendingCall, &EditingGraph)>,
    graphs: Query<&GraphDocument>,
    assets: Res<AssetRegistry>,
    mut cmd: CommandQueue,
) {
    for event in edited.read() {
        for (call, pending, editing) in waiting.iter() {
            if editing.document != event.document || editing.edit != event.edit {
                continue;
            }
            let state = graphs.get_entity(event.document);
            match &event.result {
                Ok(change) => pending
                    .reply
                    .ok(answer(&event.edit, change, state, &assets)),
                Err(reason) => pending.reply.err(refused(format!("{reason}."))),
            }
            cmd.despawn(call);
        }
    }
}

fn state_lines(machine: &AnimationStateMachine, assets: &AssetRegistry, lines: &mut Vec<String>) {
    let initial = machine.initial_state();
    for state in machine.states() {
        let graph = if state.graph.id().is_nil() {
            "no graph".to_owned()
        } else {
            assets
                .get(state.graph.id())
                .map_or_else(|| "missing graph".to_owned(), |entry| entry.address.clone())
        };
        let mark = if initial == Some(state.name) {
            " · initial"
        } else {
            ""
        };
        lines.push(format!("{}{mark} · {graph}", state.name));
    }
    for shown in shown_transitions(machine) {
        lines.push(format!(
            "{} · {} · {}s",
            shown.named, shown.trigger, shown.blend_time
        ));
    }
}

/// One parameter as graph_inspect and the Parameters section show it: `name · type · value`.
pub(super) fn param_line(
    name: Name,
    type_name: &str,
    value: &dyn Reflect,
    shared: bool,
    types: &TypeRegistry,
) -> String {
    let mut line = format!(
        "{name} · {} · {}",
        short_type(type_name),
        to_json(value, types)
    );
    if shared {
        line.push_str(" · read as several types");
    }
    line
}

fn param_lines(
    state: &GraphDocument,
    found: &[FoundParam],
    types: &TypeRegistry,
    lines: &mut Vec<String>,
) {
    if found.is_empty() {
        return;
    }
    lines.push("parameters:".to_owned());
    for param in found {
        let default;
        let value = match state.parameters.held(param) {
            Some(held) => held.value(),
            None => {
                default = param.data.default_value();
                default.as_ref()
            }
        };
        let shared = found
            .iter()
            .filter(|other| other.name == param.name)
            .count()
            > 1;
        lines.push(param_line(
            param.name,
            param.data.value_type_name(),
            value,
            shared,
            types,
        ));
    }
}

fn typed(found: &[FoundParam]) -> String {
    found
        .iter()
        .map(|param| {
            format!(
                "{} ({})",
                param.name,
                short_type(param.data.value_type_name())
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn graph_set_param(
    mut calls: ToolCalls<GraphSetParam>,
    active: Res<ActiveEditor>,
    mut documents: Query<(&mut GraphDocument, Option<&HostedApp>)>,
    assets: Res<AssetRegistry>,
    project: Res<ProjectState>,
    mut cmd: CommandQueue,
) {
    for (arguments, reply) in calls.read() {
        let front = active
            .0
            .and_then(|document| Some((document, documents.get_entity_mut(document)?)));
        let Some((document, (mut state, hosted))) = front else {
            reply.err(no_graph_tab());
            continue;
        };
        let Some(types) = hosted.and_then(HostedApp::type_registry) else {
            reply.err(refused(
                "The preview is not running, so there is no parameter to set; call status until it says busy: no.",
            ));
            continue;
        };
        let found = state.found_params(&Sources {
            types: Some(&types),
            assets: &assets,
            root: project
                .project
                .as_ref()
                .map(|project| project.root.as_path()),
        });
        if !state.parameters.shows(&found) {
            state.parameters.show(&found);
        }
        let named: Vec<&FoundParam> = found
            .iter()
            .filter(|param| param.name.as_str() == arguments.name)
            .filter(|param| {
                arguments.r#type.as_deref().is_none_or(|wanted| {
                    let type_name = param.data.value_type_name();
                    wanted == type_name || wanted == short_type(type_name)
                })
            })
            .collect();
        let label = match named[..] {
            [param] => state.parameters.held(param).map(|held| held.label()),
            [] if found.is_empty() => {
                reply.err(refused(format!(
                    "There is no parameter {}; the graph in view names no parameters.",
                    arguments.name
                )));
                continue;
            }
            [] => {
                reply.err(refused(format!(
                    "There is no parameter {}; the parameters are {}.",
                    arguments.name,
                    typed(&found)
                )));
                continue;
            }
            _ => {
                let types: Vec<&str> = named
                    .iter()
                    .map(|param| short_type(param.data.value_type_name()))
                    .collect();
                reply.err(refused(format!(
                    "{} is read as several types, {}; give type.",
                    arguments.name,
                    types.join(" and ")
                )));
                continue;
            }
        };
        let (Some(label), Some(target)) = (label, state.parameters_value()) else {
            reply.err(refused("The tab publishes no Parameters value."));
            continue;
        };
        cmd.trigger(WriteValue {
            target,
            path: label.to_owned(),
            json: arguments.value.clone(),
        });
        cmd.spawn((
            PendingCall::new(reply, "the parameter"),
            SettingParam {
                document,
                label,
                answer: format!("Set {} to {}.", arguments.name, arguments.value),
                step: ParamStep::Committing { waited: false },
            },
        ));
    }
}

fn machine_part(machine: &MachineView) -> String {
    let mut part = match machine.current {
        Some(current) => format!("in {current}"),
        None => "in no state".to_owned(),
    };
    if let Some(transition) = machine.transition {
        part.push_str(&format!(
            " · {} → {} {}",
            transition.from,
            transition.to,
            weight_text(transition.progress)
        ));
    }
    part
}

fn live_block(state: &GraphDocument, live: LiveView) -> Option<String> {
    let mut lines = Vec::new();
    if let Some(node) = state.shown_machine() {
        let machine = live.node(node)?.machine?;
        lines.push(machine_part(&machine));
    } else {
        for node in state.graph()?.nodes() {
            let Some(playing) = live.node(node.id) else {
                continue;
            };
            let mut line = format!("{} · weight {}", node.id, weight_text(playing.weight));
            let shown_inline = playing.clip_time.is_some() || playing.machine.is_some();
            if let Some(time) = playing.clip_time {
                line.push_str(&format!(" · time {}", time_text(time)));
            }
            if let Some(machine) = &playing.machine {
                line.push_str(&format!(" · {}", machine_part(machine)));
            }
            lines.push(line);
            if !shown_inline {
                lines.extend(playing.describe.as_ref().map(|line| format!("  {line}")));
            }
        }
    }
    (!lines.is_empty()).then(|| format!("live:\n{}", lines.join("\n")))
}

fn message_answer(type_path: &str, target: &str, nodes: &[PreviewNode]) -> String {
    let states: Vec<String> = nodes
        .iter()
        .filter_map(|node| node.machine?.current)
        .map(|state| state.to_string())
        .collect();
    let mut lines = vec![format!(
        "Sent {} to {target}. States: {}.",
        short_type(type_path),
        if states.is_empty() {
            "none".to_owned()
        } else {
            states.join(", ")
        }
    )];
    lines.extend(
        nodes
            .iter()
            .filter(|node| node.machine.is_none() && node.clip_time.is_none())
            .filter_map(|node| node.describe.as_ref())
            .map(|described| format!("node: {described}")),
    );
    lines.join("\n")
}

fn registered_messages(types: &TypeRegistry) -> String {
    let registered = animation_message_types(types);
    if registered.is_empty() {
        "no type is registered with register_animation_message".to_owned()
    } else {
        format!("the registered message types are {}", registered.join(", "))
    }
}

fn send_animation_message(
    mut calls: ToolCalls<SendCharacterMessage>,
    active: Res<ActiveEditor>,
    documents: Query<(&GraphDocument, Option<&HostedApp>)>,
    mut previews: PreviewSignals,
    mut cmd: CommandQueue,
) {
    for (arguments, reply) in calls.read() {
        if arguments.entity.is_some() {
            continue;
        }
        let front = active
            .0
            .and_then(|document| Some((document, documents.get_entity(document)?)));
        let Some((document, (state, hosted))) = front else {
            reply.err(no_graph_tab());
            continue;
        };
        let Some(types) = hosted.and_then(HostedApp::type_registry) else {
            reply.err(refused(
                "The preview is not running; call status until it says busy: no.",
            ));
            continue;
        };
        if !state.preview.playing {
            reply.err(refused(format!(
                "{PAUSED} A paused preview keeps no message."
            )));
            continue;
        }
        let types = types.read();
        let message = arguments.message;
        let value: ReflectedValue =
            match decode_animation_message(&types, &message.r#type, &message.value) {
                Ok(value) => value,
                Err(reason) => {
                    reply.err(refused(format!(
                        "{reason}; {}.",
                        registered_messages(&types)
                    )));
                    continue;
                }
            };
        previews.trigger(
            document,
            SendGraphMessage {
                type_path: message.r#type.clone(),
                value,
            },
        );
        let mut pending = PendingCall::new(reply, "the preview to report after the message")
            .timeout(MESSAGE_PATIENCE);
        pending.at_deadline = Some(format!(
            "Sent {}, but the preview reported no state after it; {NOT_PLAYING}",
            short_type(&message.r#type)
        ));
        cmd.spawn((
            pending,
            SendingMessage {
                document,
                type_path: message.r#type,
                reports: 0,
            },
        ));
    }
}

type Characters = (
    Query<'static, 'static, &'static AnimationPlayer>,
    Option<Res<'static, AppTypeRegistry>>,
);

type Played = (
    Query<'static, 'static, &'static AnimationPlayer>,
    Option<Res<'static, AssetStore<AnimationGraph>>>,
);

fn character_id(id: &str) -> Result<Entity, ToolError> {
    Entity::parse_id(id).ok_or_else(|| {
        ToolError::new(
            "unknown_entity",
            format!("`{id}` is not an entity id; ids look like 42v3, as scene_tree shows them while the game plays."),
        )
    })
}

fn game_not_playing() -> ToolError {
    ToolError::new(
        "not_playing",
        "The game is not playing or its scene is not in front; start it with play and bring its scene to the front, or leave out entity to send to the graph tab's preview.",
    )
}

fn refuse_without_game(
    mut calls: ToolCalls<SendCharacterMessage>,
    targets: Query<&PlayState, With<EditTarget>>,
) {
    for (arguments, reply) in calls.read() {
        let Some(id) = arguments.entity else {
            continue;
        };
        if let Err(error) = character_id(&id) {
            reply.err(error);
        } else if !targets.iter().any(|state| *state == PlayState::Playing) {
            reply.err(game_not_playing());
        }
    }
}

fn send_to_character(
    mut calls: ToolCalls<SendCharacterMessage>,
    targets: Query<&PlayState, With<EditTarget>>,
    scene: EditWorld<Characters>,
    mut signals: EditWorldSignals,
    mut cmd: CommandQueue,
) {
    let (players, types) = &*scene;
    for (arguments, reply) in calls.read() {
        let Some(id) = arguments.entity else {
            continue;
        };
        let character = match character_id(&id) {
            Ok(character) => character,
            Err(error) => {
                reply.err(error);
                continue;
            }
        };
        if !targets.iter().any(|state| *state == PlayState::Playing) {
            reply.err(game_not_playing());
            continue;
        }
        if players.get_entity(character).is_none() {
            reply.err(ToolError::new(
                "no_player",
                format!("{id} has no AnimationPlayer, or no entity {id} is in the playing game; send to the entity that carries the AnimationPlayer, which scene_tree shows."),
            ));
            continue;
        }
        let Some(types) = types else {
            reply.err(refused("The playing game has no type registry."));
            continue;
        };
        let types = types.read();
        let message = arguments.message;
        let value = match decode_animation_message(&types, &message.r#type, &message.value) {
            Ok(value) => value,
            Err(reason) => {
                reply.err(refused(format!(
                    "{reason}; {}.",
                    registered_messages(&types)
                )));
                continue;
            }
        };
        signals.trigger(SendReflectedMessage {
            character,
            type_path: message.r#type.clone(),
            value,
        });
        let mut pending = PendingCall::new(reply, "the game to run a frame after the message")
            .timeout(MESSAGE_PATIENCE);
        pending.at_deadline = Some(format!(
            "Sent {} to the character, but the game ran no frame after the message; is it still playing?",
            short_type(&message.r#type)
        ));
        cmd.spawn((
            pending,
            SendingToCharacter {
                character,
                type_path: message.r#type,
                runs: 0,
            },
        ));
    }
}

fn answer_character_sends(
    mut waiting: Query<(Entity, &PendingCall, &mut SendingToCharacter)>,
    scene: EditWorld<Played>,
    mut cmd: CommandQueue,
) {
    let (players, graphs) = &*scene;
    for (call, pending, mut sending) in waiting.iter_mut() {
        let (Some(player), Some(graphs)) = (players.get_entity(sending.character), graphs) else {
            pending.reply.err(refused(
                "The character left the game before the message was answered; call scene_tree for current ids.",
            ));
            cmd.despawn(call);
            continue;
        };
        sending.runs += 1;
        if sending.runs < 2 {
            continue;
        }
        let mut nodes = Vec::new();
        player.for_each_node(graphs, |visit| nodes.push(PreviewNode::of(visit)));
        pending
            .reply
            .ok(message_answer(&sending.type_path, "the character", &nodes));
        cmd.despawn(call);
    }
}

fn answer_message_sends(
    mut reported: EventReader<Hosted<GraphPreviewState>>,
    mut waiting: Query<(Entity, &PendingCall, &mut SendingMessage)>,
    mut cmd: CommandQueue,
) {
    let reported: Vec<&Hosted<GraphPreviewState>> = reported.read().collect();
    for (call, pending, mut sending) in waiting.iter_mut() {
        let document = sending.document;
        for played in reported.iter().filter(|hosted| hosted.document == document) {
            sending.reports += 1;
            if sending.reports < 2 {
                continue;
            }
            pending.reply.ok(message_answer(
                &sending.type_path,
                "the preview",
                &played.event.nodes,
            ));
            cmd.despawn(call);
            break;
        }
    }
}

fn answer_param_sets(
    mut committed: EventReader<Hosted<PropertyCommitted>>,
    mut reported: EventReader<Hosted<GraphPreviewState>>,
    mut waiting: Query<(Entity, &mut PendingCall, &mut SettingParam)>,
    documents: Query<(&EditorDocument, &GraphDocument, &SentPreview)>,
    mut cmd: CommandQueue,
) {
    let written: Vec<&Hosted<PropertyCommitted>> = committed
        .read()
        .filter(|hosted| !hosted.event.address.target.saved())
        .collect();
    let reported: Vec<&Hosted<GraphPreviewState>> = reported.read().collect();
    for (call, mut pending, mut setting) in waiting.iter_mut() {
        let Some((document, state, sent)) = documents.get_entity(setting.document) else {
            pending.reply.err(refused(
                "The graph's tab closed before the parameter was set.",
            ));
            cmd.despawn(call);
            continue;
        };
        if let ParamStep::Committing { waited } = setting.step {
            let result = written
                .iter()
                .find(|hosted| {
                    hosted.document == setting.document
                        && hosted
                            .event
                            .address
                            .path
                            .get_depth(0)
                            .is_some_and(|segment| segment == setting.label)
                })
                .map(|hosted| &hosted.event.result);
            match result {
                Some(Ok(())) if !state.preview.playing => {
                    pending.reply.ok(format!("{} {PAUSED}", setting.answer));
                }
                Some(Ok(())) => {
                    setting.step = ParamStep::Sending;
                    pending.waiting_for = "the preview to report what it plays";
                }
                Some(Err(refusal)) => pending.reply.err(refused(refusal.status().to_string())),
                None if waited => {
                    pending.reply.err(refused(format!("{}.", document.status)));
                }
                None => {
                    setting.step = ParamStep::Committing { waited: true };
                    continue;
                }
            }
            if setting.step != ParamStep::Sending {
                cmd.despawn(call);
                continue;
            }
        }
        if setting.step == ParamStep::Sending {
            if sent.holds_params(state) {
                setting.step = ParamStep::Playing;
            }
            continue;
        }
        let Some(played) = reported
            .iter()
            .rev()
            .find(|hosted| hosted.document == setting.document)
        else {
            continue;
        };
        let live = LiveView::of(&played.event, sent).and_then(|live| live_block(state, live));
        pending.reply.ok(match live {
            Some(live) => format!("{}\n{live}", setting.answer),
            None => format!("{} {NOT_PLAYING}", setting.answer),
        });
        cmd.despawn(call);
    }
}

fn graph_inspect(
    mut calls: ToolCalls<GraphInspect>,
    active: Res<ActiveEditor>,
    documents: Query<(&GraphDocument, &GraphLive, &SentPreview, Option<&HostedApp>)>,
    assets: Res<AssetRegistry>,
    project: Res<ProjectState>,
) {
    for (_, reply) in calls.read() {
        let front = active.0.and_then(|document| documents.get_entity(document));
        let Some((state, live, sent, hosted)) = front else {
            reply.err(no_graph_tab());
            continue;
        };
        let Some(copy) = state.graph() else {
            reply.err(refused(
                "The graph has not loaded; call status until it says busy: no, then graph_inspect.",
            ));
            continue;
        };
        let mut lines = vec![state.breadcrumb().join(" › ")];
        let types = hosted.and_then(HostedApp::type_registry);
        let found = state.found_params(&Sources {
            types: types.as_ref(),
            assets: &assets,
            root: project
                .project
                .as_ref()
                .map(|project| project.root.as_path()),
        });
        let types = types.as_ref().map(|types| types.read());
        let live = LiveView::latest(live, sent).and_then(|live| live_block(state, live));
        if let Some(machine) = state.machine() {
            state_lines(machine, &assets, &mut lines);
            if let Some(types) = types.as_deref() {
                param_lines(state, &found, types, &mut lines);
            }
            lines.extend(live);
            reply.ok(lines.join("\n"));
            continue;
        }
        for node in copy.nodes() {
            let place = match copy.link_of(node.id) {
                Some(link) => format!("{}#{}", link.parent, link.slot),
                None if node.id == copy.result() => "result".to_owned(),
                None => "loose".to_owned(),
            };
            let mut line = format!("{} · {} · {place}", node.id, node.short_name());
            if let Some(subtitle) = subtitle(node, types.as_deref(), &assets) {
                line.push_str(" · ");
                line.push_str(&subtitle);
            }
            lines.push(line);
        }
        if let Some(types) = types.as_deref() {
            param_lines(state, &found, types, &mut lines);
        }
        lines.extend(live);
        reply.ok(lines.join("\n"));
    }
}
