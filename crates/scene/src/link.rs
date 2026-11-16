use std::collections::HashMap;

use concerto_ecs::{
    component::name::Name,
    entity::hierarchy::{ChildOf, Children},
    events::{event_channel::EventChannel, event_reader::EventReader},
    reflect::{AppTypeRegistry, ReflectComponent, ReflectSceneComponent},
    resource::{Res, Resource},
    signal::{On, Signal},
    CommandQueue, Component, Entity, Event, Query, World,
};
use concerto_foundation::assets::{
    asset_server::AssetServer,
    asset_store::AssetStore,
    handle::{AssetHandle, AssetLoaded},
    AssetId,
};
use concerto_foundation::transform::Transform;
use concerto_reflect::{Reflect, ReflectDefault};
use log::warn;

use crate::link_path::{child_path, node_paths, strip_prefix};
use crate::scene::{LinkChange, Scene, SceneNode};
use crate::spawner::{spawn_scene_in_world, RemovedComponent, SpawnFilter};

/// A placed scene: spawns its nodes under this entity, applies its changes to them, and spawns them again when the scene loads again.
#[derive(Component)]
#[require(SceneLinkState, Transform)]
pub struct SceneLink {
    scene: AssetHandle<Scene>,
    own: Vec<LinkChange>,
    inherited: Vec<LinkChange>,
    outcomes: Vec<ChangeOutcome>,
    inherited_outcomes: Vec<ChangeOutcome>,
    handoffs: Vec<Handoff>,
    spawned: Vec<Entity>,
    chain: Vec<AssetId>,
}

#[derive(Clone)]
struct Handoff {
    outcome: usize,
    inner: Entity,
    at: usize,
    node_path: String,
    change_path: String,
}

impl SceneLink {
    /// A link to `scene` with no changes.
    pub fn new(scene: AssetHandle<Scene>) -> Self {
        Self {
            scene,
            own: Vec::new(),
            inherited: Vec::new(),
            outcomes: Vec::new(),
            inherited_outcomes: Vec::new(),
            handoffs: Vec::new(),
            spawned: Vec::new(),
            chain: Vec::new(),
        }
    }

    /// This link with `change` added to its own changes.
    pub fn with_change(mut self, change: LinkChange) -> Self {
        self.own.push(change);
        self
    }

    /// The linked scene.
    pub fn scene(&self) -> &AssetHandle<Scene> {
        &self.scene
    }

    /// The changes this link makes, without those an enclosing link hands it.
    pub fn changes(&self) -> &[LinkChange] {
        &self.own
    }

    /// How each of [`changes`](Self::changes) went at the last spawn, in the same order.
    pub fn outcomes(&self) -> &[ChangeOutcome] {
        &self.outcomes
    }

    /// Adds `change` to this link's own changes; it applies at the next spawn.
    pub fn push_change(&mut self, change: LinkChange) {
        self.own.push(change);
    }

    /// Removes the own change at `index`; the linked nodes keep it until the next spawn.
    pub fn remove_change(&mut self, index: usize) -> Option<LinkChange> {
        if index >= self.own.len() {
            return None;
        }
        if index < self.outcomes.len() {
            self.outcomes.remove(index);
            self.handoffs.retain(|handoff| handoff.outcome != index);
            for handoff in &mut self.handoffs {
                if handoff.outcome > index {
                    handoff.outcome -= 1;
                }
            }
        }
        Some(self.own.remove(index))
    }
}

/// Whether a link's change found its node at the last spawn.
#[derive(Clone, Debug, PartialEq)]
pub enum ChangeOutcome {
    Applied,
    Unapplied(String),
}

/// Where a [`SceneLink`] is: `Waiting` spawns (again) once its scene is loaded.
#[derive(Component, Default, Clone, Debug, PartialEq)]
pub enum SceneLinkState {
    #[default]
    Waiting,
    Spawned,
    Refused(String),
}

/// An entity a link spawned from its scene, and its path from that link.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct LinkedNode {
    pub link: Entity,
    pub path: String,
}

/// Puts an entity that is not part of a link under the linked node at `path` from the link it sits on.
#[derive(Component, Reflect, Default, Clone, Debug, PartialEq)]
#[reflect(Component, Default, SceneComponent)]
pub struct LinkParent {
    pub path: String,
}

