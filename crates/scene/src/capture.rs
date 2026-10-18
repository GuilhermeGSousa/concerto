use std::collections::HashMap;

use anyhow::bail;
use concerto_ecs::{
    component::name::Name, entity::hierarchy::Children, reflect::SceneCapture, Entity, World,
};

use crate::scene::{Scene, SceneNode, SerializedComponent};

/// Reads every descendant of `root` back into a [`Scene`]; the inverse of
/// [`spawn_scene`](crate::spawner::spawn_scene), listing every asset its handles name.
pub fn capture_scene(world: &World, root: Entity) -> anyhow::Result<Scene> {
    capture(world, child_entities(world, root))
}

/// Like [`capture_scene`], but `entity` itself is node 0.
pub fn capture_subtree(world: &World, entity: Entity) -> anyhow::Result<Scene> {
    capture(world, vec![entity])
}

fn capture(world: &World, roots: Vec<Entity>) -> anyhow::Result<Scene> {
    let mut entities = Vec::new();
    let mut children = Vec::new();
    for root in roots {
        collect(world, root, &mut entities, &mut children);
    }
    let indices: HashMap<Entity, usize> = entities
        .iter()
        .enumerate()
        .map(|(index, entity)| (*entity, index))
        .collect();
    let mut capture = SceneCapture::new(indices);

    let nodes = entities
        .iter()
        .zip(children)
        .map(|(entity, children)| capture_node(world, *entity, children, &mut capture))
        .collect::<anyhow::Result<Vec<_>>>()?;

    Ok(Scene {
        nodes,
        referenced_assets: capture.into_referenced_assets(),
    })
}

fn child_entities(world: &World, entity: Entity) -> Vec<Entity> {
    world
        .get_component_for_entity::<Children>(entity)
        .map(|children| children.iter().copied().collect())
        .unwrap_or_default()
}

fn collect(
    world: &World,
    entity: Entity,
    entities: &mut Vec<Entity>,
    children: &mut Vec<Vec<usize>>,
) -> usize {
    let index = entities.len();
    entities.push(entity);
    children.push(Vec::new());
    for child in child_entities(world, entity) {
        let child_index = collect(world, child, entities, children);
        children[index].push(child_index);
    }
    index
}

fn capture_node(
    world: &World,
    entity: Entity,
    children: Vec<usize>,
    capture: &mut SceneCapture,
) -> anyhow::Result<SceneNode> {
    let name = world
        .get_component_for_entity::<Name>(entity)
        .map(|name| name.as_str().to_owned())
        .unwrap_or_default();

    let mut components = Vec::new();
    for info in world.component_types(entity) {
        let value = match info.capture(world, entity, capture) {
            Ok(value) => value,
            Err(error) => bail!(
                "component '{}' on '{name}' cannot be written to a scene: {error}",
                info.name()
            ),
        };
        components.push(SerializedComponent {
            type_name: info.name().to_owned(),
            value,
        });
    }
    components.sort_by(|a, b| a.type_name.cmp(&b.type_name));

    Ok(SceneNode {
        name,
        children,
        components,
    })
}
