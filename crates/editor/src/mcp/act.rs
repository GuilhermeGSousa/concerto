use std::path::PathBuf;

use concerto_app::{App, schedule_groups::LateUpdate};
use concerto_ecs::{
    Component, Entity, IntoSystemConfig, Query, Res, ResMut,
    command::CommandQueue,
    component::name::Name,
    events::event_reader::EventReader,
    query::filter::{With, Without},
};
use concerto_foundation::assets::{Asset, AssetId};
use concerto_mcp::{
    AppMcpExt, McpServe, McpTool, Needs, PendingCall, ToolCalls, ToolError, answer_unanswered,
};
use concerto_scene::scene::Scene;
use schemars::JsonSchema;
use serde::Deserialize;

use super::observe::resolve;
use crate::{
    asset_editor::{AssetEditorCommand, AssetEditorCommands, AssetEditorRegistry, EditorDocument},
    guard::{GuardChoice, UnsavedGuard},
    hosted::{EditWorld, EditWorldSignals},
    project::{
        AssetEntry, EditorCommand, EditorCommands, Project, ProjectOpenFailed, ProjectOpened,
        ProjectState,
    },
    scene::{SceneOpenFailed, SceneOpened},
    tools::{
        camera::{FrameAll, FrameSelection},
        selection,
    },
};

/// Opens the project in a directory of imported content, closing every open document, and waits until its catalogue is read.
#[derive(Deserialize, JsonSchema)]
pub struct OpenProject {
    /// The project's directory; a relative path is read from the editor's working directory.
    pub path: String,
}

/// Opens an asset of the project in its editor; a scene replaces the one shown, and the call waits until it has loaded.
#[derive(Deserialize, JsonSchema)]
pub struct OpenAsset {
    /// The asset's address, such as content/level.gasset, or its id, as list_assets shows them.
    pub asset: String,
}

/// Closes an open document.
#[derive(Deserialize, JsonSchema)]
pub struct CloseDocument {
    /// The document's id, such as 3v1, as status lists it.
    pub document: String,
}

/// Selects an entity of the open scene, or clears the selection when no entity is given.
#[derive(Deserialize, JsonSchema)]
pub struct Select {
    /// The entity to select, such as 42v3, as scene_tree shows it.
    pub entity: Option<String>,
}

/// Moves the editor camera to show an entity and its children, selecting it, or the whole scene when no entity is given.
#[derive(Deserialize, JsonSchema)]
pub struct Frame {
    /// The entity to frame, such as 42v3, as scene_tree shows it.
    pub entity: Option<String>,
}

#[derive(Component)]
struct OpeningProject {
    requested: PathBuf,
    canonical: Option<PathBuf>,
}

#[derive(Component)]
struct OpeningAsset {
    asset: AssetEntry,
    kind: &'static str,
    document: Option<Entity>,
}

#[derive(Component)]
struct ClosingDocument(Entity);

#[derive(Component)]
struct Guarded;

#[derive(Component)]
struct CancelledPrompt(Vec<String>);

#[derive(Component)]
struct Framing(String);

pub(super) fn build(app: &mut App) {
    app.add_tool::<OpenProject, _>(McpTool::new("open_project"), open_project)
        .add_tool::<OpenAsset, _>(McpTool::new("open_asset").needs(Needs::Project), open_asset)
        .add_tool::<CloseDocument, _>(McpTool::new("close_document"), close_document)
        .add_tool::<Select, _>(McpTool::new("select").needs(Needs::Scene), select)
        .add_tool::<Frame, _>(McpTool::new("frame").needs(Needs::Scene), frame)
        .add_system(
            McpServe,
            answer_framed.before(frame).before(answer_unanswered),
        )
        .add_system(
            LateUpdate,
            (
                cancel_raised_prompt,
                answer_project_opens,
                answer_asset_opens,
                answer_closes,
            ),
        );
}

fn busy() -> ToolError {
    ToolError::new(
        "busy",
        "A project is opening; call status until it says busy: no, then try again.",
    )
}

fn project_opening(state: &ProjectState, commands: &EditorCommands) -> bool {
    state.busy()
        || commands
            .0
            .iter()
            .any(|command| matches!(command, EditorCommand::OpenProject(_)))
}

fn open_project(
    mut calls: ToolCalls<OpenProject>,
    state: Res<ProjectState>,
    mut commands: ResMut<EditorCommands>,
    mut cmd: CommandQueue,
) {
    for (arguments, reply) in calls.read() {
        if project_opening(&state, &commands) {
            reply.err(busy());
            continue;
        }
        let requested = PathBuf::from(&arguments.path);
        commands
            .0
            .push_back(EditorCommand::OpenProject(requested.clone()));
        cmd.spawn((
            PendingCall::new(reply, "the project to open"),
            OpeningProject {
                canonical: requested.canonicalize().ok(),
                requested,
            },
            Guarded,
        ));
    }
}

fn counted(count: usize, one: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {one}s")
    }
}

