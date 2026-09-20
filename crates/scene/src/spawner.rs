use ecs::{
    command::CommandQueue,
    component::{name::Name, Component},
    entity::Entity,
    query::Query,
    resource::Res,
};
use essential::assets::{asset_store::AssetStore, handle::AssetHandle};
use essential::transform::Transform;

use crate::scene::Scene;

/// Entity mapping produced when a [`Scene`] is queued for spawning.
#[derive(Debug, Clone)]
pub struct SpawnedScene {
    /// One runtime entity for each scene node, in source-node order.
    pub node_entities: Vec<Entity>,
    /// Runtime entities for nodes that have no valid parent in the scene.
    pub root_entities: Vec<Entity>,
}

/// Queues one entity per scene node, applies allowed serialized components,
/// wires the source hierarchy, and parents every root to `parent`.
///
/// The returned node vector is stable by source index and can be retained by
/// editors for hierarchy selection and inspection.
pub fn spawn_scene(cmd: &mut CommandQueue<'_, '_>, scene: &Scene, parent: Entity) -> SpawnedScene {
    // The authored name is spawned as a component rather than left in the scene
    // file, so tools reading the live world can still identify the entity.
    let mut node_entities = Vec::with_capacity(scene.nodes.len());
    for node in &scene.nodes {
        node_entities.push(cmd.spawn(Name::new(node.name.clone())).entity());
    }

    let shared_nodes: std::sync::Arc<[Entity]> = node_entities.clone().into();
    for (index, node) in scene.nodes.iter().enumerate() {
        for component in &node.components {
            cmd.apply_scene_component(
                component.type_name.clone(),
                component.data.clone(),
                node_entities[index],
                shared_nodes.clone(),
            );
        }
    }

    let mut has_parent = vec![false; scene.nodes.len()];
    for (index, node) in scene.nodes.iter().enumerate() {
        for &child in &node.children {
            let Some(&child_entity) = node_entities.get(child) else {
                continue;
            };
            cmd.add_child(node_entities[index], child_entity);
            has_parent[child] = true;
        }
    }

    let root_entities = node_entities
        .iter()
        .enumerate()
        .filter_map(|(index, entity)| (!has_parent[index]).then_some(*entity))
        .collect::<Vec<_>>();
    for root in &root_entities {
        cmd.add_child(parent, *root);
    }

    SpawnedScene {
        node_entities,
        root_entities,
    }
}

/// Attach to an entity to have [`spawn_scene_components`] expand the referenced
/// [`Scene`] into a child entity hierarchy on the next run.
#[derive(Component)]
pub struct SceneSpawnerComponent(pub AssetHandle<Scene>);

/// Expands every entity carrying a [`SceneSpawnerComponent`] whose scene asset
/// has finished loading into one entity per `SceneNode`. Each node's serialized
/// components are applied generically through the component registry, and the
/// parent/child hierarchy is wired up. Root nodes are parented to the spawner
/// entity so they inherit its transform.
pub fn spawn_scene_components(
    mut cmd: CommandQueue,
    spawners: Query<(Entity, &SceneSpawnerComponent, Option<&Transform>)>,
    scenes: Res<AssetStore<Scene>>,
) {
    for (spawner_entity, spawner, spawner_transform) in spawners.iter() {
        let Some(scene) = scenes.get(&spawner.0) else {
            continue;
        };

        if spawner_transform.is_none() {
            cmd.insert(Transform::IDENTITY, spawner_entity);
        }

        spawn_scene(&mut cmd, scene, spawner_entity);

        cmd.remove::<SceneSpawnerComponent>(spawner_entity);
    }
}
