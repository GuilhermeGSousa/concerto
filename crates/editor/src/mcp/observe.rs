use concerto_app::App;
use concerto_ecs::{
    Entity, IntoSystemConfig, Query, Res,
    component::name::Name,
    query::filter::With,
    system::input::{StaticSystemInput, SystemInputData},
};
use concerto_mcp::{
    AppMcpExt, McpRead, McpServe, McpTool, Needs, ToolCalls, answer_unanswered,
    receive_read_only_calls,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value};

use super::{BusyInputs, is_busy};
use crate::{
    asset_editor::{ActiveEditor, EditorDocument, Unsaved},
    hosted::{EditTarget, EditWorld, HostedApp, PlayState, input::ViewportShown},
    project::{Project, ProjectState},
    tools::selection::Selection,
};

/// What the editor has open: the project, its documents, the active one, the selection, the viewport's size, and whether it is busy.
#[derive(Deserialize, JsonSchema)]
pub struct Status {}

/// The open project's assets as `address · kind · id` lines, filtered and paged.
#[derive(Deserialize, JsonSchema)]
pub struct ListAssets {
    /// Text to find in an asset's name, address or kind, ignoring case.
    pub query: Option<String>,
    /// The exact kind to keep, such as Scene, Mesh or Texture.
    pub kind: Option<String>,
    /// The project-relative folder to keep, such as content/meshes, with its subfolders.
    pub folder: Option<String>,
    /// How many assets to show.
    #[serde(default = "fifty")]
    pub limit: usize,
    /// How many matching assets to skip.
    #[serde(default)]
    pub offset: usize,
}

fn fifty() -> usize {
    50
}

pub(super) fn build(app: &mut App) {
    app.add_tool::<Status, _>(McpTool::read_only("status"), status)
        .add_system(McpServe, status_of_edit_target.before(answer_unanswered))
        .add_system(
            McpRead,
            status_of_edit_target
                .after(receive_read_only_calls)
                .before(answer_unanswered),
        )
        .add_tool::<ListAssets, _>(
            McpTool::read_only("list_assets").needs(Needs::Project),
            list_assets,
        );
}

type StatusInputs = (
    Res<'static, ProjectState>,
    Res<'static, ActiveEditor>,
    Res<'static, ViewportShown>,
    Query<
        'static,
        'static,
        (
            Entity,
            &'static EditorDocument,
            Option<&'static Unsaved>,
            Option<&'static PlayState>,
        ),
    >,
    BusyInputs,
);

fn status(
    mut calls: ToolCalls<Status>,
    editor: StaticSystemInput<StatusInputs>,
    targets: Query<&HostedApp, With<EditTarget>>,
) {
    let editing = targets.iter().next().is_some();
    for (_, reply) in calls.read() {
        if !editing {
            reply.ok(describe(&editor, "none"));
        }
    }
}

fn status_of_edit_target(
    mut calls: ToolCalls<Status>,
    editor: StaticSystemInput<StatusInputs>,
    selection: EditWorld<Option<Res<Selection>>>,
    names: EditWorld<Query<&Name>>,
) {
    let selected = match selection.as_deref().and_then(Selection::entity) {
        Some(entity) => match names.get_entity(entity) {
            Some(name) => format!("{} {}", entity.to_id_string(), name.as_str()),
            None => entity.to_id_string(),
        },
        None => "none".to_owned(),
    };
    for (_, reply) in calls.read() {
        reply.ok(describe(&editor, &selected));
    }
}

