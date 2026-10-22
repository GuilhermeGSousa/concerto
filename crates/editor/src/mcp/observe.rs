use concerto_app::App;
use concerto_ecs::{
    Entity, IntoSystemConfig, Query, Res,
    component::name::Name,
    entity::hierarchy::{ChildOf, Children},
    query::filter::{With, Without},
    signal::listener::GlobalListener,
    system::input::{ComponentMetadata, StaticSystemInput, SystemInputData},
};
use concerto_mcp::{
    AppMcpExt, McpRead, McpServe, McpTool, Needs, ToolCalls, ToolError, answer_unanswered,
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
    tools::{scene::SceneRoot, selection::Selection},
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

/// The open scene's entities as an indented tree, one `id name [components]` line each; while a game plays, the game's.
#[derive(Deserialize, JsonSchema)]
pub struct SceneTree {
    /// The entity to start from, such as 42v3, instead of the scene's top level.
    pub root: Option<String>,
    /// How many levels to show, counting the first entity's as one.
    #[serde(default = "three")]
    pub depth: usize,
    /// How many entities to show.
    #[serde(default = "two_hundred")]
    pub limit: usize,
}

/// The open scene's entities that match a name and a component, each with its path of names from the top.
#[derive(Deserialize, JsonSchema)]
pub struct FindEntities {
    /// Text to find in the entity's name, ignoring case.
    pub name: Option<String>,
    /// A component the entity carries, by short name such as Light or by full type path.
    pub component: Option<String>,
    /// How many matches to show.
    #[serde(default = "fifty")]
    pub limit: usize,
}

fn fifty() -> usize {
    50
}

fn three() -> usize {
    3
}

fn two_hundred() -> usize {
    200
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
        )
        .add_tool::<SceneTree, _>(
            McpTool::read_only("scene_tree").needs(Needs::Scene),
            scene_tree,
        )
        .add_tool::<FindEntities, _>(
            McpTool::read_only("find_entities").needs(Needs::Scene),
            find_entities,
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

type EditTargetDocument = Query<
    'static,
    'static,
    (&'static EditorDocument, Option<&'static PlayState>),
    With<EditTarget>,
>;

type TopLevel = Query<'static, 'static, Entity, (Without<ChildOf>, Without<GlobalListener>)>;

struct SceneView<'a> {
    starts: Vec<Entity>,
    children: &'a dyn Fn(Entity) -> Vec<Entity>,
    name: &'a dyn Fn(Entity) -> Option<&'a str>,
    label: &'a dyn Fn(Entity) -> String,
    components: &'a dyn Fn(Entity) -> Vec<(&'static str, &'static str)>,
}

impl SceneView<'_> {
    fn line(&self, entity: Entity, label: &str) -> String {
        let names: Vec<_> = (self.components)(entity)
            .into_iter()
            .map(|(short, _)| short)
            .collect();
        format!("{} {label} [{}]", entity.to_id_string(), names.join(", "))
    }

    fn subtree(&self, entity: Entity) -> usize {
        1 + (self.children)(entity)
            .into_iter()
            .map(|child| self.subtree(child))
            .sum::<usize>()
    }

    fn within(&self, entity: Entity, level: usize, depth: usize) -> usize {
        if level >= depth {
            return 0;
        }
        1 + (self.children)(entity)
            .into_iter()
            .map(|child| self.within(child, level + 1, depth))
            .sum::<usize>()
    }
}

fn top_of_scene(
    playing: bool,
    roots: &Query<Entity, With<SceneRoot>>,
    top_level: &SystemInputData<TopLevel>,
) -> Vec<Entity> {
    let mut starts: Vec<Entity> = if playing {
        top_level.iter().collect()
    } else {
        roots.iter().collect()
    };
    starts.sort_by_key(|entity| (entity.index(), entity.generation()));
    starts
}

fn components_of(
    components: &ComponentMetadata,
    entity: Entity,
) -> Vec<(&'static str, &'static str)> {
    let mut found: Vec<_> = components
        .component_ids(entity)
        .iter()
        .filter_map(|&id| components.registration(id))
        .map(|registration| (registration.short_name(), registration.type_path()))
        .collect();
    found.sort_unstable();
    found
}

