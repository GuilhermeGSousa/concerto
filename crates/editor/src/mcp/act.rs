use std::path::PathBuf;

use concerto_app::{App, schedule_groups::LateUpdate};
use concerto_ecs::{
    Component, Entity, IntoSystemConfig, Query, Res, ResMut,
    command::CommandQueue,
    component::name::Name,
    events::event_reader::EventReader,
    query::filter::{With, Without},
};
use concerto_foundation::{
    assets::{
        Asset, AssetId,
        registry::{AssetEntry, AssetRegistry},
    },
    transform::{GlobalTransform, Transform},
};
use concerto_mcp::{
    AppMcpExt, McpServe, McpTool, Needs, PendingCall, ToolCalls, ToolError, answer_unanswered,
};
use concerto_scene::scene::Scene;
use glam::Vec3;
use schemars::JsonSchema;
use serde::Deserialize;

use super::observe::{SceneEntities, resolve};
use crate::{
    asset_editor::{AssetEditorCommand, AssetEditorCommands, AssetEditorRegistry, EditorDocument},
    guard::{GuardChoice, UnsavedGuard},
    hosted::{EditTarget, EditWorld, EditWorldSignals, PlayState},
    project::{EditorCommand, EditorCommands, ProjectOpenFailed, ProjectOpened, ProjectState},
    scene::{SaveFailed, SceneOpenFailed, SceneOpened},
    tools::{
        camera::{
            EditorCamera, FlyCamera, FrameAll, FrameSelection, SetCameraPose, ViewportCommands,
        },
        selection,
    },
};

/// Opens the project in a directory of imported content, closing every open document, and waits until its catalogue is read.
#[derive(Deserialize, JsonSchema)]
pub struct OpenProject {
    /// The project's directory; a relative path is read from the editor's working directory.
    pub path: String,
    /// What to do with unsaved changes the call would lose; without it such a call fails with unsaved_changes.
    pub unsaved: Option<UnsavedChoice>,
}

/// Opens an asset of the project in its editor; a scene replaces the one shown, and the call waits until it has loaded.
#[derive(Deserialize, JsonSchema)]
pub struct OpenAsset {
    /// The asset's address, such as content/level.gasset, or its id, as list_assets shows them.
    pub asset: String,
    /// What to do with unsaved changes the call would lose; without it such a call fails with unsaved_changes.
    pub unsaved: Option<UnsavedChoice>,
}

/// Closes an open document.
#[derive(Deserialize, JsonSchema)]
pub struct CloseDocument {
    /// The document's id, such as 3v1, as status lists it.
    pub document: String,
    /// What to do with unsaved changes the call would lose; without it such a call fails with unsaved_changes.
    pub unsaved: Option<UnsavedChoice>,
}

/// What a call does with unsaved changes it would lose; a call that loses none ignores it.
#[derive(Deserialize, JsonSchema, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum UnsavedChoice {
    /// Save them first, then carry on; a save that fails answers save_failed.
    Save,
    /// Drop them and carry on.
    Discard,
}