/// The scene this world's top-level links sit in, so a link back to it is refused.
#[derive(Resource)]
pub struct LinkRoot(pub AssetId);

/// A link finished spawning its scene.
#[derive(Event, Clone)]
pub struct SceneLinkSpawned {
    pub link: Entity,
}

/// A linked node's outermost link and its path from there.
#[derive(Clone, Debug, PartialEq)]
pub struct OwnedPath {
    pub link: Entity,
    pub path: String,
}

/// The outermost link `entity` was spawned by, following nested links outwards, and the path joined on the way.
pub fn owning_link(world: &World, entity: Entity) -> Option<OwnedPath> {
    let node = world.get_component_for_entity::<LinkedNode>(entity)?;
    let mut owned = OwnedPath {
        link: node.link,
        path: node.path.clone(),
    };
    while let Some(outer) = world.get_component_for_entity::<LinkedNode>(owned.link) {
        owned = OwnedPath {
            link: outer.link,
            path: child_path(&outer.path, &owned.path),
        };
    }
    Some(owned)
}

pub(crate) struct SpawnLinks {
    links: Vec<Entity>,
}

impl Signal for SpawnLinks {}

pub(crate) fn request_link_spawns(
    mut loaded: EventReader<AssetLoaded<Scene>>,
    links: Query<(Entity, &mut SceneLink, &mut SceneLinkState)>,
    scenes: Res<AssetStore<Scene>>,
    server: Option<Res<AssetServer>>,
    mut cmd: CommandQueue,
) {
    let reloaded: Vec<AssetId> = loaded.read().map(|event| event.id).collect();
    let mut ready = Vec::new();
    for (entity, mut link, mut state) in links.iter() {
        if let (AssetHandle::Weak(..), Some(server)) = (&link.scene, server.as_deref()) {
            link.scene = server.load::<Scene>(link.scene.id());
        }
        let waiting = matches!(*state, SceneLinkState::Waiting);
        if !waiting && reloaded.contains(&link.scene.id()) {
            **state = SceneLinkState::Waiting;
        }
        if matches!(*state, SceneLinkState::Waiting) && scenes.get(&link.scene).is_some() {
            ready.push(entity);
        }
    }
    if !ready.is_empty() {
        cmd.trigger(SpawnLinks { links: ready });
    }
}

pub(crate) fn spawn_links(on: On<SpawnLinks>, world: &mut World) {
    for &link in &on.signal().links {
        let waiting = world
            .get_component_for_entity::<SceneLinkState>(link)
            .is_some_and(|state| matches!(state, SceneLinkState::Waiting));
        if waiting {
            spawn_link(world, link);
        }
    }
}

struct Crossing {
    change: usize,
    node: usize,
    path: String,
}

impl Crossing {
    fn find(scene: &Scene, paths: &[String], change: usize, path: &str) -> Option<Self> {
        scene
            .nodes
            .iter()
            .enumerate()
            .find_map(|(node, scene_node)| {
                scene_node.link.as_ref()?;
                strip_prefix(path, &paths[node]).map(|path| Crossing { change, node, path })
            })
    }
}

struct Moved {
    entity: Entity,
    path: Option<String>,
}

