use std::{path::Path, sync::Arc};

use concerto_animation::{
    graph::AnimationGraph,
    node::{PlaceholderNode, ReflectAnimationNode, ResultNode},
};
use concerto_app::App;
use concerto_ecs::{
    Component, Entity, Event, Query, Res, With,
    command::{CommandQueue, EntityCommandQueue},
    events::{event_reader::EventReader, event_writer::EventWriter},
    reflect::AppTypeRegistry,
    signal::{On, Signal, lifecycle::Add},
};
use concerto_editor::{
    asset_editor::{AssetEditor, EditorDocument, PreviewSpec, Unsaved, finish_asset_request},
    guard::DiscardEdits,
    hosted::{Hosted, HostedApp},
    inspector::{DocumentValue, InspectedValues, PropertyTarget, RefreshInspector},
    preview::PreviewFailed,
    project::ProjectState,
    tools::edits::PropertyCommitted,
};
use concerto_foundation::assets::{
    Asset,
    content::{decode_content_asset, read_content_asset},
    handle::LiveLeaves,
    registry::AssetEntry,
};
use concerto_reflect::{Reflect, ReflectedValue, TypeRegistration, TypeRegistry};

use super::copy::{GraphCopy, GraphNodeId};

const NODE: &str = "Node";
const READ_ONLY: &str = "The preview is not running, so the graph is read-only";

/// What a graph tab has selected: a node, or the link that makes `child` an input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphSelection {
    Node(GraphNodeId),
    Link { child: GraphNodeId },
}

/// What a graph's tab holds: the working copy of the graph in view and the selection.
#[derive(Component, Default)]
pub struct GraphDocument {
    pub graph: Option<GraphCopy>,
    pub selection: Option<GraphSelection>,
}

impl GraphDocument {
    /// The line a placeholder node shows as the status while it is selected.
    pub fn selected_warning(&self) -> Option<String> {
        let Some(GraphSelection::Node(id)) = self.selection else {
            return None;
        };
        let copy = self.graph.as_ref()?;
        let node = copy.node(id)?;
        if !copy.editable() || node.live().is_some() {
            return None;
        }
        Some(node.warning.clone().unwrap_or_else(|| {
            format!("{} is not registered as an animation node", node.type_name)
        }))
    }

    fn keep_selection(&mut self) {
        let Some(copy) = &self.graph else {
            self.selection = None;
            return;
        };
        let kept = match self.selection {
            Some(GraphSelection::Node(id)) => copy.node(id).is_some(),
            Some(GraphSelection::Link { child }) => copy.link_of(child).is_some(),
            None => true,
        };
        if !kept {
            self.selection = None;
        }
    }
}

/// One edit of the graph a tab has in view.
#[derive(Clone, Debug, PartialEq)]
pub enum GraphEdit {
    /// Adds a node of the type named by its short name or full path, loose or as an input of `parent`.
    AddNode {
        type_name: String,
        parent: Option<GraphNodeId>,
        slot: Option<usize>,
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
    /// Selects a node or a link, or nothing.
    Select(Option<GraphSelection>),
}

/// Applies `edit` to the graph the tab `document` has in view; the outcome is a [`GraphEdited`].
pub struct EditGraph {
    pub document: Entity,
    pub edit: GraphEdit,
}

impl Signal for EditGraph {}

/// What an applied [`GraphEdit`] changed: the node it added, the slot a connection took, and the input it displaced.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GraphChange {
    pub node: Option<GraphNodeId>,
    pub slot: Option<usize>,
    pub loose: Option<GraphNodeId>,
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

/// What a graph's preview adds to the game's recipe.
pub fn graph_preview(_: &mut App) {}

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
    types: Option<&AppTypeRegistry>,
) -> Result<GraphChange, String> {
    let copy = state.graph.as_mut().ok_or("No graph is open")?;
    if let GraphEdit::Select(selection) = edit {
        match selection {
            Some(GraphSelection::Node(id)) if copy.node(*id).is_none() => {
                return Err(format!(
                    "There is no node {id}; graph_inspect lists the nodes"
                ));
            }
            Some(GraphSelection::Link { child }) if copy.link_of(*child).is_none() => {
                return Err(format!("Node {child} is not an input of any node"));
            }
            _ => {}
        }
        state.selection = *selection;
        return Ok(GraphChange::default());
    }
    if !copy.editable() {
        return Err(READ_ONLY.to_owned());
    }
    let change = match edit {
        GraphEdit::AddNode {
            type_name,
            parent,
            slot,
        } => {
            let types = types.ok_or(READ_ONLY)?;
            add_node(copy, &types.read(), type_name, *parent, *slot)?
        }
        GraphEdit::Connect {
            child,
            parent,
            slot,
        } => {
            let connected = copy.connect(*child, *parent, *slot)?;
            GraphChange {
                node: None,
                slot: Some(connected.slot),
                loose: connected.loose,
            }
        }
        GraphEdit::RemoveNode(id) => {
            copy.remove_node(*id)?;
            GraphChange::default()
        }
        GraphEdit::RemoveLink { child } => {
            copy.remove_link(*child)?;
            GraphChange::default()
        }
        GraphEdit::Select(_) => GraphChange::default(),
    };
    state.keep_selection();
    Ok(change)
}