impl From<UnsavedChoice> for GuardChoice {
    fn from(choice: UnsavedChoice) -> Self {
        match choice {
            UnsavedChoice::Save => GuardChoice::Save,
            UnsavedChoice::Discard => GuardChoice::Discard,
        }
    }
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

/// Places the editor camera at a position, facing a point or turned by yaw and pitch, and answers the pose it took.
#[derive(Deserialize, JsonSchema)]
pub struct SetCamera {
    /// Where the camera goes, as [x, y, z] in metres.
    pub position: [f32; 3],
    /// A point to face, as [x, y, z]; give this or yaw and pitch.
    pub look_at: Option<[f32; 3]>,
    /// Radians around world up: 0 faces -Z and a positive yaw turns left. Give it with pitch, or look_at instead.
    pub yaw: Option<f32>,
    /// Radians up from level, kept within ±1.55. Give it with yaw, or look_at instead.
    pub pitch: Option<f32>,
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
struct Guarded(Option<GuardChoice>);

impl Guarded {
    fn new(unsaved: Option<UnsavedChoice>) -> Self {
        Self(unsaved.map(GuardChoice::from))
    }
}

#[derive(Component)]
struct CancelledPrompt(Vec<String>);

#[derive(Component)]
struct SavingFirst(Vec<Entity>);

#[derive(Component)]
struct AwaitingPose {
    framed: Option<String>,
}

pub(super) fn build(app: &mut App) {
    app.add_tool::<OpenProject, _>(McpTool::new("open_project"), open_project)
        .add_tool::<OpenAsset, _>(McpTool::new("open_asset").needs(Needs::Project), open_asset)
        .add_tool::<CloseDocument, _>(McpTool::new("close_document"), close_document)
        .add_tool::<Select, _>(McpTool::new("select").needs(Needs::Scene), select)
        .add_tool::<Frame, _>(McpTool::new("frame").needs(Needs::Scene), frame)
        .add_tool::<SetCamera, _>(McpTool::new("set_camera").needs(Needs::Scene), set_camera)
        .add_system(
            McpServe,
            answer_posed
                .before(frame)
                .before(set_camera)
                .before(answer_unanswered),
        )
        .add_system(
            LateUpdate,
            (
                answer_raised_prompt,
                answer_failed_saves,
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
            Guarded::new(arguments.unsaved),
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
    registry: Res<AssetRegistry>,
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
                .map(|_| {
                    format!(
                        ": {}, {}",
                        counted(registry.iter().count(), "asset"),
                        counted(registry.of_type::<Scene>().count(), "scene")
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

fn find_asset<'a>(registry: &'a AssetRegistry, asset: &str) -> Result<&'a AssetEntry, ToolError> {
    registry
        .id_for_address(asset)
        .or_else(|| AssetId::from_simple_hex(asset).ok())
        .and_then(|id| registry.get(id))
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
    registry: Res<AssetRegistry>,
    editors: Res<AssetEditorRegistry>,
    documents: Query<(Entity, &mut EditorDocument)>,
    mut commands: ResMut<EditorCommands>,
    mut cmd: CommandQueue,
) {
    for (arguments, reply) in calls.read() {
        if project_opening(&state, &commands) {
            reply.err(busy());
            continue;
        }
        if state.project.is_none() {
            continue;
        }
        let asset = match find_asset(&registry, &arguments.asset) {
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
            .find(|(_, document)| document.asset_type == kind)
            .map(|(entity, _)| entity);
        if let Some(entity) = document
            && let Some((_, mut shown)) = documents.get_entity(entity)
        {
            if shown
                .current
                .as_ref()
                .is_some_and(|current| current.id == asset.id)
            {
                reply.ok(opened(&asset, entity));
                continue;
            }
            if shown
                .failed
                .as_ref()
                .is_some_and(|failed| failed.id == asset.id)
            {
                shown.failed = None;
            }
        }
        cmd.spawn((
            PendingCall::new(reply, "the asset to open"),
            OpeningAsset {
                asset,
                kind,
                document,
            },
            Guarded::new(arguments.unsaved),
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
            continue;
        }
        let failed = documents.iter().find(|(_, document)| {
            document.asset_type == opening.kind
                && document
                    .failed
                    .as_ref()
                    .is_some_and(|asset| asset.id == opening.asset.id)
        });
        if let Some((_, document)) = failed {
            pending
                .reply
                .err(ToolError::new("open_failed", document.status.clone()));
            cmd.despawn(entity);
        }
    }
}

pub(super) fn unknown_document(id: &str) -> ToolError {
    ToolError::new(
        "unknown_entity",
        format!("No open document `{id}`; status lists the documents and their ids."),
    )
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
            reply.err(unknown_document(id));
            continue;
        };
        requests.0.push_back(AssetEditorCommand::Close(document));
        cmd.spawn((
            PendingCall::new(reply, "the document to close"),
            ClosingDocument(document),
            Guarded::new(arguments.unsaved),
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

pub(super) fn describe_document(entity: Entity, documents: &Query<&EditorDocument>) -> String {
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

fn answer_raised_prompt(
    mut guard: ResMut<UnsavedGuard>,
    raising: Query<(Entity, &Guarded), (With<PendingCall>, Without<CancelledPrompt>)>,
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
    let Some((call, guarded)) = raising.iter().next() else {
        return;
    };
    if let Some(choice) = guarded.0 {
        if choice == GuardChoice::Save {
            cmd.insert(SavingFirst(guard.documents().to_vec()), call);
        }
        guard.choose(choice);
        return;
    }
    let named = guard
        .documents()
        .iter()
        .map(|&document| describe_document(document, &documents))
        .collect();
    guard.choose(GuardChoice::Cancel);
    cmd.insert(CancelledPrompt(named), call);
}

fn answer_failed_saves(
    mut failed: EventReader<SaveFailed>,
    waiting: Query<(Entity, &PendingCall, &SavingFirst)>,
    mut cmd: CommandQueue,
) {
    let failures: Vec<&SaveFailed> = failed.read().collect();
    for (entity, pending, saving) in waiting.iter() {
        if let Some(failure) = failures
            .iter()
            .find(|failure| saving.0.contains(&failure.document))
        {
            pending
                .reply
                .err(ToolError::new("save_failed", failure.message.clone()));
            cmd.despawn(entity);
        }
    }
}

pub(super) fn label(entity: Entity, names: &Query<&Name>) -> String {
    match names.get_entity(entity) {
        Some(name) if !name.as_str().is_empty() => {
            format!("{} {}", entity.to_id_string(), name.as_str())
        }
        _ => entity.to_id_string(),
    }
}

fn select(
    mut calls: ToolCalls<Select>,
    entities: EditWorld<SceneEntities>,
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

fn playing(targets: &Query<&PlayState, With<EditTarget>>) -> bool {
    targets.iter().any(|state| *state == PlayState::Playing)
}

fn camera_refused() -> ToolError {
    ToolError::new(
        "refused",
        "The game is playing and has no editor camera; stop the game to place the camera.",
    )
}

fn frame(
    mut calls: ToolCalls<Frame>,
    entities: EditWorld<SceneEntities>,
    names: EditWorld<Query<&Name>>,
    fly: EditWorld<Option<Res<FlyCamera>>>,
    targets: Query<&PlayState, With<EditTarget>>,
    mut scene: EditWorldSignals,
    mut cmd: CommandQueue,
) {
    for (arguments, reply) in calls.read() {
        if playing(&targets) {
            reply.err(camera_refused());
            continue;
        }
        if fly.is_none() {
            continue;
        }
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
            AwaitingPose {
                framed: Some(framed),
            },
        ));
    }
}

fn invalid_pose(message: &str) -> ToolError {
    ToolError::new("invalid_arguments", message)
}

fn pose_of(arguments: &SetCamera) -> Result<SetCameraPose, ToolError> {
    let finite = arguments
        .position
        .iter()
        .chain(arguments.look_at.iter().flatten())
        .chain(&arguments.yaw)
        .chain(&arguments.pitch)
        .all(|number| number.is_finite());
    if !finite {
        return Err(invalid_pose(
            "position, look_at, yaw and pitch must be finite numbers within f32's range, about ±3.4e38.",
        ));
    }
    let position = Vec3::from_array(arguments.position);
    match (arguments.look_at, arguments.yaw, arguments.pitch) {
        (Some(target), None, None) => SetCameraPose::looking_at(position, Vec3::from_array(target))
            .ok_or_else(|| {
                invalid_pose("look_at is the camera's own position; give a point away from it.")
            }),
        (None, Some(yaw), Some(pitch)) => Ok(SetCameraPose {
            position,
            yaw,
            pitch,
        }),
        _ => Err(invalid_pose(
            "Give either look_at, or yaw and pitch together, but not both.",
        )),
    }
}

fn set_camera(
    mut calls: ToolCalls<SetCamera>,
    fly: EditWorld<Option<Res<FlyCamera>>>,
    targets: Query<&PlayState, With<EditTarget>>,
    mut scene: EditWorldSignals,
    mut cmd: CommandQueue,
) {
    for (arguments, reply) in calls.read() {
        if playing(&targets) {
            reply.err(camera_refused());
            continue;
        }
        if fly.is_none() {
            continue;
        }
        match pose_of(&arguments) {
            Ok(pose) => {
                scene.trigger(pose);
                cmd.spawn((
                    PendingCall::new(reply, "the camera to move"),
                    AwaitingPose { framed: None },
                ));
            }
            Err(error) => reply.err(error),
        }
    }
}

fn shown(value: f32) -> f32 {
    (value * 1000.0).round() / 1000.0 + 0.0
}

pub(super) fn describe_pose(fly: &FlyCamera) -> String {
    let [x, y, z] = fly.position.to_array().map(shown);
    let [fx, fy, fz] = fly.forward().to_array().map(shown);
    format!(
        "Camera at ({x:.3}, {y:.3}, {z:.3}), yaw {:.3}, pitch {:.3}, looking along ({fx:.3}, {fy:.3}, {fz:.3}).",
        shown(fly.yaw),
        shown(fly.pitch)
    )
}

fn answer_posed(
    waiting: Query<(Entity, &PendingCall, &AwaitingPose)>,
    fly: EditWorld<Option<Res<FlyCamera>>>,
    requests: EditWorld<Option<Res<ViewportCommands>>>,
    cameras: EditWorld<Query<(&Transform, &GlobalTransform), With<EditorCamera>>>,
    mut cmd: CommandQueue,
) {
    let (Some(fly), Some(requests)) = (fly.as_deref(), requests.as_deref()) else {
        return;
    };
    let framing = requests.frame_selection || requests.frame_all;
    let placed = cameras.iter().next().is_some_and(|(transform, drawn)| {
        fly.placed(transform) && drawn.matrix() == transform.compute_matrix()
    });
    if framing || !placed {
        return;
    }
    for (entity, pending, awaiting) in waiting.iter() {
        let pose = describe_pose(fly);
        match &awaiting.framed {
            Some(framed) => pending.reply.ok(format!("Framed {framed}. {pose}")),
            None => pending.reply.ok(pose),
        }
        cmd.despawn(entity);
    }
}