fn spawn_link(world: &mut World, link: Entity) -> Option<()> {
    let scene_link = world.get_component_for_entity::<SceneLink>(link)?;
    let handle = scene_link.scene.clone();
    let own_count = scene_link.own.len();
    let changes: Vec<LinkChange> = scene_link
        .own
        .iter()
        .chain(&scene_link.inherited)
        .cloned()
        .collect();
    let previous = scene_link.spawned.clone();
    let mut chain = scene_link.chain.clone();
    let id = handle.id();
    let scene = world
        .get_resource::<AssetStore<Scene>>()?
        .get(&handle)?
        .clone();
    if chain.is_empty() {
        chain.extend(world.get_resource::<LinkRoot>().map(|root| root.0));
    }
    let scene_name = scene_address(world, id);
    if chain.contains(&id) {
        let through: Vec<String> = chain.iter().map(|id| scene_address(world, *id)).collect();
        let reason = format!("{scene_name} links itself through {}", through.join(" → "));
        warn!("{}: {reason}", link_name(world, link));
        world.insert(SceneLinkState::Refused(reason), link);
        return None;
    }

    let moved = lift_own_entities(world, link);
    for entity in previous {
        if world.entity_is_valid(entity) {
            world.despawn(entity);
        }
    }

    let paths = node_paths(&scene);
    let mut outcomes = vec![ChangeOutcome::Applied; changes.len()];
    let mut removed_nodes = Vec::new();
    let mut removed_components = Vec::new();
    let mut crossings = Vec::new();
    for (index, change) in changes.iter().enumerate() {
        if let Some(node) = paths.iter().position(|path| path == change.path()) {
            match change {
                LinkChange::RemoveNode { .. } => removed_nodes.push(node),
                LinkChange::RemoveComponent { type_name, .. } => removed_components.extend(
                    stored_names(world, &scene.nodes[node], type_name)
                        .into_iter()
                        .map(|type_name| RemovedComponent { node, type_name }),
                ),
                LinkChange::AddComponent { .. } => {}
            }
            continue;
        }
        match Crossing::find(&scene, &paths, index, change.path()) {
            Some(crossing) => crossings.push(crossing),
            None => {
                warn!(
                    "{}: no node at {} in {scene_name}",
                    link_name(world, link),
                    change.path()
                );
                outcomes[index] = unapplied(change.path(), &scene_name);
            }
        }
    }

    let spawned = spawn_scene_in_world(
        world,
        &scene,
        link,
        &SpawnFilter::new(removed_nodes, removed_components),
    );
    let live: Vec<bool> = spawned
        .node_entities
        .iter()
        .map(|entity| world.entity_is_valid(*entity))
        .collect();
    for (index, entity) in spawned.node_entities.iter().enumerate() {
        if live[index] {
            world.insert(
                LinkedNode {
                    link,
                    path: paths[index].clone(),
                },
                *entity,
            );
        }
    }
    let spawned_nodes: Vec<Entity> = spawned
        .node_entities
        .iter()
        .zip(&live)
        .filter_map(|(entity, live)| live.then_some(*entity))
        .collect();
    if let Some(scene_link) = world.get_component_for_entity_mut::<SceneLink>(link) {
        scene_link.spawned = spawned_nodes;
    }

    for change in &changes {
        let LinkChange::AddComponent { path, component } = change else {
            continue;
        };
        let Some(node) = paths.iter().position(|known| known == path) else {
            continue;
        };
        if live[node] {
            world.apply_scene_component(
                &component.type_name,
                &component.value,
                spawned.node_entities[node],
                &spawned.node_entities,
            );
        }
    }

    let mut handoffs = Vec::new();
    for (index, node) in scene.nodes.iter().enumerate() {
        if node.link.is_none() || !live[index] {
            continue;
        }
        let inner = spawned.node_entities[index];
        let handed: Vec<&Crossing> = crossings
            .iter()
            .filter(|crossing| crossing.node == index)
            .collect();
        handoffs.extend(handed.iter().enumerate().map(|(at, crossing)| Handoff {
            outcome: crossing.change,
            inner,
            at,
            node_path: paths[index].clone(),
            change_path: changes[crossing.change].path().to_owned(),
        }));
        let inherited = handed
            .iter()
            .map(|crossing| with_path(&changes[crossing.change], crossing.path.clone()))
            .collect();
        let mut inner_chain = chain.clone();
        inner_chain.push(id);
        let Some(inner_link) = world.get_component_for_entity_mut::<SceneLink>(inner) else {
            continue;
        };
        inner_link.inherited = inherited;
        inner_link.chain = inner_chain;
        spawn_link(world, inner);
    }

    if let Some(scene_link) = world.get_component_for_entity_mut::<SceneLink>(link) {
        scene_link.inherited_outcomes = outcomes.split_off(own_count);
        scene_link.outcomes = outcomes;
        scene_link.handoffs = handoffs;
    }
    settle_handoffs(world, link);
    place_own_entities(world, link, &moved, &scene_name);

    world.insert(SceneLinkState::Spawned, link);
    if let Some(channel) = world.get_resource_mut::<EventChannel<SceneLinkSpawned>>() {
        channel.push_event(SceneLinkSpawned { link });
    }
    settle_outer_links(world, link);
    Some(())
}