fn children_of(children: &Query<&Children>, entity: Entity) -> Vec<Entity> {
    children
        .get_entity(entity)
        .map(|children| children.iter().copied().collect())
        .unwrap_or_default()
}

pub(super) fn resolve(id: &str, entities: &Query<Entity>) -> Result<Entity, ToolError> {
    let Some(entity) = Entity::parse_id(id) else {
        return Err(ToolError::new(
            "unknown_entity",
            format!("`{id}` is not an entity id; ids look like 42v3, as scene_tree shows them."),
        ));
    };
    if entities.get_entity(entity).is_some() {
        Ok(entity)
    } else if entities.iter().any(|live| live.index() == entity.index()) {
        Err(ToolError::new(
            "stale_entity",
            format!(
                "{id} is gone and its slot holds another entity; call scene_tree for current ids."
            ),
        ))
    } else {
        Err(ToolError::new(
            "unknown_entity",
            format!("No entity {id}; call scene_tree for current ids."),
        ))
    }
}

fn scene_tree(
    mut calls: ToolCalls<SceneTree>,
    roots: EditWorld<Query<Entity, With<SceneRoot>>>,
    top_level: EditWorld<TopLevel>,
    children: EditWorld<Query<&Children>>,
    names: EditWorld<Query<&Name>>,
    entities: EditWorld<Query<Entity>>,
    components: EditWorld<ComponentMetadata>,
    documents: StaticSystemInput<EditTargetDocument>,
) {
    let Some((document, play)) = documents.iter().next() else {
        return;
    };
    let address = scene_address(document);
    let playing = play == Some(&PlayState::Playing);
    let kids = |entity| children_of(&children, entity);
    let name = |entity| names.get_entity(entity).map(|name| name.as_str());
    let label = |entity| label_of(entity, &roots, &names, address);
    let carried = |entity| components_of(&components, entity);
    for (arguments, reply) in calls.read() {
        let starts = match &arguments.root {
            Some(id) => match resolve(id, &entities) {
                Ok(entity) => vec![entity],
                Err(error) => {
                    reply.err(error);
                    continue;
                }
            },
            None => top_of_scene(playing, &roots, &top_level),
        };
        let view = SceneView {
            starts,
            children: &kids,
            name: &name,
            label: &label,
            components: &carried,
        };
        reply.ok(tree(&view, &arguments));
    }
}

fn find_entities(
    mut calls: ToolCalls<FindEntities>,
    roots: EditWorld<Query<Entity, With<SceneRoot>>>,
    top_level: EditWorld<TopLevel>,
    children: EditWorld<Query<&Children>>,
    names: EditWorld<Query<&Name>>,
    components: EditWorld<ComponentMetadata>,
    documents: StaticSystemInput<EditTargetDocument>,
) {
    let Some((document, play)) = documents.iter().next() else {
        return;
    };
    let address = scene_address(document);
    let playing = play == Some(&PlayState::Playing);
    let kids = |entity| children_of(&children, entity);
    let name = |entity| names.get_entity(entity).map(|name| name.as_str());
    let label = |entity| label_of(entity, &roots, &names, address);
    let carried = |entity| components_of(&components, entity);
    for (arguments, reply) in calls.read() {
        let view = SceneView {
            starts: top_of_scene(playing, &roots, &top_level),
            children: &kids,
            name: &name,
            label: &label,
            components: &carried,
        };
        reply.ok(find(&view, &arguments));
    }
}

fn scene_address(document: &EditorDocument) -> &str {
    document
        .current
        .as_ref()
        .map_or("(scene)", |asset| asset.address.as_str())
}

fn label_of(
    entity: Entity,
    roots: &Query<Entity, With<SceneRoot>>,
    names: &Query<&Name>,
    address: &str,
) -> String {
    if roots.get_entity(entity).is_some() {
        return address.to_owned();
    }
    names
        .get_entity(entity)
        .filter(|name| !name.as_str().is_empty())
        .map_or_else(|| "(unnamed)".to_owned(), |name| name.as_str().to_owned())
}

