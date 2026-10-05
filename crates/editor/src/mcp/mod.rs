//! The editor's MCP tools: the verbs a person has in the editor, for an agent.
//!
//! Every tool here is a plain function of `&mut World` and its arguments, run by
//! `concerto_mcp::serve_requests` at the start of a frame. Tools change the
//! editor only through the same resources its panels write — `Selection`,
//! `EditorCommands`, `FlyCamera`, `ViewportCommands` — so an agent and a person
//! drive one editor, not two.
//!
//! Entities are named by [`Entity::to_id_string`] (`42v3`); a stale id is an
//! error, never a different entity.
mod act;
mod observe;

use std::{collections::HashMap, time::Duration};

use concerto_app::{App, Plugin, schedule_groups::Update};
use concerto_ecs::{
    Entity, Query, Res, ResMut, Resource, World,
    component::name::Name,
    entity::hierarchy::{ChildOf, Children},
};
use concerto_foundation::assets::AssetId;
use concerto_mcp::{Handled, McpActivity, McpPlugin, ServerIdentity, ToolError};

use crate::{
    asset_editor::EditorDocument,
    project::{AssetEntry, EditorCommands, Project, ProjectState},
    scene::{SceneRoot, SceneState},
};

/// What the agent is told when it connects.
const INSTRUCTIONS: &str = "\
The Concerto engine's editor, running headless: no window, no panels, the same \
project, asset editors, scene preview, selection and camera.

Start with `open_project` (for example `examples/render-test`), then \
`list_assets kind=Scene` and `open_asset`. `scene_tree` shows the open scene's \
live entities; `inspect` reads an entity's component values. `status` says what \
is open and selected at any time.

Entities are named like `42v3`: index, then generation. An id from before a \
scene was reopened is stale and is rejected; call `scene_tree` again.

Nothing here saves: edits and selections live only as long as the editor.";

pub fn identity() -> ServerIdentity {
    ServerIdentity {
        name: "concerto-editor".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        instructions: INSTRUCTIONS.into(),
    }
}

/// Registers [`McpPlugin`] and the editor's tools. Register after
/// [`EditorPlugin`](crate::EditorPlugin).
pub struct EditorMcpPlugin;

impl Plugin for EditorMcpPlugin {
    fn build(&self, app: &mut App) {
        app.register_plugin(McpPlugin)
            .insert_resource(McpActivity::default())
            .add_system(Update, report_activity);
        observe::register(app);
        act::register(app);
    }
}

/// Keeps a headless editor running frames while it is loading something, so a
/// project or scene finishes opening without a tool call to drive it.
fn report_activity(
    project: Res<ProjectState>,
    scene: Res<SceneState>,
    documents: Query<&EditorDocument>,
    mut activity: ResMut<McpActivity>,
) {
    if project.busy() || scene.loading() || documents.iter().any(|doc| doc.pending.is_some()) {
        activity.keep_awake();
    }
}

/// How long opening a project or an asset may take.
const LOAD_TIMEOUT: Duration = Duration::from_secs(60);

/// Runs `then` once no project is opening or queued to open, so a call made
/// while the editor is still starting up waits rather than fails.
fn when_project_settled(
    world: &mut World,
    then: impl FnOnce(&mut World) -> Handled + Send + Sync + 'static,
) -> Handled {
    Handled::after(world, LOAD_TIMEOUT, project_settled, then)
}

fn project_settled(world: &World) -> bool {
    let opening = world
        .get_resource::<ProjectState>()
        .is_some_and(ProjectState::busy);
    let queued = world
        .get_resource::<EditorCommands>()
        .is_some_and(|commands| !commands.0.is_empty());
    !opening && !queued
}

/// A resource handle that marks the resource changed when written, as a
/// system's `ResMut` does, so panels and change-detecting systems notice.
fn res_mut<T: Resource>(world: &mut World) -> ResMut<'_, T> {
    assert!(
        world.get_resource::<T>().is_some(),
        "{} is not installed",
        std::any::type_name::<T>()
    );
    ResMut::new(world.as_unsafe_world_cell_mut())
}

/// Parses an entity id and checks it still names a live entity.
fn entity_arg(world: &World, id: &str) -> Result<Entity, ToolError> {
    let entity = Entity::parse_id(id).ok_or_else(|| {
        ToolError::new(
            "invalid_entity",
            format!("`{id}` is not an entity id. Ids look like `42v3`."),
        )
    })?;
    if !world.entity_is_valid(entity) {
        return Err(ToolError::new(
            "stale_entity",
            format!("{id} no longer exists. Call `scene_tree` for current ids."),
        ));
    }
    Ok(entity)
}

fn project(world: &World) -> Result<&Project, ToolError> {
    world
        .get_resource::<ProjectState>()
        .and_then(|state| state.project.as_ref())
        .ok_or_else(|| {
            ToolError::new(
                "no_project",
                "No project is open. Call `open_project` first.",
            )
        })
}