fn describe(editor: &SystemInputData<StatusInputs>, selection: &str) -> String {
    let (project, active, viewport, documents, busy) = editor;
    let mut lines = vec![
        match &project.project {
            Some(open) => format!("project: {}", open.root.display()),
            None => "project: none".to_owned(),
        },
        format!("project status: {}", project.status),
    ];
    let mut open: Vec<_> = documents.iter().collect();
    open.sort_by_key(|(_, document, _, _)| document.order);
    if open.is_empty() {
        lines.push("documents: none".to_owned());
    } else {
        lines.push("documents:".to_owned());
        for (entity, document, unsaved, play) in open {
            lines.push(describe_document(entity, document, unsaved.is_some(), play));
        }
    }
    lines.push(format!(
        "active: {}",
        active
            .0
            .map_or_else(|| "none".to_owned(), |entity| entity.to_id_string())
    ));
    lines.push(format!("selection: {selection}"));
    lines.push(match viewport.0 {
        Some(view) => format!("viewport: {}x{}", view.size.0, view.size.1),
        None => "viewport: not shown".to_owned(),
    });
    lines.push(format!(
        "busy: {}",
        if is_busy(busy) { "yes" } else { "no" }
    ));
    lines.join("\n")
}

fn describe_document(
    entity: Entity,
    document: &EditorDocument,
    unsaved: bool,
    play: Option<&PlayState>,
) -> String {
    let mut line = format!(
        "  {} {} {}",
        entity.to_id_string(),
        document.asset_type,
        document
            .current
            .as_ref()
            .map_or("(nothing loaded)", |asset| asset.address.as_str())
    );
    if let Some(pending) = &document.pending {
        line.push_str(&format!(" · opening {}", pending.address));
    }
    match play {
        Some(PlayState::Editing) => line.push_str(" · editing"),
        Some(PlayState::Starting) => line.push_str(" · starting"),
        Some(PlayState::Playing) => line.push_str(" · playing"),
        None => {}
    }
    if unsaved {
        line.push_str(" · unsaved");
    }
    if !document.status.is_empty() {
        line.push_str(&format!(" · \"{}\"", document.status));
    }
    line
}

fn list_assets(mut calls: ToolCalls<ListAssets>, state: Res<ProjectState>) {
    for (arguments, reply) in calls.read() {
        if let Some(project) = &state.project {
            reply.ok(list(project, &arguments));
        }
    }
}

fn counted(count: usize) -> String {
    if count == 1 {
        "1 asset".to_owned()
    } else {
        format!("{count} assets")
    }
}

fn list(project: &Project, arguments: &ListAssets) -> String {
    let matches = project.filtered_assets(
        arguments.query.as_deref().unwrap_or(""),
        arguments.folder.as_deref(),
        arguments.kind.as_deref(),
    );
    if matches.is_empty() {
        return format!(
            "No assets match. The project has {}.",
            counted(project.assets.len())
        );
    }
    let found = if matches.len() == 1 {
        "1 asset matches".to_owned()
    } else {
        format!("{} match", counted(matches.len()))
    };
    let limit = arguments.limit.max(1);
    let offset = arguments.offset;
    let shown: Vec<_> = matches.iter().skip(offset).take(limit).collect();
    if shown.is_empty() {
        return format!("{found}; offset {offset} is past the last.");
    }
    let mut lines = vec![format!(
        "{found}; showing {}–{}:",
        offset + 1,
        offset + shown.len()
    )];
    for asset in &shown {
        lines.push(format!(
            "{} · {} · {}",
            asset.address,
            asset.kind,
            asset.id.simple_hex()
        ));
    }
    let next = offset + shown.len();
    if next < matches.len() {
        lines.push(format!(
            "… {} more: list_assets {}",
            matches.len() - next,
            continuation(arguments, next, limit)
        ));
    }
    lines.join("\n")
}

fn continuation(arguments: &ListAssets, offset: usize, limit: usize) -> Value {
    let mut call = Map::new();
    if let Some(folder) = &arguments.folder {
        call.insert("folder".into(), folder.clone().into());
    }
    if let Some(kind) = &arguments.kind {
        call.insert("kind".into(), kind.clone().into());
    }
    call.insert("limit".into(), limit.into());
    call.insert("offset".into(), offset.into());
    if let Some(query) = &arguments.query {
        call.insert("query".into(), query.clone().into());
    }
    Value::Object(call)
}
