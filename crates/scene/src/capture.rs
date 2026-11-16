use std::any::{Any, TypeId};

use anyhow::bail;
use concerto_ecs::{
    component::name::Name,
    entity::{hierarchy::Children, EntityHashMap},
    reflect::{AppTypeRegistry, ReflectSceneComponent, ReflectSceneLeaf, SceneCapture},
    Entity, World,
};
use concerto_foundation::assets::{handle::ReflectAssetHandle, AssetId};
use concerto_reflect::{
    apply_value_with, decode_leaf, LeafError, LeafHook, Reflect, ReflectDefault, ReflectedValue,
    TypeRegistry,
};
use log::warn;

use crate::link::{find_linked, owning_link, shown_name, LinkParent, LinkedNode, SceneLink};
use crate::link_path::LinkPath;
use crate::node_id::{derive_missing_ids, NodeId, SceneNodeId};
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
    link_parent: Option<LinkPath>,
}

#[derive(Clone)]
struct Place {
    parent: Option<usize>,
    link_parent: Option<LinkPath>,
}

#[derive(Default)]
struct Collector {
    nodes: Vec<Collected>,
    indices: EntityHashMap<usize>,
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
            let parks = world
                .get_component_for_entity::<SceneLink>(entity)
                .is_some();
            for child in child_entities(world, entity) {
                let parked = parks
                    && world
                        .get_component_for_entity::<LinkParent>(child)
                        .is_some();
                let place = match parked {
                    true => Place {
                        parent: below.parent,
                        link_parent: None,
                    },
                    false => below.clone(),
                };
                self.collect(world, child, place);
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
    let kept: Vec<Option<NodeId>> = collector
        .nodes
        .iter()
        .map(|collected| {
            world
                .get_component_for_entity::<SceneNodeId>(collected.entity)
                .map(|id| id.0)
        })
        .collect();

    let mut nodes = collector
        .nodes
        .into_iter()
        .map(|collected| capture_node(world, collected, &mut capture))
        .collect::<anyhow::Result<Vec<_>>>()?;
    derive_missing_ids(&mut nodes, &kept);

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
            Err(error) => match capture
                .outside()
                .filter(|&entity| owning_link(world, entity).is_some())
            {
                Some(linked) => bail!(
                    "component '{}' on '{name}' references the linked node {}; references into a link are not supported yet",
                    info.name(),
                    shown_name(linked, world.get_component_for_entity::<Name>(linked))
                ),
                None => bail!(
                    "component '{}' on '{name}' cannot be written to a scene: {error}",
                    info.name()
                ),
            },
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
        id: NodeId::default(),
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
    path: &LinkPath,
    component: &SerializedComponent,
    capture: &mut SceneCapture,
) -> anyhow::Result<ReflectedValue> {
    let target = find_linked(world, link, path);
    let shown = target.map_or_else(
        || path.to_string(),
        |target| shown_name(target, world.get_component_for_entity::<Name>(target)),
    );
    let Some(registry) = world.get_resource::<AppTypeRegistry>() else {
        return Ok(unlisted(component, &shown));
    };
    let Some(registration) = registry.read().get_by_name(&component.type_name) else {
        return Ok(unlisted(component, &shown));
    };
    let mut added = SceneCapture::default();
    let live = registration
        .data::<ReflectSceneComponent>()
        .zip(target)
        .and_then(|(scene, target)| (scene.capture)(world, target, &mut added));
    let value = match live {
        Some(Ok(value)) => value,
        Some(Err(_)) if added.outside().is_some() => bail!(
            "{} added to {shown} holds an entity reference; references in added components are not supported yet",
            registration.short_name()
        ),
        Some(Err(error)) => bail!(
            "component '{}' added to {shown} cannot be written to a scene: {error}",
            component.type_name
        ),
        None => {
            let Some(mut value) = registration
                .data::<ReflectDefault>()
                .map(ReflectDefault::default)
            else {
                return Ok(unlisted(component, &shown));
            };
            apply_value_with(
                &mut *value,
                &component.value,
                &mut StoredAssets {
                    registry: &registry.read(),
                    capture: &mut added,
                },
            );
            component.value.clone()
        }
    };
    for id in added.referenced_assets() {
        capture.reference(*id);
    }
    Ok(value)
}

fn unlisted(component: &SerializedComponent, node: &str) -> ReflectedValue {
    warn!(
        "{} added to {node} cannot be decoded, so the assets it names are not listed",
        component.type_name
    );
    component.value.clone()
}

struct StoredAssets<'a> {
    registry: &'a TypeRegistry,
    capture: &'a mut SceneCapture,
}

impl LeafHook for StoredAssets<'_> {
    fn encode(&mut self, _: &dyn Reflect) -> Option<Result<Vec<u8>, LeafError>> {
        None
    }

    fn decode(&mut self, leaf: &mut dyn Reflect, bytes: &[u8]) -> Option<Result<(), LeafError>> {
        let type_id = (&*leaf as &dyn Any).type_id();
        if self
            .registry
            .get_type_data::<ReflectAssetHandle>(type_id)
            .is_some()
        {
            let mut id = AssetId::nil();
            return Some(decode_leaf(&mut id, bytes).map(|()| self.capture.reference(id)));
        }
        self.registry
            .get_type_data::<ReflectSceneLeaf>(type_id)
            .map(|_| Ok(()))
    }
}