fn settle_outer_links(world: &mut World, link: Entity) {
    let mut current = link;
    while let Some(outer) = linked_by(world, current) {
        let spawned = world
            .get_component_for_entity::<SceneLinkState>(outer)
            .is_some_and(|state| matches!(state, SceneLinkState::Spawned));
        if !spawned {
            return;
        }
        settle_handoffs(world, outer);
        let Some(scene) = world
            .get_component_for_entity::<SceneLink>(outer)
            .map(|scene_link| scene_link.scene.id())
        else {
            return;
        };
        let scene_name = scene_address(world, scene);
        place_own_entities(world, outer, &[], &scene_name);
        current = outer;
    }
}

fn settle_handoffs(world: &mut World, link: Entity) {
    let Some(scene_link) = world.get_component_for_entity::<SceneLink>(link) else {
        return;
    };
    let handoffs = scene_link.handoffs.clone();
    let scene_name = scene_address(world, scene_link.scene.id());
    let settled: Vec<ChangeOutcome> = handoffs
        .iter()
        .map(|handoff| handed_outcome(world, handoff, &scene_name))
        .collect();
    let Some(scene_link) = world.get_component_for_entity_mut::<SceneLink>(link) else {
        return;
    };
    let own = scene_link.outcomes.len();
    for (handoff, outcome) in handoffs.iter().zip(settled) {
        let slot = match handoff.outcome < own {
            true => scene_link.outcomes.get_mut(handoff.outcome),
            false => scene_link.inherited_outcomes.get_mut(handoff.outcome - own),
        };
        if let Some(slot) = slot {
            *slot = outcome;
        }
    }
}

fn handed_outcome(world: &World, handoff: &Handoff, scene_name: &str) -> ChangeOutcome {
    let lost = || unapplied(&handoff.change_path, scene_name);
    let Some(inner) = world.get_component_for_entity::<SceneLink>(handoff.inner) else {
        return lost();
    };
    match world.get_component_for_entity::<SceneLinkState>(handoff.inner) {
        Some(SceneLinkState::Waiting) => {
            ChangeOutcome::Unapplied(format!("waiting for {} to load", handoff.node_path))
        }
        Some(SceneLinkState::Spawned) => match inner.inherited_outcomes.get(handoff.at) {
            Some(ChangeOutcome::Applied) => ChangeOutcome::Applied,
            _ => lost(),
        },
        _ => lost(),
    }
}

fn unapplied(path: &str, scene_name: &str) -> ChangeOutcome {
    ChangeOutcome::Unapplied(format!("no node at {path} in {scene_name}"))
}

fn with_path(change: &LinkChange, path: String) -> LinkChange {
    match change {
        LinkChange::AddComponent { component, .. } => LinkChange::AddComponent {
            path,
            component: component.clone(),
        },
        LinkChange::RemoveComponent { type_name, .. } => LinkChange::RemoveComponent {
            path,
            type_name: type_name.clone(),
        },
        LinkChange::RemoveNode { .. } => LinkChange::RemoveNode { path },
    }
}

fn stored_names(world: &World, node: &SceneNode, type_name: &str) -> Vec<String> {
    let registry = world.get_resource::<AppTypeRegistry>().map(|r| r.read());
    let resolve = |name: &str| {
        registry
            .as_ref()
            .and_then(|registry| registry.get_by_name(name))
            .map(|registration| registration.type_id())
    };
    let wanted = resolve(type_name);
    node.components
        .iter()
        .filter(|component| {
            component.type_name == type_name
                || (wanted.is_some() && resolve(&component.type_name) == wanted)
        })
        .map(|component| component.type_name.clone())
        .collect()
}

