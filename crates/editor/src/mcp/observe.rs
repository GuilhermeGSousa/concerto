use std::any::TypeId;

use concerto_app::App;
use concerto_ecs::{
    Entity, IntoSystemConfig, Query, Res,
    component::name::Name,
    entity::hierarchy::{ChildOf, Children},
    query::filter::{With, Without},
    signal::listener::GlobalListener,
    system::input::{ComponentMetadata, StaticSystemInput, SystemInputData},
};
use concerto_foundation::assets::{
    asset_server::AssetServer,
    registry::{AssetEntry, AssetRegistry},
};
use concerto_mcp::{
    AppMcpExt, McpRead, McpServe, McpTool, Needs, Reply, ToolCalls, ToolError, answer_unanswered,
    host::HostPicture, receive_read_only_calls,
};
use concerto_reflect::{PathSegment, PropertyPath, Reflect, ReflectRef, to_json, with_property};
use concerto_render::{
    gpu::Gpu, readback::read_texture, render_asset::render_texture::RenderTexture,
};
use image::{ImageFormat, RgbaImage, imageops::FilterType};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value};

use super::{BusyInputs, act::describe_pose, is_busy};
use crate::{
    asset_editor::{ActiveEditor, EditorDocument, Unsaved},
    hosted::{EditTarget, EditWorld, HostedApp, PlayState, input::ViewportShown},
    inspector::{InspectionSource, InspectorRegistry, Property},
    project::ProjectState,
    tools::{
        camera::FlyCamera, entity_ops::scene_member_by, scene::SceneRoot, selection::Selection,
    },
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

/// One entity's components as JSON, each with its property paths, whether an inspector adapter edits each, and which name an enum to write whole; lists longer than 16 are cut.
#[derive(Deserialize, JsonSchema)]
pub struct Inspect {
    /// The entity, such as 42v3, as scene_tree shows it.
    pub entity: String,
    /// The components to show, by short name such as Transform or by full type path; all of them when absent.
    pub components: Option<Vec<String>>,
}

/// A PNG of the viewport, the scene being edited or the game playing, with the camera's pose while editing; or of the whole editor, panels included.
#[derive(Deserialize, JsonSchema)]
pub struct Screenshot {
    /// What to picture: viewport or editor.
    pub target: ScreenshotTarget,
    /// The longest edge of the PNG in pixels; a larger picture is scaled down to it, a smaller one is kept.
    #[serde(default = "one_thousand_twenty_four")]
    pub max_edge: u32,
}

/// What `screenshot` pictures.
#[derive(Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ScreenshotTarget {
    /// The scene being edited, or the game while it plays, as the viewport draws it; needs an open scene.
    Viewport,
    /// The whole editor window, panels included.
    Editor,
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

fn one_thousand_twenty_four() -> u32 {
    1024
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
        )
        .add_tool::<Inspect, _>(McpTool::read_only("inspect").needs(Needs::Scene), inspect)
        .add_tool::<Screenshot, _>(
            McpTool::read_only("screenshot").needs(Needs::Scene),
            screenshot_editor,
        )
        .add_system(McpServe, screenshot_viewport.before(answer_unanswered))
        .add_system(
            McpRead,
            screenshot_viewport
                .after(receive_read_only_calls)
                .before(answer_unanswered),
        );
}

type StatusInputs = (
    Res<'static, ProjectState>,
    Option<Res<'static, AssetRegistry>>,
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
            reply.ok(describe(&editor, "none", false));
        }
    }
}

fn status_of_edit_target(
    mut calls: ToolCalls<Status>,
    editor: StaticSystemInput<StatusInputs>,
    selection: EditWorld<Option<Res<Selection>>>,
    names: EditWorld<Query<&Name>>,
    assets: EditWorld<Option<Res<AssetServer>>>,
) {
    let selected = match selection.as_deref().and_then(Selection::entity) {
        Some(entity) => match names.get_entity(entity) {
            Some(name) => format!("{} {}", entity.to_id_string(), name.as_str()),
            None => entity.to_id_string(),
        },
        None => "none".to_owned(),
    };
    for (_, reply) in calls.read() {
        reply.ok(describe(
            &editor,
            &selected,
            assets.as_deref().is_some_and(AssetServer::is_loading),
        ));
    }
}

