use concerto_app::{App, schedule_groups::LateUpdate};
use concerto_ecs::{
    Component, Entity, Query, Res, command::CommandQueue, events::event_reader::EventReader,
};
use concerto_editor::{
    asset_editor::{ActiveEditor, EditorDocument},
    hosted::HostedApp,
};
use concerto_foundation::assets::registry::AssetRegistry;
use concerto_mcp::{
    AppMcpExt, McpTool, Needs, PendingCall, Reply, ToolCalls, ToolError, ToolRegistry,
};
use schemars::JsonSchema;
use serde::Deserialize;

use super::{
    copy::{GraphCopy, GraphNodeId},
    document::{EditGraph, GraphChange, GraphDocument, GraphEdit, GraphEdited, GraphSelection},
    panels::subtitle,
};

/// Reads the animation graph the front tab shows: its name, then one line per node, `id · type · parent#slot · subtitle`, with the result node and loose nodes marked.
#[derive(Deserialize, JsonSchema)]
pub struct GraphInspect {}

/// Adds a node to the animation graph the front tab shows and answers its id.
#[derive(Deserialize, JsonSchema)]
pub struct GraphAddNode {
    /// The node's type, by short name such as LayerNode, or by full path when two types share one.
    pub r#type: String,
    /// The node to make it an input of, by its id in graph_inspect; loose without one.
    pub parent: Option<u32>,
    /// The parent's input slot, from 0; its first empty slot without one.
    pub slot: Option<usize>,
}

/// Makes a node an input of another in the animation graph the front tab shows, moving it if it already is one.
#[derive(Deserialize, JsonSchema)]
pub struct GraphConnect {
    /// The node that becomes an input, by its id in graph_inspect.
    pub child: u32,
    /// The node that takes it.
    pub parent: u32,
    /// The parent's input slot, from 0: a taken slot of a node with named slots is replaced, and its input left loose. The first empty slot without one.
    pub slot: Option<usize>,
}

/// Removes a node, leaving its inputs loose, or a link, from the animation graph the front tab shows; give one of the two.
#[derive(Deserialize, JsonSchema)]
pub struct GraphRemove {
    /// The node to remove, by its id in graph_inspect.
    pub node: Option<u32>,
    /// The link to remove, named by the node it makes an input.
    pub link: Option<u32>,
}

/// Selects a node of the animation graph the front tab shows, or nothing, publishing it as the value Node that inspect, set_property, list_push and list_remove take as target.
#[derive(Deserialize, JsonSchema)]
pub struct GraphSelect {
    /// The node to select, by its id in graph_inspect; clears the selection without one.
    pub node: Option<u32>,
}

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

fn graph_connect(
    mut calls: ToolCalls<GraphConnect>,
    active: Res<ActiveEditor>,
    graphs: Query<&GraphDocument>,
    mut cmd: CommandQueue,
) {
    for (arguments, reply) in calls.read() {
        let edit = GraphEdit::Connect {
            child: GraphNodeId(arguments.child),
            parent: GraphNodeId(arguments.parent),
            slot: arguments.slot,
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
        let edit = match (arguments.node, arguments.link) {
            (Some(node), None) => GraphEdit::RemoveNode(GraphNodeId(node)),
            (None, Some(child)) => GraphEdit::RemoveLink {
                child: GraphNodeId(child),
            },
            _ => {
                reply.err(ToolError::new(
                    "invalid_arguments",
                    "Give node, or link, the node the link makes an input, and not both.",
                ));
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
        let selection = arguments
            .node
            .map(|node| GraphSelection::Node(GraphNodeId(node)));
        start(
            reply,
            GraphEdit::Select(selection),
            &active,
            &graphs,
            &mut cmd,
        );
    }
}

fn kind(copy: Option<&GraphCopy>, id: GraphNodeId) -> String {
    copy.and_then(|copy| copy.node(id))
        .map_or_else(|| "a node".to_owned(), |node| node.described())
}

fn answer(edit: &GraphEdit, change: &GraphChange, copy: Option<&GraphCopy>) -> String {
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
        GraphEdit::Connect { child, parent, .. } => format!(
            "Node {child} is input {} of node {parent}{displaced}.",
            change.slot.unwrap_or(0)
        ),
        GraphEdit::RemoveNode(node) => format!("Removed node {node}."),
        GraphEdit::RemoveLink { child } => format!("Node {child} is loose."),
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
        GraphEdit::Select(None) => "Nothing is selected.".to_owned(),
    }
}

fn answer_graph_edits(
    mut edited: EventReader<GraphEdited>,
    waiting: Query<(Entity, &PendingCall, &EditingGraph)>,
    graphs: Query<&GraphDocument>,
    mut cmd: CommandQueue,
) {
    for event in edited.read() {
        for (call, pending, editing) in waiting.iter() {
            if editing.document != event.document || editing.edit != event.edit {
                continue;
            }
            let copy = graphs
                .get_entity(event.document)
                .and_then(|state| state.graph.as_ref());
            match &event.result {
                Ok(change) => pending.reply.ok(answer(&event.edit, change, copy)),
                Err(reason) => pending.reply.err(refused(format!("{reason}."))),
            }
            cmd.despawn(call);
        }
    }
}

fn graph_inspect(
    mut calls: ToolCalls<GraphInspect>,
    active: Res<ActiveEditor>,
    documents: Query<(&EditorDocument, &GraphDocument, Option<&HostedApp>)>,
    assets: Res<AssetRegistry>,
) {
    for (_, reply) in calls.read() {
        let front = active.0.and_then(|document| documents.get_entity(document));
        let Some((document, state, hosted)) = front else {
            reply.err(no_graph_tab());
            continue;
        };
        let Some(copy) = &state.graph else {
            reply.err(refused(
                "The graph has not loaded; call status until it says busy: no, then graph_inspect.",
            ));
            continue;
        };
        let types = hosted.and_then(HostedApp::type_registry);
        let types = types.as_ref().map(|types| types.read());
        let mut lines = vec![document.title.clone()];
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