/// Finds an asset by address, by id (hex, with or without dashes), or by a
/// display name that only one asset has.
fn find_asset<'a>(project: &'a Project, query: &str) -> Result<&'a AssetEntry, ToolError> {
    let query = query.trim();
    if let Some(asset) = project.assets.iter().find(|asset| asset.address == query) {
        return Ok(asset);
    }
    if let Ok(id) = AssetId::from_simple_hex(&query.replace('-', ""))
        && let Some(asset) = project.assets.iter().find(|asset| asset.id == id)
    {
        return Ok(asset);
    }
    let named: Vec<_> = project
        .assets
        .iter()
        .filter(|asset| asset.display_name == query)
        .collect();
    match named.as_slice() {
        [asset] => Ok(asset),
        [] => Err(ToolError::new(
            "unknown_asset",
            format!("No asset `{query}`. Pass an address or id from `list_assets`."),
        )),
        several => Err(ToolError::new(
            "ambiguous_asset",
            format!(
                "{} assets are named `{query}`; pass one of their addresses: {}.",
                several.len(),
                several
                    .iter()
                    .take(8)
                    .map(|asset| asset.address.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )),
    }
}

/// Names and hierarchy of every entity, gathered once per call so the rest of
/// a tool can work from `&World`.
struct WorldIndex {
    entities: Vec<Entity>,
    names: HashMap<Entity, String>,
    children: HashMap<Entity, Vec<Entity>>,
    parents: HashMap<Entity, Entity>,
    /// Scene roots in spawn order, with the address they were opened from.
    scene_roots: Vec<(Entity, String)>,
}

impl WorldIndex {
    fn new(world: &mut World) -> Self {
        let mut index = WorldIndex {
            entities: Vec::new(),
            names: HashMap::new(),
            children: HashMap::new(),
            parents: HashMap::new(),
            scene_roots: Vec::new(),
        };
        let mut query =
            world.query::<(Entity, Option<&Name>, Option<&Children>, Option<&ChildOf>), ()>();
        for (entity, name, children, parent) in query.iter(world) {
            index.entities.push(entity);
            if let Some(name) = name.filter(|name| !name.as_str().is_empty()) {
                index.names.insert(entity, name.as_str().to_owned());
            }
            if let Some(children) = children {
                index
                    .children
                    .insert(entity, children.iter().copied().collect());
            }
            if let Some(parent) = parent {
                index.parents.insert(entity, parent.parent());
            }
        }
        let mut roots = world.query::<(Entity, &SceneRoot), ()>();
        index.scene_roots = roots
            .iter(world)
            .map(|(entity, root)| (entity, root.address.clone()))
            .collect();
        index.entities.sort_by_key(spawn_order);
        index
            .scene_roots
            .sort_by_key(|(entity, _)| spawn_order(entity));
        index
    }

    fn name(&self, entity: Entity) -> &str {
        self.names.get(&entity).map_or("(unnamed)", String::as_str)
    }

    fn children(&self, entity: Entity) -> &[Entity] {
        self.children.get(&entity).map_or(&[], Vec::as_slice)
    }

    /// Entities with no parent, in spawn order.
    fn world_roots(&self) -> impl Iterator<Item = Entity> + '_ {
        self.entities
            .iter()
            .copied()
            .filter(|entity| !self.parents.contains_key(entity))
    }

    /// `Hero/Body/Spine`: names from the topmost ancestor down.
    fn path(&self, entity: Entity) -> String {
        let mut names = vec![self.name(entity)];
        let mut current = entity;
        while let Some(&parent) = self.parents.get(&current) {
            names.push(self.name(parent));
            current = parent;
        }
        names.reverse();
        names.join("/")
    }

    /// Whether `entity` is a scene root or below one.
    fn in_scene(&self, entity: Entity) -> bool {
        let mut current = entity;
        loop {
            if self.scene_roots.iter().any(|(root, _)| *root == current) {
                return true;
            }
            match self.parents.get(&current) {
                Some(&parent) => current = parent,
                None => return false,
            }
        }
    }

    fn subtree_size(&self, entity: Entity) -> usize {
        1 + self
            .children(entity)
            .iter()
            .map(|&child| self.subtree_size(child))
            .sum::<usize>()
    }
}

/// Spawn order is not query order; sorting keeps output stable between calls.
fn spawn_order(entity: &Entity) -> (u32, u32) {
    (entity.index(), entity.generation().get())
}

/// Short names of the scene components an entity carries, sorted.
fn component_names(world: &World, entity: Entity) -> Vec<&'static str> {
    let mut names: Vec<_> = world
        .component_types(entity)
        .map(|info| info.short())
        .collect();
    names.sort_unstable();
    names
}