struct TreeLines<'a, 'b> {
    view: &'a SceneView<'b>,
    depth: usize,
    limit: usize,
    shown: usize,
    lines: Vec<String>,
}

impl TreeLines<'_, '_> {
    fn walk(&mut self, entity: Entity, level: usize) {
        if self.shown >= self.limit {
            return;
        }
        self.shown += 1;
        let indent = "  ".repeat(level);
        let line = self.view.line(entity, &(self.view.label)(entity));
        self.lines.push(format!("{indent}{line}"));
        let children = (self.view.children)(entity);
        if children.is_empty() {
            return;
        }
        if level + 1 >= self.depth {
            let below = self.view.subtree(entity) - 1;
            self.lines.push(format!(
                "{indent}  … {below} more below: scene_tree {}",
                tree_call(Some(entity), self.depth.max(2), self.limit)
            ));
            return;
        }
        for child in children {
            self.walk(child, level + 1);
        }
    }
}

fn tree(view: &SceneView, arguments: &SceneTree) -> String {
    if view.starts.is_empty() {
        return "The scene has no entities.".to_owned();
    }
    let depth = arguments.depth.max(1);
    let mut lines = TreeLines {
        view,
        depth,
        limit: arguments.limit.max(1),
        shown: 0,
        lines: Vec::new(),
    };
    for &start in &view.starts {
        lines.walk(start, 0);
    }
    let total: usize = view
        .starts
        .iter()
        .map(|&start| view.within(start, 0, depth))
        .sum();
    let shown = lines.shown;
    let mut lines = lines.lines;
    if shown < total {
        lines.push(format!(
            "… {} more within depth {depth}: scene_tree {}",
            total - shown,
            tree_call(
                arguments.root.as_ref().and(view.starts.first().copied()),
                depth,
                total
            )
        ));
    }
    lines.join("\n")
}

fn tree_call(root: Option<Entity>, depth: usize, limit: usize) -> Value {
    let mut call = Map::new();
    call.insert("depth".into(), depth.into());
    call.insert("limit".into(), limit.into());
    if let Some(root) = root {
        call.insert("root".into(), root.to_id_string().into());
    }
    Value::Object(call)
}

fn find(view: &SceneView, arguments: &FindEntities) -> String {
    let name = arguments.name.as_deref().map(str::to_lowercase);
    let wanted = arguments.component.as_deref();
    let mut found = Vec::new();
    let mut path = Vec::new();
    for &start in &view.starts {
        search(view, start, &mut path, &mut |entity, path| {
            let named = name.as_deref().is_none_or(|name| {
                (view.name)(entity).is_some_and(|candidate| candidate.to_lowercase().contains(name))
            });
            let carries = wanted.is_none_or(|wanted| {
                (view.components)(entity)
                    .iter()
                    .any(|(short, full)| short.eq_ignore_ascii_case(wanted) || *full == wanted)
            });
            if named && carries {
                found.push(view.line(entity, &path.join(" > ")));
            }
        });
    }
    if found.is_empty() {
        return "No entities match.".to_owned();
    }
    let limit = arguments.limit.max(1);
    let mut lines = vec![if found.len() == 1 {
        "1 entity matches:".to_owned()
    } else {
        format!("{} entities match:", found.len())
    }];
    lines.extend(found.iter().take(limit).cloned());
    if found.len() > limit {
        let mut call = Map::new();
        if let Some(component) = &arguments.component {
            call.insert("component".into(), component.clone().into());
        }
        call.insert("limit".into(), found.len().into());
        if let Some(name) = &arguments.name {
            call.insert("name".into(), name.clone().into());
        }
        lines.push(format!(
            "… {} more: find_entities {}",
            found.len() - limit,
            Value::Object(call)
        ));
    }
    lines.join("\n")
}

fn search(
    view: &SceneView,
    entity: Entity,
    path: &mut Vec<String>,
    visit: &mut dyn FnMut(Entity, &[String]),
) {
    path.push((view.label)(entity));
    visit(entity, path);
    for child in (view.children)(entity) {
        search(view, child, path, visit);
    }
    path.pop();
}