fn answer_project_opens(
    mut opened: EventReader<ProjectOpened>,
    mut failed: EventReader<ProjectOpenFailed>,
    state: Res<ProjectState>,
    waiting: Query<(Entity, &PendingCall, &OpeningProject), Without<CancelledPrompt>>,
    mut cmd: CommandQueue,
) {
    for event in opened.read() {
        for (entity, pending, opening) in waiting.iter() {
            if opening.canonical.as_ref() != Some(&event.root) {
                continue;
            }
            let summary = state
                .project
                .as_ref()
                .filter(|project| project.root == event.root)
                .map(|project| {
                    format!(
                        ": {}, {}",
                        counted(project.assets.len(), "asset"),
                        counted(project.scenes().count(), "scene")
                    )
                })
                .unwrap_or_default();
            pending
                .reply
                .ok(format!("Opened {}{summary}.", event.root.display()));
            cmd.despawn(entity);
        }
    }
    for event in failed.read() {
        for (entity, pending, opening) in waiting.iter() {
            if opening.requested == event.root {
                pending
                    .reply
                    .err(ToolError::new("open_failed", event.message.clone()));
                cmd.despawn(entity);
            }
        }
    }
}

fn find_asset<'a>(project: &'a Project, asset: &str) -> Result<&'a AssetEntry, ToolError> {
    let id = AssetId::from_simple_hex(asset).ok();
    project
        .assets
        .iter()
        .find(|entry| entry.address == asset || Some(entry.id) == id)
        .ok_or_else(|| {
            ToolError::new(
                "unknown_asset",
                format!("No asset `{asset}` in the project; list_assets shows addresses and ids."),
            )
        })
}

fn opened(asset: &AssetEntry, document: Entity) -> String {
    format!(
        "Opened {} in document {}.",
        asset.address,
        document.to_id_string()
    )
}

fn open_asset(
    mut calls: ToolCalls<OpenAsset>,
    state: Res<ProjectState>,
    editors: Res<AssetEditorRegistry>,
    documents: Query<(Entity, &EditorDocument)>,
    mut commands: ResMut<EditorCommands>,
    mut cmd: CommandQueue,
) {
    for (arguments, reply) in calls.read() {
        if project_opening(&state, &commands) {
            reply.err(busy());
            continue;
        }
        let Some(project) = &state.project else {
            continue;
        };
        let asset = match find_asset(project, &arguments.asset) {
            Ok(asset) => asset.clone(),
            Err(error) => {
                reply.err(error);
                continue;
            }
        };
        let Some(kind) = editors.editor_kind(&asset.kind) else {
            reply.err(ToolError::new(
                "open_failed",
                format!(
                    "{} is a {}, which has no editor; list_assets kind=Scene shows what opens.",
                    asset.address, asset.kind
                ),
            ));
            continue;
        };
        commands.0.push_back(EditorCommand::OpenAsset(asset.id));
        let document = documents
            .iter()
            .find(|(_, document)| document.asset_type == kind);
        if let Some((entity, _)) = document.filter(|(_, document)| {
            document
                .current
                .as_ref()
                .is_some_and(|current| current.id == asset.id)
        }) {
            reply.ok(opened(&asset, entity));
            continue;
        }
        cmd.spawn((
            PendingCall::new(reply, "the asset to open"),
            OpeningAsset {
                asset,
                kind,
                document: document.map(|(entity, _)| entity),
            },
            Guarded,
        ));
    }
}

fn answer_asset_opens(
    mut opened_scenes: EventReader<SceneOpened>,
    mut failed_scenes: EventReader<SceneOpenFailed>,
    documents: Query<(Entity, &EditorDocument)>,
    waiting: Query<(Entity, &PendingCall, &OpeningAsset), Without<CancelledPrompt>>,
    mut cmd: CommandQueue,
) {
    let awaits = |opening: &OpeningAsset, document: Entity, asset: &AssetEntry| {
        opening.asset.id == asset.id && opening.document.is_none_or(|known| known == document)
    };
    for event in opened_scenes.read() {
        for (entity, pending, opening) in waiting.iter() {
            if awaits(opening, event.document, &event.asset) {
                pending.reply.ok(opened(&opening.asset, event.document));
                cmd.despawn(entity);
            }
        }
    }
    for event in failed_scenes.read() {
        for (entity, pending, opening) in waiting.iter() {
            if awaits(opening, event.document, &event.asset) {
                pending
                    .reply
                    .err(ToolError::new("open_failed", event.message.clone()));
                cmd.despawn(entity);
            }
        }
    }
    for (entity, pending, opening) in waiting.iter() {
        if opening.kind == Scene::name() {
            continue;
        }
        let shown = documents.iter().find(|(_, document)| {
            document.asset_type == opening.kind
                && [&document.current, &document.pending]
                    .into_iter()
                    .flatten()
                    .any(|asset| asset.id == opening.asset.id)
        });
        if let Some((document, _)) = shown {
            pending.reply.ok(opened(&opening.asset, document));
            cmd.despawn(entity);
        }
    }
}