fn node_value(document: Entity, id: GraphNodeId) -> Arc<dyn PropertyTarget> {
    Arc::new(DocumentValue::<GraphDocument> {
        name: NODE.to_owned(),
        document,
        read: Box::new(move |state| {
            let node = state.graph.as_ref()?.node(id)?.live()?;
            Some(node.as_reflect())
        }),
        edit: Box::new(move |state| {
            let node = state.graph.as_mut()?.node_mut(id)?.live_mut()?;
            Some(node.as_reflect_mut() as &mut dyn Reflect)
        }),
    })
}

fn publish_selection(document: Entity, state: &GraphDocument, cmd: &mut CommandQueue) {
    let live = |id: GraphNodeId| {
        let node = state.graph.as_ref()?.node(id)?;
        node.live().map(|_| id)
    };
    let values = match state.selection {
        Some(GraphSelection::Node(id)) => live(id)
            .map(|id| node_value(document, id))
            .into_iter()
            .collect(),
        _ => Vec::new(),
    };
    cmd.insert(InspectedValues(values), document);
    cmd.trigger(RefreshInspector);
}

pub(crate) fn apply_graph_edit(
    on: On<EditGraph>,
    documents: Query<(&mut EditorDocument, &mut GraphDocument, Option<&HostedApp>)>,
    mut edited: EventWriter<GraphEdited>,
    mut cmd: CommandQueue,
) {
    let EditGraph { document, edit } = on.signal();
    let Some((mut editor, mut state, hosted)) = documents.get_entity(*document) else {
        return;
    };
    let types = hosted.and_then(HostedApp::type_registry);
    let before = state.selection;
    let result = apply(&mut state, edit, types.as_ref());
    let status = match &result {
        Ok(_) => state.selected_warning().unwrap_or_default(),
        Err(reason) => reason.clone(),
    };
    if editor.status != status {
        editor.status = status;
    }
    if result.is_ok() && !matches!(edit, GraphEdit::Select(_)) {
        cmd.insert(Unsaved, *document);
    }
    if state.selection != before {
        publish_selection(*document, &state, &mut cmd);
    }
    edited.write(GraphEdited {
        document: *document,
        edit: edit.clone(),
        result,
    });
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

fn hold(document: Entity, state: &mut GraphDocument, copy: GraphCopy, cmd: &mut CommandQueue) {
    state.graph = Some(copy);
    state.selection = None;
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
                hold(entity, &mut state, copy, &mut cmd);
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
        Ok(copy) => hold(document, &mut state, copy, &mut cmd),
        Err(error) => {
            log::warn!("Discarded edits stay: {error:#}");
            cmd.insert(Unsaved, document);
        }
    }
}

pub(crate) fn follow_node_edits(
    mut committed: EventReader<Hosted<PropertyCommitted>>,
    documents: Query<Entity, With<GraphDocument>>,
    mut cmd: CommandQueue,
) {
    for hosted in committed.read() {
        if hosted.event.result.is_ok() && documents.get_entity(hosted.document).is_some() {
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
