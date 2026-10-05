//! Tools that read the editor and change nothing.
use std::fmt::Write;

use concerto_app::App;
use concerto_ecs::{Entity, World};
use concerto_mcp::{Handled, McpApp, McpTool, NoArguments, ToolError, ToolOutput};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{WorldIndex, component_names, entity_arg, project, when_project_settled};
use crate::{
    asset_editor::{ActiveEditor, EditorDocument},
    project::ProjectState,
    scene::SceneState,
    selection::{Selection, SelectionKind},
    viewport::{EditorViewport, FlyCamera},
};

pub(super) fn register(app: &mut App) {
    app.add_mcp_tool(
        McpTool::new(
            "status",
            "What the editor has open and selected: project, asset editors, \
             selection, camera pose and viewport size. Cheap; call it whenever \
             unsure what state the editor is in.",
            status,
        )
        .read_only(),
    )
    .add_mcp_tool(
        McpTool::new(
            "list_assets",
            "The open project's imported assets as `address · kind · id` lines, \
             filtered and paged. Kinds include Scene, Mesh, Material, Texture, \
             Skeleton and Animation; only Scene assets open in an editor.",
            list_assets,
        )
        .read_only(),
    )
    .add_mcp_tool(
        McpTool::new(
            "scene_tree",
            "The live entity tree, one `id name [components]` line per entity, \
             indented by depth. Starts at the open scene's root unless `root` is \
             given. Truncated subtrees end with the call that continues them.",
            scene_tree,
        )
        .read_only(),
    )
    .add_mcp_tool(
        McpTool::new(
            "find_entities",
            "Entities whose name contains `name` and/or that carry `component`, \
             with their paths from the root. Searches the open scene unless \
             `all` is set.",
            find_entities,
        )
        .read_only(),
    )
    .add_mcp_tool(
        McpTool::new(
            "inspect",
            "An entity's name, parent, children and the current value of each \
             scene component, as JSON. Values are live: they reflect edits and \
             anything systems changed since the scene loaded.",
            inspect,
        )
        .read_only(),
    );
}