fn close_document(
    mut calls: ToolCalls<CloseDocument>,
    documents: Query<Entity, With<EditorDocument>>,
    mut requests: ResMut<AssetEditorCommands>,
    mut cmd: CommandQueue,
) {
    for (arguments, reply) in calls.read() {
        let id = &arguments.document;
        let Some(document) =
            Entity::parse_id(id).filter(|&entity| documents.get_entity(entity).is_some())
        else {
            reply.err(ToolError::new(
                "unknown_entity",
                format!("No open document `{id}`; status lists the documents and their ids."),
            ));
            continue;
        };
        requests.0.push_back(AssetEditorCommand::Close(document));
        cmd.spawn((
            PendingCall::new(reply, "the document to close"),
            ClosingDocument(document),
            Guarded,
        ));
    }
}

fn answer_closes(
    documents: Query<&EditorDocument>,
    waiting: Query<(Entity, &PendingCall, &ClosingDocument), Without<CancelledPrompt>>,
    mut cmd: CommandQueue,
) {
    for (entity, pending, closing) in waiting.iter() {
        if documents.get_entity(closing.0).is_none() {
            pending
                .reply
                .ok(format!("Closed document {}.", closing.0.to_id_string()));
            cmd.despawn(entity);
        }
    }
}

fn describe_document(entity: Entity, documents: &Query<&EditorDocument>) -> String {
    let address = documents.get_entity(entity).map(|document| {
        document
            .current
            .as_ref()
            .map_or(document.title.as_str(), |asset| asset.address.as_str())
            .to_owned()
    });
    match address {
        Some(address) => format!("{} {address}", entity.to_id_string()),
        None => entity.to_id_string(),
    }
}

fn cancel_raised_prompt(
    mut guard: ResMut<UnsavedGuard>,
    raising: Query<Entity, (With<PendingCall>, With<Guarded>, Without<CancelledPrompt>)>,
    cancelled: Query<(Entity, &PendingCall, &CancelledPrompt)>,
    documents: Query<&EditorDocument>,
    mut cmd: CommandQueue,
) {
    if !guard.asking() {
        for (entity, pending, prompt) in cancelled.iter() {
            pending.reply.err(ToolError::new(
                "unsaved_changes",
                format!(
                    "The call would discard unsaved changes in {}, so it was cancelled; save them first.",
                    prompt.0.join(", ")
                ),
            ));
            cmd.despawn(entity);
        }
        return;
    }
    let Some(call) = raising.iter().next() else {
        return;
    };
    let named = guard
        .documents()
        .iter()
        .map(|&document| describe_document(document, &documents))
        .collect();
    guard.choose(GuardChoice::Cancel);
    cmd.insert(CancelledPrompt(named), call);
}

fn label(entity: Entity, names: &Query<&Name>) -> String {
    match names.get_entity(entity) {
        Some(name) if !name.as_str().is_empty() => {
            format!("{} {}", entity.to_id_string(), name.as_str())
        }
        _ => entity.to_id_string(),
    }
}

fn select(
    mut calls: ToolCalls<Select>,
    entities: EditWorld<Query<Entity>>,
    names: EditWorld<Query<&Name>>,
    mut scene: EditWorldSignals,
) {
    for (arguments, reply) in calls.read() {
        match arguments.entity.as_deref().map(|id| resolve(id, &entities)) {
            Some(Ok(entity)) => {
                scene.trigger(selection::Select(Some(entity)));
                reply.ok(format!("Selected {}.", label(entity, &names)));
            }
            Some(Err(error)) => reply.err(error),
            None => {
                scene.trigger(selection::Select(None));
                reply.ok("Selection cleared.");
            }
        }
    }
}

fn frame(
    mut calls: ToolCalls<Frame>,
    entities: EditWorld<Query<Entity>>,
    names: EditWorld<Query<&Name>>,
    mut scene: EditWorldSignals,
    mut cmd: CommandQueue,
) {
    for (arguments, reply) in calls.read() {
        let framed = match arguments.entity.as_deref().map(|id| resolve(id, &entities)) {
            Some(Ok(entity)) => {
                scene.trigger(selection::Select(Some(entity)));
                scene.trigger(FrameSelection);
                label(entity, &names)
            }
            Some(Err(error)) => {
                reply.err(error);
                continue;
            }
            None => {
                scene.trigger(FrameAll);
                "the whole scene".to_owned()
            }
        };
        cmd.spawn((
            PendingCall::new(reply, "the camera to frame"),
            Framing(framed),
        ));
    }
}

fn answer_framed(waiting: Query<(Entity, &PendingCall, &Framing)>, mut cmd: CommandQueue) {
    for (entity, pending, framing) in waiting.iter() {
        pending.reply.ok(format!("Framed {}.", framing.0));
        cmd.despawn(entity);
    }
}