fn describe(editor: &SystemInputData<StatusInputs>, selection: &str, loading: bool) -> String {
    let (project, registry, active, viewport, documents, busy) = editor;
    let mut lines = vec![
        match &project.project {
            Some(open) => format!("project: {}", open.root.display()),
            None => "project: none".to_owned(),
        },
        format!("project status: {}", project.line(registry.as_deref())),
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
        if loading || is_busy(busy) {
            "yes"
        } else {
            "no"
        }
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

fn list_assets(
    mut calls: ToolCalls<ListAssets>,
    state: Res<ProjectState>,
    registry: Res<AssetRegistry>,
) {
    for (arguments, reply) in calls.read() {
        if state.project.is_some() {
            reply.ok(list(&registry, &arguments));
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

fn filtered<'a>(registry: &'a AssetRegistry, arguments: &ListAssets) -> Vec<&'a AssetEntry> {
    let query = arguments.query.as_deref().unwrap_or("").to_lowercase();
    let folder = arguments.folder.as_deref();
    let kind = arguments.kind.as_deref();
    registry
        .iter()
        .filter(|asset| {
            folder.is_none_or(|folder| {
                folder.is_empty()
                    || asset.folder() == folder
                    || asset
                        .folder()
                        .strip_prefix(folder)
                        .is_some_and(|suffix| suffix.starts_with('/'))
            })
        })
        .filter(|asset| kind.is_none_or(|kind| asset.kind == kind))
        .filter(|asset| {
            query.is_empty()
                || asset.name().to_lowercase().contains(&query)
                || asset.address.to_lowercase().contains(&query)
                || asset.kind.to_lowercase().contains(&query)
        })
        .collect()
}

fn list(registry: &AssetRegistry, arguments: &ListAssets) -> String {
    let matches = filtered(registry, arguments);
    if matches.is_empty() {
        return format!(
            "No assets match. The project has {}.",
            counted(registry.iter().count())
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

pub(super) type SceneEntities =
    Query<'static, 'static, (Entity, Option<&'static ChildOf>, Option<&'static SceneRoot>)>;

fn outside_open_scene(entity: Entity, entities: &SystemInputData<SceneEntities>) -> bool {
    entities.iter().any(|(_, _, root)| root.is_some())
        && scene_member_by(entity, |entity| {
            entities
                .get_entity(entity)
                .map(|(_, parent, root)| (root.is_some(), parent.map(ChildOf::parent)))
        })
        .is_err()
}

pub(super) fn resolve(
    id: &str,
    entities: &SystemInputData<SceneEntities>,
) -> Result<Entity, ToolError> {
    let Some(entity) = Entity::parse_id(id) else {
        return Err(ToolError::new(
            "unknown_entity",
            format!("`{id}` is not an entity id; ids look like 42v3, as scene_tree shows them."),
        ));
    };
    if entities.get_entity(entity).is_some() {
        if outside_open_scene(entity, entities) {
            Err(ToolError::new(
                "unknown_entity",
                format!("{id} is not part of the open scene; call scene_tree for the scene's ids."),
            ))
        } else {
            Ok(entity)
        }
    } else if entities
        .iter()
        .any(|(live, ..)| live.index() == entity.index())
    {
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
    entities: EditWorld<SceneEntities>,
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

const LIST_LIMIT: usize = 16;

fn inspect(
    mut calls: ToolCalls<Inspect>,
    source: EditWorld<InspectionSource>,
    entities: EditWorld<SceneEntities>,
    names: EditWorld<Query<&Name>>,
    registry: Res<InspectorRegistry>,
) {
    for (arguments, reply) in calls.read() {
        let answer = resolve(&arguments.entity, &entities)
            .and_then(|entity| inspection(&source, &registry, &names, entity, &arguments));
        match answer {
            Ok(text) => reply.ok(text),
            Err(error) => reply.err(error),
        }
    }
}

pub(super) fn names_component(short: &str, full: &str, name: &str) -> bool {
    short.eq_ignore_ascii_case(name) || full == name
}

fn inspection(
    source: &InspectionSource,
    registry: &InspectorRegistry,
    names: &Query<&Name>,
    entity: Entity,
    arguments: &Inspect,
) -> Result<String, ToolError> {
    let mut carried: Vec<_> = source
        .visible_components(entity)
        .into_iter()
        .filter_map(|(id, short)| Some((short, source.types().get(id)?.type_path(), id)))
        .collect();
    carried.sort_unstable();
    if let Some(requested) = &arguments.components {
        let missing: Vec<&str> = requested
            .iter()
            .filter(|name| {
                !carried
                    .iter()
                    .any(|(short, full, _)| names_component(short, full, name))
            })
            .map(String::as_str)
            .collect();
        if !missing.is_empty() {
            let present: Vec<&str> = carried.iter().map(|(short, _, _)| *short).collect();
            return Err(ToolError::new(
                "unknown_component",
                format!(
                    "{} carries no {}; it carries {}.",
                    entity.to_id_string(),
                    missing.join(", "),
                    if present.is_empty() {
                        "none".to_owned()
                    } else {
                        present.join(", ")
                    }
                ),
            ));
        }
    }
    let components: Vec<Value> = carried
        .into_iter()
        .filter(|(short, full, _)| {
            arguments.components.as_ref().is_none_or(|requested| {
                requested
                    .iter()
                    .any(|name| names_component(short, full, name))
            })
        })
        .map(|(short, full, id)| component_json(source, registry, entity, short, full, id))
        .collect();
    let mut answer = Map::new();
    answer.insert("entity".into(), entity.to_id_string().into());
    if let Some(name) = names.get_entity(entity) {
        answer.insert("name".into(), name.as_str().into());
    }
    answer.insert("components".into(), components.into());
    Ok(Value::Object(answer).to_string())
}

pub(super) fn component_json(
    source: &InspectionSource,
    registry: &InspectorRegistry,
    entity: Entity,
    short: &str,
    full: &str,
    id: TypeId,
) -> Value {
    let mut component = Map::new();
    component.insert("name".into(), short.into());
    component.insert("type".into(), full.into());
    match source.read(entity, id) {
        Some(value) => {
            let types = source.types();
            component.insert("value".into(), bounded(to_json(value, &types)));
            let rows = registry.collect(value, &types);
            let listed = listed_paths(value, &rows);
            let paths: Vec<Value> = shown_paths(&listed)
                .iter()
                .map(|shown| {
                    let listed = match shown {
                        ShownPath::Listed(listed) => listed,
                        ShownPath::Cut { list, total } => return cut_note(list, *total).into(),
                    };
                    let mut path = Map::new();
                    path.insert("path".into(), listed.path.to_string().into());
                    path.insert("adapter".into(), listed.adapter().is_some().into());
                    if listed.names_enum {
                        path.insert("enum".into(), true.into());
                    }
                    Value::Object(path)
                })
                .collect();
            component.insert("paths".into(), paths.into());
        }
        None => {
            component.insert("value".into(), Value::Null);
            component.insert("unavailable".into(), true.into());
        }
    }
    Value::Object(component)
}

pub(super) struct ListedPath<'a> {
    pub(super) path: PropertyPath,
    pub(super) row: Option<&'a Property>,
    pub(super) names_enum: bool,
}

impl<'a> ListedPath<'a> {
    /// The row whose editor writes this path, or `None` when `set_property` writes it whole: an enum, a list, or a path no editor claims.
    pub(super) fn adapter(&self) -> Option<&'a Property> {
        self.row
            .filter(|property| !self.names_enum && !property.is_list() && property.has_editor())
    }
}

pub(super) fn listed_paths<'a>(value: &dyn Reflect, rows: &'a [Property]) -> Vec<ListedPath<'a>> {
    let mut listed: Vec<ListedPath> = Vec::new();
    for row in rows.iter().filter(|row| !row.heading) {
        let segments = row.path.segments();
        for depth in 0..segments.len() {
            let above = PropertyPath::new(segments[..depth].iter().copied());
            if names_enum(value, &above) && listed.iter().all(|listed| listed.path != above) {
                listed.push(ListedPath {
                    path: above,
                    row: None,
                    names_enum: true,
                });
            }
        }
        listed.push(ListedPath {
            path: row.path.clone(),
            row: Some(row),
            names_enum: names_enum(value, &row.path),
        });
    }
    listed
}

/// One entry of a component's `paths`: a listed path, or the note that stands for a long list's cut elements.
pub(super) enum ShownPath<'a> {
    Listed(&'a ListedPath<'a>),
    Cut { list: PropertyPath, total: usize },
}

/// The paths `inspect` shows: each list's first [`LIST_LIMIT`] elements, then a note in place of the rest.
pub(super) fn shown_paths<'a>(listed: &'a [ListedPath<'a>]) -> Vec<ShownPath<'a>> {
    let mut shown: Vec<ShownPath> = Vec::new();
    for entry in listed {
        let segments = entry.path.segments();
        let Some((depth, index)) =
            segments
                .iter()
                .enumerate()
                .find_map(|(depth, segment)| match segment {
                    PathSegment::Index(index) if *index >= LIST_LIMIT => Some((depth, *index)),
                    _ => None,
                })
        else {
            shown.push(ShownPath::Listed(entry));
            continue;
        };
        let list = PropertyPath::new(segments[..depth].iter().copied());
        match shown.last_mut() {
            Some(ShownPath::Cut { list: last, total }) if *last == list => *total = index + 1,
            _ => shown.push(ShownPath::Cut {
                list,
                total: index + 1,
            }),
        }
    }
    shown
}

/// What a [`ShownPath::Cut`] says, in the form `value`'s cut list does.
pub(super) fn cut_note(list: &PropertyPath, total: usize) -> String {
    format!("… {} more of {list}, {total} in all", total - LIST_LIMIT)
}

fn names_enum(value: &dyn Reflect, path: &PropertyPath) -> bool {
    let mut found = false;
    let _ = with_property(value, path, &mut |field| {
        found = matches!(field.reflect_ref(), ReflectRef::Enum(_));
    });
    found
}

fn bounded(value: Value) -> Value {
    match value {
        Value::Array(elements) => {
            let total = elements.len();
            let mut kept: Vec<Value> = elements.into_iter().take(LIST_LIMIT).map(bounded).collect();
            if total > LIST_LIMIT {
                kept.push(format!("… {} more, {total} in all", total - LIST_LIMIT).into());
            }
            Value::Array(kept)
        }
        Value::Object(fields) => Value::Object(
            fields
                .into_iter()
                .map(|(key, value)| (key, bounded(value)))
                .collect(),
        ),
        other => other,
    }
}

fn screenshot_editor(
    mut calls: ToolCalls<Screenshot>,
    gpu: Res<Gpu>,
    host: Option<Res<HostPicture>>,
) {
    for (arguments, reply) in calls.read() {
        if arguments.target == ScreenshotTarget::Editor {
            let drawn = host.as_deref().and_then(|host| host.0.get());
            answer_picture(reply, &gpu, drawn, arguments.max_edge, "");
        }
    }
}

fn screenshot_viewport(
    mut calls: ToolCalls<Screenshot>,
    gpu: Res<Gpu>,
    targets: Query<&HostedApp, With<EditTarget>>,
    fly: EditWorld<Option<Res<FlyCamera>>>,
) {
    let Some(target) = targets.iter().next() else {
        return;
    };
    let pose = fly.as_deref().map(describe_pose).unwrap_or_default();
    for (arguments, reply) in calls.read() {
        if arguments.target == ScreenshotTarget::Viewport {
            let drawn = target.picture().get();
            answer_picture(reply, &gpu, drawn, arguments.max_edge, &pose);
        }
    }
}

fn answer_picture(
    reply: &Reply,
    gpu: &Gpu,
    drawn: Option<RenderTexture>,
    max_edge: u32,
    text: &str,
) {
    if max_edge == 0 {
        reply.err(ToolError::new(
            "invalid_arguments",
            "max_edge is the PNG's longest edge in pixels; give 1 or more.",
        ));
        return;
    }
    let Some(drawn) = drawn else {
        reply.err(ToolError::new(
            "busy",
            "Nothing has been drawn there yet; try again after a call that runs a frame, such as frame.",
        ));
        return;
    };
    let picture = fitted(read_texture(gpu, drawn.texture()), max_edge);
    let mut png = std::io::Cursor::new(Vec::new());
    match picture.write_to(&mut png, ImageFormat::Png) {
        Ok(()) => reply.ok_image(png.get_ref(), text),
        Err(error) => reply.err(ToolError::new(
            "internal",
            format!("The picture did not encode as PNG: {error}"),
        )),
    }
}

fn fitted(picture: RgbaImage, max_edge: u32) -> RgbaImage {
    let (width, height) = picture.dimensions();
    let longest = width.max(height);
    if longest <= max_edge {
        return picture;
    }
    let scaled = |edge: u32| {
        let edge =
            (u64::from(edge) * u64::from(max_edge) + u64::from(longest) / 2) / u64::from(longest);
        u32::try_from(edge).unwrap_or(max_edge).max(1)
    };
    image::imageops::resize(&picture, scaled(width), scaled(height), FilterType::Nearest)
}

#[cfg(test)]
mod tests {
    use concerto_reflect::TypeRegistry;

    use super::*;

    #[derive(Reflect, Default)]
    struct Stride {
        length: f32,
    }

    #[derive(Reflect, Default)]
    struct Track {
        strides: Vec<Stride>,
        grid: Vec<Vec<f32>>,
        gain: f32,
    }

    fn shown(track: &Track) -> Vec<String> {
        let mut types = TypeRegistry::default();
        types.register::<Track>();
        let rows = InspectorRegistry::default().collect(track, &types);
        shown_paths(&listed_paths(track, &rows))
            .iter()
            .map(|shown| match shown {
                ShownPath::Listed(listed) => listed.path.to_string(),
                ShownPath::Cut { list, total } => cut_note(list, *total),
            })
            .collect()
    }

    fn strides(count: usize) -> Vec<Stride> {
        (0..count).map(|_| Stride::default()).collect()
    }

    #[test]
    fn a_struct_elements_heading_is_not_a_path_but_its_fields_are() {
        let track = Track {
            strides: strides(2),
            ..Track::default()
        };

        assert_eq!(
            shown(&track),
            [
                "strides",
                "strides.0.length",
                "strides.1.length",
                "grid",
                "gain"
            ]
        );
    }

    #[test]
    fn a_long_list_shows_its_first_sixteen_elements_and_says_how_many_were_cut() {
        let track = Track {
            strides: strides(20),
            ..Track::default()
        };

        let mut expected = vec!["strides".to_owned()];
        expected.extend((0..16).map(|index| format!("strides.{index}.length")));
        expected.push("… 4 more of strides, 20 in all".to_owned());
        expected.extend(["grid".to_owned(), "gain".to_owned()]);
        assert_eq!(shown(&track), expected);
    }

    #[test]
    fn a_list_of_exactly_sixteen_elements_is_not_cut() {
        let track = Track {
            strides: strides(16),
            ..Track::default()
        };

        assert!(!shown(&track).iter().any(|path| path.contains("more of")));
    }

    #[test]
    fn each_list_is_cut_on_its_own_and_a_cut_element_hides_its_inner_lists() {
        let mut grid = vec![Vec::new(); 18];
        grid[0] = vec![0.0; 17];
        grid[17] = vec![0.0; 17];
        let track = Track {
            grid,
            ..Track::default()
        };

        let paths = shown(&track);
        assert_eq!(paths[0..2], ["strides", "grid"]);
        assert_eq!(paths[2], "grid.0");
        assert_eq!(paths[18], "grid.0.15");
        assert_eq!(paths[19], "… 1 more of grid.0, 17 in all");
        assert_eq!(paths[20], "grid.1");
        assert_eq!(paths[34], "grid.15");
        assert_eq!(paths[35], "… 2 more of grid, 18 in all");
        assert_eq!(paths[36], "gain");
        assert_eq!(paths.len(), 37);
    }
}