fn status(world: &mut World, _: NoArguments) -> Handled {
    let mut out = String::new();
    let headless = world.get_resource::<crate::dock::PanelRegistry>().is_none();
    let _ = writeln!(
        out,
        "mode: {}",
        if headless { "headless" } else { "windowed" }
    );

    let state = world.get_resource::<ProjectState>().expect("ProjectState");
    match &state.project {
        Some(project) => {
            let _ = writeln!(out, "project: {}", project.root.display());
        }
        None => {
            let _ = writeln!(out, "project: none");
        }
    }
    if !state.status.is_empty() {
        let _ = writeln!(out, "project status: {}", state.status);
    }

    let active = world
        .get_resource::<ActiveEditor>()
        .and_then(|active| active.0);
    let mut documents: Vec<_> = world
        .query::<(Entity, &EditorDocument), ()>()
        .iter(world)
        .map(|(entity, doc)| (entity, doc.clone()))
        .collect();
    documents.sort_by_key(|(_, doc)| doc.order);
    if documents.is_empty() {
        let _ = writeln!(out, "editors: none open");
    } else {
        let _ = writeln!(out, "editors:");
        for (entity, doc) in documents {
            let address = doc
                .current
                .as_ref()
                .map_or("(nothing loaded)", |asset| asset.address.as_str());
            let mut line = format!(
                "  {} {} \"{}\" {}",
                entity.to_id_string(),
                doc.asset_type,
                doc.title,
                address
            );
            if Some(entity) == active {
                line.push_str(" [active]");
            }
            if let Some(pending) = &doc.pending {
                let _ = write!(line, " [opening {}]", pending.address);
            } else if !doc.status.is_empty() {
                let _ = write!(line, " [{}]", doc.status);
            }
            let _ = writeln!(out, "{line}");
        }
    }
    if let Some(scene) = world.get_resource::<SceneState>()
        && !scene.status.is_empty()
    {
        let _ = writeln!(out, "scene status: {}", scene.status);
    }

    let selection = world
        .get_resource::<Selection>()
        .and_then(Selection::current);
    let _ = match selection {
        Some(SelectionKind::Entity(entity)) => {
            let name = world
                .get_component_for_entity::<concerto_ecs::component::name::Name>(entity)
                .map(|name| format!(" \"{}\"", name.as_str()))
                .unwrap_or_default();
            writeln!(out, "selection: entity {}{name}", entity.to_id_string())
        }
        Some(SelectionKind::Asset(id)) => {
            let address = project(world)
                .ok()
                .and_then(|project| project.assets.iter().find(|asset| asset.id == id))
                .map_or_else(|| id.simple_hex(), |asset| asset.address.clone());
            writeln!(out, "selection: asset {address}")
        }
        None => writeln!(out, "selection: none"),
    };

    if let (Some(fly), Some(viewport)) = (
        world.get_resource::<FlyCamera>(),
        world.get_resource::<EditorViewport>(),
    ) {
        let p = fly.position;
        let _ = writeln!(
            out,
            "camera: position [{:.3}, {:.3}, {:.3}], yaw {:.3}, pitch {:.3} · viewport {}x{}",
            p.x + 0.0,
            p.y + 0.0,
            p.z + 0.0,
            fly.yaw + 0.0,
            fly.pitch + 0.0,
            viewport.size[0],
            viewport.size[1]
        );
    }
    ToolOutput::text(out.trim_end()).into()
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListAssets {
    /// Case-insensitive text matched against name, address and kind.
    #[serde(default)]
    query: Option<String>,
    /// Exact kind, e.g. `Scene`.
    #[serde(default)]
    kind: Option<String>,
    /// A folder, e.g. `content/levels`; includes its subfolders.
    #[serde(default)]
    folder: Option<String>,
    #[serde(default = "fifty")]
    limit: usize,
    #[serde(default)]
    offset: usize,
}

fn fifty() -> usize {
    50
}

fn list_assets(world: &mut World, args: ListAssets) -> Handled {
    when_project_settled(world, move |world| list_settled_assets(world, args))
}

fn list_settled_assets(world: &mut World, args: ListAssets) -> Handled {
    let project = match project(world) {
        Ok(project) => project,
        Err(error) => return error.into(),
    };
    let matches = project.filtered_assets(
        args.query.as_deref().unwrap_or(""),
        args.folder.as_deref(),
        args.kind.as_deref(),
    );
    if matches.is_empty() {
        return ToolOutput::text(format!(
            "No assets match. The project has {} assets.",
            project.assets.len()
        ))
        .into();
    }
    let shown: Vec<_> = matches
        .iter()
        .skip(args.offset)
        .take(args.limit.max(1))
        .collect();
    let mut out = format!(
        "{} {} match; showing {}–{}:\n",
        matches.len(),
        if matches.len() == 1 {
            "asset"
        } else {
            "assets"
        },
        (args.offset + 1).min(matches.len()),
        args.offset + shown.len()
    );
    for asset in &shown {
        let _ = writeln!(
            out,
            "{} · {} · {}",
            asset.address,
            asset.kind,
            asset.id.simple_hex()
        );
    }
    let next = args.offset + shown.len();
    if next < matches.len() {
        let _ = writeln!(
            out,
            "… {} more (list_assets offset={next})",
            matches.len() - next
        );
    }
    ToolOutput::text(out.trim_end()).into()
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SceneTree {
    /// Start here instead of at the open scene's root.
    #[serde(default)]
    root: Option<String>,
    /// Levels to print below each starting entity, counting it as the first.
    #[serde(default = "three")]
    depth: usize,
    /// Most entity lines to print.
    #[serde(default = "two_hundred")]
    limit: usize,
    /// Start at every parentless entity, including the editor's own camera,
    /// grid and light, instead of only scene roots.
    #[serde(default)]
    all: bool,
}

fn three() -> usize {
    3
}

fn two_hundred() -> usize {
    200
}

fn scene_tree(world: &mut World, args: SceneTree) -> Handled {
    let index = WorldIndex::new(world);
    let starts: Vec<Entity> = match &args.root {
        Some(id) => match entity_arg(world, id) {
            Ok(entity) => vec![entity],
            Err(error) => return error.into(),
        },
        None if args.all => index.world_roots().collect(),
        None => index
            .scene_roots
            .iter()
            .map(|(entity, _)| *entity)
            .collect(),
    };
    if starts.is_empty() {
        return ToolOutput::text(
            "No scene is open. Open one with `open_asset`, or pass all=true to \
             see the editor's own entities.",
        )
        .into();
    }
    let depth = args.depth.max(1);
    let limit = args.limit.max(1);
    let mut tree = Tree {
        world,
        index: &index,
        depth,
        limit,
        printed: 0,
        out: String::new(),
    };
    for &start in &starts {
        tree.walk(start, 0);
    }
    let total: usize = starts
        .iter()
        .map(|&start| count_within(&index, start, 0, depth))
        .sum();
    let mut out = tree.out;
    if tree.printed < total {
        let mut call = String::from("scene_tree");
        if let Some(root) = &args.root {
            let _ = write!(call, " root={root}");
        }
        if args.all {
            call.push_str(" all=true");
        }
        let _ = writeln!(
            out,
            "… {} more within depth {depth} not shown ({call} limit={}, or pass a deeper entity as root)",
            total - tree.printed,
            total
        );
    }
    ToolOutput::text(out.trim_end()).into()
}

struct Tree<'a> {
    world: &'a World,
    index: &'a WorldIndex,
    depth: usize,
    limit: usize,
    printed: usize,
    out: String,
}

impl Tree<'_> {
    fn walk(&mut self, entity: Entity, level: usize) {
        if self.printed >= self.limit {
            return;
        }
        self.printed += 1;
        let indent = "  ".repeat(level);
        let _ = write!(
            self.out,
            "{indent}{} {} [{}]",
            entity.to_id_string(),
            self.index.name(entity),
            component_names(self.world, entity).join(", ")
        );
        if let Some((_, address)) = self
            .index
            .scene_roots
            .iter()
            .find(|(root, _)| *root == entity)
        {
            let _ = write!(self.out, " (scene {address})");
        }
        self.out.push('\n');
        let children = self.index.children(entity);
        if children.is_empty() {
            return;
        }
        if level + 1 >= self.depth {
            let below = self.index.subtree_size(entity) - 1;
            let _ = writeln!(
                self.out,
                "{indent}  … {below} more below (scene_tree root={})",
                entity.to_id_string()
            );
            return;
        }
        for &child in children {
            self.walk(child, level + 1);
        }
    }
}

