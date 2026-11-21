use concerto_ecs::{command::CommandQueue, component::name::Name, entity::Entity, World};

use concerto_foundation::assets::{asset_server::AssetServer, handle::AssetHandle};

use crate::link::SceneLink;
use crate::node_id::SceneNodeId;
use crate::scene::{Scene, SceneLinkData};

/// Nodes and components a spawn leaves out; the default leaves out nothing.
#[derive(Default)]
pub struct SpawnFilter {
    removed_nodes: Vec<usize>,
    removed_components: Vec<RemovedComponent>,
}

pub(crate) struct RemovedComponent {
    pub(crate) node: usize,
    pub(crate) type_name: String,
}

impl SpawnFilter {
    pub(crate) fn new(
        removed_nodes: Vec<usize>,
        removed_components: Vec<RemovedComponent>,
    ) -> Self {
        Self {
            removed_nodes,
            removed_components,
        }
    }

    fn skipped_nodes(&self, scene: &Scene) -> Vec<bool> {
        let mut skipped = vec![false; scene.nodes.len()];
        let mut pending = self.removed_nodes.clone();
        while let Some(index) = pending.pop() {
            let Some(flag) = skipped.get_mut(index) else {
                continue;
            };
            if *flag {
                continue;
            }
            *flag = true;
            pending.extend(scene.nodes[index].children.iter().copied());
        }
        skipped
    }

    fn keeps_component(&self, node: usize, type_name: &str) -> bool {
        !self
            .removed_components
            .iter()
            .any(|removed| removed.node == node && removed.type_name == type_name)
    }
}

/// Entity mapping produced when a [`Scene`] is queued for spawning.
#[derive(Debug, Clone)]
pub struct SpawnedScene {
    /// One runtime entity for each scene node, in source-node order; a node the filter left out has an entity that was despawned.
    pub node_entities: Vec<Entity>,
    /// Runtime entities for nodes that have no valid parent in the scene.
    pub root_entities: Vec<Entity>,
}

/// Queues one entity per scene node, applies allowed serialized components,
/// wires the source hierarchy, and parents every root to `parent`.
///
/// The returned node vector is stable by source index and can be retained by
/// editors for hierarchy selection and inspection.
pub fn spawn_scene(
    cmd: &mut CommandQueue<'_, '_>,
    scene: &Scene,
    parent: Entity,
    filter: &SpawnFilter,
) -> SpawnedScene {
    let skipped = filter.skipped_nodes(scene);
    // The authored name is spawned as a component rather than left in the scene
    // file, so tools reading the live world can still identify the entity.
    let mut node_entities = Vec::with_capacity(scene.nodes.len());
    for (index, node) in scene.nodes.iter().enumerate() {
        node_entities.push(match skipped[index] {
            true => cmd.spawn(()).entity(),
            false => cmd
                .spawn((Name::new(node.name.clone()), SceneNodeId(node.id)))
                .entity(),
        });
    }

    let shared_nodes: std::sync::Arc<[Entity]> = node_entities.clone().into();
    for (index, node) in scene.nodes.iter().enumerate() {
        if skipped[index] {
            continue;
        }
        for component in &node.components {
            if !filter.keeps_component(index, &component.type_name) {
                continue;
            }
            cmd.apply_scene_component(
                component.type_name.clone(),
                component.value.clone(),
                node_entities[index],
                shared_nodes.clone(),
            );
        }
    }

    for (index, node) in scene.nodes.iter().enumerate() {
        if let Some(data) = node.link.as_ref().filter(|_| !skipped[index]) {
            cmd.insert(
                stored_link(data, AssetHandle::weak(data.scene)),
                node_entities[index],
            );
        }
    }

    let has_parent = parented_nodes(scene, &skipped);
    for (index, node) in scene.nodes.iter().enumerate() {
        if skipped[index] {
            continue;
        }
        for &child in &node.children {
            if child < node_entities.len() && !skipped[child] {
                cmd.add_child(node_entities[index], node_entities[child]);
            }
        }
    }

    let root_entities = roots(&node_entities, &has_parent, &skipped);
    for root in &root_entities {
        cmd.add_child(parent, *root);
    }
    for (index, entity) in node_entities.iter().enumerate() {
        if skipped[index] {
            cmd.despawn(*entity);
        }
    }

    SpawnedScene {
        node_entities,
        root_entities,
    }
}

/// [`spawn_scene`] for callers holding the [`World`] itself; the scene exists when this returns.
pub fn spawn_scene_in_world(
    world: &mut World,
    scene: &Scene,
    parent: Entity,
    filter: &SpawnFilter,
) -> SpawnedScene {
    let skipped = filter.skipped_nodes(scene);
    let node_entities: Vec<Entity> = scene
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| match skipped[index] {
            true => world.spawn(()),
            false => world.spawn((Name::new(node.name.clone()), SceneNodeId(node.id))),
        })
        .collect();

    for (index, node) in scene.nodes.iter().enumerate() {
        if skipped[index] {
            continue;
        }
        for component in &node.components {
            if !filter.keeps_component(index, &component.type_name) {
                continue;
            }
            world.apply_scene_component(
                &component.type_name,
                &component.value,
                node_entities[index],
                &node_entities,
            );
        }
    }

    for (index, node) in scene.nodes.iter().enumerate() {
        if let Some(data) = node.link.as_ref().filter(|_| !skipped[index]) {
            let handle = match world.get_resource::<AssetServer>() {
                Some(server) => server.load::<Scene>(data.scene),
                None => AssetHandle::weak(data.scene),
            };
            world.insert(stored_link(data, handle), node_entities[index]);
        }
    }

    let has_parent = parented_nodes(scene, &skipped);
    for (index, node) in scene.nodes.iter().enumerate() {
        if skipped[index] {
            continue;
        }
        for &child in &node.children {
            if child < node_entities.len() && !skipped[child] {
                world
                    .entity_mut(node_entities[index])
                    .add_child(node_entities[child]);
            }
        }
    }

    let root_entities = roots(&node_entities, &has_parent, &skipped);
    for root in &root_entities {
        world.entity_mut(parent).add_child(*root);
    }
    for (index, entity) in node_entities.iter().enumerate() {
        if skipped[index] {
            world.despawn(*entity);
        }
    }

    SpawnedScene {
        node_entities,
        root_entities,
    }
}

fn stored_link(data: &SceneLinkData, handle: AssetHandle<Scene>) -> SceneLink {
    data.changes
        .iter()
        .cloned()
        .fold(SceneLink::new(handle), SceneLink::with_change)
}

fn parented_nodes(scene: &Scene, skipped: &[bool]) -> Vec<bool> {
    let mut has_parent = vec![false; scene.nodes.len()];
    for (index, node) in scene.nodes.iter().enumerate() {
        if skipped[index] {
            continue;
        }
        for &child in &node.children {
            if let Some(flag) = has_parent.get_mut(child) {
                *flag = true;
            }
        }
    }
    has_parent
}

fn roots(node_entities: &[Entity], has_parent: &[bool], skipped: &[bool]) -> Vec<Entity> {
    node_entities
        .iter()
        .enumerate()
        .filter_map(|(index, entity)| (!has_parent[index] && !skipped[index]).then_some(*entity))
        .collect()
}