fn lift_own_entities(world: &mut World, link: Entity) -> Vec<Moved> {
    let mut moved = Vec::new();
    let mut pending = children_of(world, link);
    while let Some(entity) = pending.pop() {
        if world
            .get_component_for_entity::<LinkParent>(entity)
            .is_none()
            || linked_by(world, entity) == Some(link)
        {
            pending.extend(children_of(world, entity));
            continue;
        }
        let path = match owned_by(world, link, entity) {
            true => None,
            false => world
                .get_component_for_entity::<ChildOf>(entity)
                .map(ChildOf::parent)
                .and_then(|parent| path_from(world, link, parent)),
        };
        world.entity_mut(link).add_child(entity);
        moved.push(Moved { entity, path });
    }
    moved
}

fn place_own_entities(world: &mut World, link: Entity, moved: &[Moved], scene_name: &str) {
    let crossing: HashMap<Entity, Option<String>> = moved
        .iter()
        .map(|moved| (moved.entity, moved.path.clone()))
        .collect();
    for entity in children_of(world, link) {
        let Some(own_path) = world
            .get_component_for_entity::<LinkParent>(entity)
            .map(|parent| parent.path.clone())
        else {
            continue;
        };
        if linked_by(world, entity) == Some(link) {
            continue;
        }
        let path = match owned_by(world, link, entity) {
            true => Some(own_path),
            false => crossing.get(&entity).cloned().flatten(),
        };
        let Some(path) = path else {
            continue;
        };
        match find_linked(world, link, &path) {
            Some(parent) => {
                world.entity_mut(parent).add_child(entity);
            }
            None if waits_on_inner_link(world, link, &path) => {}
            None => warn!(
                "{}: no node at {path} in {scene_name} for {}",
                link_name(world, link),
                link_name(world, entity)
            ),
        }
    }
}

fn find_linked(world: &World, link: Entity, path: &str) -> Option<Entity> {
    let scene_link = world.get_component_for_entity::<SceneLink>(link)?;
    for &entity in &scene_link.spawned {
        let Some(node) = world.get_component_for_entity::<LinkedNode>(entity) else {
            continue;
        };
        if node.path == path {
            return Some(entity);
        }
        if let Some(rest) = strip_prefix(path, &node.path) {
            if let Some(found) = find_linked(world, entity, &rest) {
                return Some(found);
            }
        }
    }
    None
}

fn waits_on_inner_link(world: &World, link: Entity, path: &str) -> bool {
    let Some(scene_link) = world.get_component_for_entity::<SceneLink>(link) else {
        return false;
    };
    scene_link.spawned.iter().any(|&entity| {
        let Some(rest) = world
            .get_component_for_entity::<LinkedNode>(entity)
            .and_then(|node| strip_prefix(path, &node.path))
        else {
            return false;
        };
        match world.get_component_for_entity::<SceneLinkState>(entity) {
            Some(SceneLinkState::Waiting) => true,
            Some(SceneLinkState::Spawned) => waits_on_inner_link(world, entity, &rest),
            _ => false,
        }
    })
}

fn path_from(world: &World, link: Entity, entity: Entity) -> Option<String> {
    let node = world.get_component_for_entity::<LinkedNode>(entity)?;
    let mut owner = node.link;
    let mut path = node.path.clone();
    while owner != link {
        let outer = world.get_component_for_entity::<LinkedNode>(owner)?;
        path = child_path(&outer.path, &path);
        owner = outer.link;
    }
    Some(path)
}

fn owned_by(world: &World, link: Entity, entity: Entity) -> bool {
    linked_by(world, entity) == linked_by(world, link)
}

fn linked_by(world: &World, entity: Entity) -> Option<Entity> {
    world
        .get_component_for_entity::<LinkedNode>(entity)
        .map(|node| node.link)
}

fn children_of(world: &World, entity: Entity) -> Vec<Entity> {
    world
        .get_component_for_entity::<Children>(entity)
        .map(|children| children.iter().copied().collect())
        .unwrap_or_default()
}

fn link_name(world: &World, entity: Entity) -> String {
    world
        .get_component_for_entity::<Name>(entity)
        .map(|name| name.as_str().to_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| entity.to_id_string())
}

fn scene_address(world: &World, id: AssetId) -> String {
    world
        .get_resource::<AssetServer>()
        .and_then(AssetServer::registry)
        .and_then(|registry| registry.get(id).map(|entry| entry.address.clone()))
        .unwrap_or_else(|| id.simple_hex())
}
