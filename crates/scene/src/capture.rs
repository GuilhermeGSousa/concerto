use std::any::TypeId;
use std::collections::HashMap;

use anyhow::bail;
use concerto_ecs::{
    component::name::Name,
    entity::hierarchy::Children,
    reflect::{AppTypeRegistry, ReflectSceneComponent, SceneCapture},
    Entity, World,
};
use concerto_reflect::ReflectedValue;

use crate::link::{find_linked, owning_link, LinkParent, LinkedNode, SceneLink};
use crate::scene::{LinkChange, Scene, SceneLinkData, SceneNode, SerializedComponent};

/// Reads every descendant of `root` back into a [`Scene`]; the inverse of
/// [`spawn_scene`](crate::spawner::spawn_scene), listing every asset its handles name.
pub fn capture_scene(world: &World, root: Entity) -> anyhow::Result<Scene> {
    capture(world, child_entities(world, root))
}

/// Like [`capture_scene`], but `entity` itself is node 0.
pub fn capture_subtree(world: &World, entity: Entity) -> anyhow::Result<Scene> {
    capture(world, vec![entity])
}

struct Collected {
    entity: Entity,
    children: Vec<usize>,
    link_parent: Option<String>,
}

#[derive(Clone)]
struct Place {
    parent: Option<usize>,
    link_parent: Option<String>,
}

#[derive(Default)]
struct Collector {
    nodes: Vec<Collected>,
    indices: HashMap<Entity, usize>,
}

impl Collector {
    fn collect(&mut self, world: &World, entity: Entity, place: Place) {
        if world
            .get_component_for_entity::<LinkedNode>(entity)
            .is_some()
        {
            let below = owning_link(world, entity)
                .and_then(|owned| {
                    Some(Place {
                        parent: Some(*self.indices.get(&owned.link)?),
                        link_parent: Some(owned.path),
                    })
                })
                .unwrap_or(Place {
                    parent: place.parent,
                    link_parent: None,
                });
            for child in child_entities(world, entity) {
                self.collect(world, child, below.clone());
            }
            return;
        }
        let index = self.nodes.len();
        self.nodes.push(Collected {
            entity,
            children: Vec::new(),
            link_parent: place.link_parent,
        });
        self.indices.insert(entity, index);
        if let Some(parent) = place.parent {
            self.nodes[parent].children.push(index);
        }
        for child in child_entities(world, entity) {
            self.collect(
                world,
                child,
                Place {
                    parent: Some(index),
                    link_parent: None,
                },
            );
        }
    }
}

fn capture(world: &World, roots: Vec<Entity>) -> anyhow::Result<Scene> {
    let mut collector = Collector::default();
    for root in roots {
        collector.collect(
            world,
            root,
            Place {
                parent: None,
                link_parent: None,
            },
        );
    }
    let mut capture = SceneCapture::new(collector.indices);

    let nodes = collector
        .nodes
        .into_iter()
        .map(|collected| capture_node(world, collected, &mut capture))
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

fn capture_node(
    world: &World,
    collected: Collected,
    capture: &mut SceneCapture,
) -> anyhow::Result<SceneNode> {
    let entity = collected.entity;
    let name = world
        .get_component_for_entity::<Name>(entity)
        .map(|name| name.as_str().to_owned())
        .unwrap_or_default();

    let mut components = Vec::new();
    for info in world.component_types(entity) {
        if collected.link_parent.is_some()
            && info.registration().type_id() == TypeId::of::<LinkParent>()
        {
            continue;
        }
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
    if let Some(path) = collected.link_parent {
        components.push(SerializedComponent {
            type_name: std::any::type_name::<LinkParent>().to_owned(),
            value: concerto_reflect::to_value(&LinkParent { path })?,
        });
    }
    components.sort_by(|a, b| a.type_name.cmp(&b.type_name));

    let link = match world.get_component_for_entity::<SceneLink>(entity) {
        Some(link) => Some(link_data(world, entity, link, capture)?),
        None => None,
    };

    Ok(SceneNode {
        name,
        children: collected.children,
        components,
        link,
    })
}

fn link_data(
    world: &World,
    entity: Entity,
    link: &SceneLink,
    capture: &mut SceneCapture,
) -> anyhow::Result<SceneLinkData> {
    capture.reference(link.scene().id());
    let changes = link
        .changes()
        .iter()
        .map(|change| match change {
            LinkChange::AddComponent { path, component } => Ok(LinkChange::AddComponent {
                path: path.clone(),
                component: SerializedComponent {
                    type_name: component.type_name.clone(),
                    value: added_value(world, entity, path, component, capture)?,
                },
            }),
            other => Ok(other.clone()),
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(SceneLinkData {
        scene: link.scene().id(),
        changes,
    })
}

fn added_value(
    world: &World,
    link: Entity,
    path: &str,
    component: &SerializedComponent,
    capture: &mut SceneCapture,
) -> anyhow::Result<ReflectedValue> {
    let Some(target) = find_linked(world, link, path) else {
        return Ok(component.value.clone());
    };
    let Some(scene) = world
        .get_resource::<AppTypeRegistry>()
        .and_then(|registry| registry.read().get_by_name(&component.type_name))
        .and_then(|registration| registration.data::<ReflectSceneComponent>().copied())
    else {
        return Ok(component.value.clone());
    };
    match (scene.capture)(world, target, capture) {
        Some(Ok(value)) => Ok(value),
        Some(Err(error)) => bail!(
            "component '{}' added at {path} cannot be written to a scene: {error}",
            component.type_name
        ),
        None => Ok(component.value.clone()),
    }
}
