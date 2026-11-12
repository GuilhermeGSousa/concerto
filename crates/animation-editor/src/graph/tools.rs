use concerto_animation::node::state_machine::AnimationStateMachine;
use concerto_app::{App, schedule_groups::LateUpdate};
use concerto_ecs::{
    Component, Entity, Query, Res, command::CommandQueue, events::event_reader::EventReader,
};
use concerto_editor::{asset_editor::ActiveEditor, hosted::HostedApp};
use concerto_foundation::assets::{AssetId, registry::AssetRegistry};
use concerto_mcp::{
    AppMcpExt, McpTool, Needs, PendingCall, Reply, ToolCalls, ToolError, ToolRegistry,
};
use concerto_name::Name;
use schemars::JsonSchema;
use serde::Deserialize;

use super::{
    copy::{GraphCopy, GraphNodeId},
    document::{
        EditGraph, GraphChange, GraphDocument, GraphEdit, GraphEdited, GraphSelection, OpenTarget,
        TransitionRef, shown_transitions,
    },
    panels::subtitle,
};

/// Reads what the front animation graph tab shows: the breadcrumb, then for a pose graph one line per node, `id · type · parent#slot · subtitle`, with the result node and loose nodes marked, and for a state machine one line per state, `name · initial · graph`, then one per transition, `from → to #index · trigger · blend`.
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

/// Selects a node, a state or a transition of what the front animation graph tab shows, or nothing, publishing it as the value Node, State or Transition that inspect, set_property, list_push and list_remove take as target.
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
    .add_system(LateUpdate, answer_graph_edits);
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
        let edit = GraphEdit::AddNode {
            type_name: arguments.r#type,
            parent: arguments.parent.map(GraphNodeId),
            slot: arguments.slot,
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
            if live {
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

fn graph_inspect(
    mut calls: ToolCalls<GraphInspect>,
    active: Res<ActiveEditor>,
    documents: Query<(&GraphDocument, Option<&HostedApp>)>,
    assets: Res<AssetRegistry>,
) {
    for (_, reply) in calls.read() {
        let front = active.0.and_then(|document| documents.get_entity(document));
        let Some((state, hosted)) = front else {
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
        if let Some(machine) = state.machine() {
            state_lines(machine, &assets, &mut lines);
            reply.ok(lines.join("\n"));
            continue;
        }
        let types = hosted.and_then(HostedApp::type_registry);
        let types = types.as_ref().map(|types| types.read());
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
        reply.ok(lines.join("\n"));
    }
}