/// Entities `scene_tree` would print from `entity` with no line limit.
fn count_within(index: &WorldIndex, entity: Entity, level: usize, depth: usize) -> usize {
    if level >= depth {
        return 0;
    }
    1 + index
        .children(entity)
        .iter()
        .map(|&child| count_within(index, child, level + 1, depth))
        .sum::<usize>()
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct FindEntities {
    /// Case-insensitive text the entity's name must contain.
    #[serde(default)]
    name: Option<String>,
    /// A scene component the entity must carry, by short name (`Light`) or
    /// full type path.
    #[serde(default)]
    component: Option<String>,
    /// Search every entity, not only the open scene.
    #[serde(default)]
    all: bool,
    #[serde(default = "fifty")]
    limit: usize,
}

fn find_entities(world: &mut World, args: FindEntities) -> Handled {
    let index = WorldIndex::new(world);
    let name = args.name.as_deref().map(str::to_lowercase);
    let found: Vec<Entity> = index
        .entities
        .iter()
        .copied()
        .filter(|&entity| args.all || index.in_scene(entity))
        .filter(|&entity| {
            name.as_deref().is_none_or(|name| {
                index
                    .names
                    .get(&entity)
                    .is_some_and(|candidate| candidate.to_lowercase().contains(name))
            })
        })
        .filter(|&entity| {
            args.component.as_deref().is_none_or(|wanted| {
                world
                    .component_types(entity)
                    .any(|info| info.name() == wanted || info.short().eq_ignore_ascii_case(wanted))
            })
        })
        .collect();
    if found.is_empty() {
        let scope = if args.all {
            ""
        } else {
            " in the open scene (pass all=true to search everything)"
        };
        return ToolOutput::text(format!("No entities match{scope}.")).into();
    }
    let limit = args.limit.max(1);
    let mut out = String::new();
    for &entity in found.iter().take(limit) {
        let _ = writeln!(
            out,
            "{} {} [{}]",
            entity.to_id_string(),
            index.path(entity),
            component_names(world, entity).join(", ")
        );
    }
    if found.len() > limit {
        let _ = writeln!(
            out,
            "… {} more (find_entities limit={})",
            found.len() - limit,
            found.len()
        );
    }
    ToolOutput::text(out.trim_end()).into()
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Inspect {
    /// The entity, e.g. `42v3`.
    entity: String,
    /// Only these components, by short name or full type path.
    #[serde(default)]
    components: Option<Vec<String>>,
}

/// Children listed before `inspect` summarises the rest as a count.
const INSPECT_CHILDREN: usize = 50;

fn inspect(world: &mut World, args: Inspect) -> Handled {
    let entity = match entity_arg(world, &args.entity) {
        Ok(entity) => entity,
        Err(error) => return error.into(),
    };
    let index = WorldIndex::new(world);
    let infos: Vec<_> = world.component_types(entity).collect();
    if let Some(wanted) = &args.components {
        let missing: Vec<_> = wanted
            .iter()
            .filter(|wanted| {
                !infos
                    .iter()
                    .any(|info| info.name() == *wanted || info.short().eq_ignore_ascii_case(wanted))
            })
            .collect();
        if !missing.is_empty() {
            return ToolError::new(
                "unknown_component",
                format!(
                    "{} has no {}. It has: {}.",
                    args.entity,
                    missing
                        .iter()
                        .map(|name| format!("`{name}`"))
                        .collect::<Vec<_>>()
                        .join(", "),
                    component_names(world, entity).join(", ")
                ),
            )
            .into();
        }
    }

    let mut components = Map::new();
    for info in &infos {
        if let Some(wanted) = &args.components
            && !wanted
                .iter()
                .any(|wanted| info.name() == wanted || info.short().eq_ignore_ascii_case(wanted))
        {
            continue;
        }
        // Short names unless two components share one.
        let shared = infos
            .iter()
            .filter(|other| other.short() == info.short())
            .count()
            > 1;
        let key = if shared { info.name() } else { info.short() };
        let value = info
            .read(world, entity)
            .unwrap_or_else(|| json!({ "unavailable": "this component does not serialise" }));
        components.insert(key.to_owned(), value);
    }

    let children = index.children(entity);
    let mut report = Map::new();
    report.insert("entity".into(), json!(entity.to_id_string()));
    report.insert("name".into(), json!(index.names.get(&entity)));
    report.insert("path".into(), json!(index.path(entity)));
    report.insert(
        "parent".into(),
        json!(index.parents.get(&entity).map(Entity::to_id_string)),
    );
    report.insert(
        "children".into(),
        json!(
            children
                .iter()
                .take(INSPECT_CHILDREN)
                .map(Entity::to_id_string)
                .collect::<Vec<_>>()
        ),
    );
    if children.len() > INSPECT_CHILDREN {
        report.insert("children_total".into(), json!(children.len()));
    }
    report.insert("components".into(), Value::Object(components));
    // Engine plumbing (render mirrors, UI state) has no read side; say it is
    // there rather than let the entity look smaller than it is.
    let hidden = world.component_ids(entity).len() - infos.len();
    if hidden > 0 && args.components.is_none() {
        report.insert("unreadable_components".into(), json!(hidden));
    }
    ToolOutput::json(&Value::Object(report)).into()
}
