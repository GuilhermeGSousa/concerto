use std::collections::HashMap;

use concerto_ecs::{
    component::name::Name,
    entity::hierarchy::Children,
    events::{event_channel::EventChannel, event_reader::EventReader},
    reflect::{AppTypeRegistry, ReflectComponent, ReflectSceneComponent, SceneCapture},
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
use concerto_reflect::{Reflect, ReflectDefault, ReflectedValue};
use log::warn;

use crate::link_path::LinkPath;
use crate::node_id::NodeId;
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
    spawned: HashMap<NodeId, Entity>,
    roots: Vec<Entity>,
    chain: Vec<AssetId>,
}

#[derive(Clone)]
struct Handoff {
    outcome: usize,
    inner: Entity,
    at: usize,
    node_name: String,
    change_path: LinkPath,
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
            spawned: HashMap::new(),
            roots: Vec::new(),
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

/// An entity a link spawned from its scene, and the id of the scene node it was spawned from.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct LinkedNode {
    pub link: Entity,
    pub node: NodeId,
}

/// Puts an entity that is not part of a link under the linked node at `path` from the link it sits on.
#[derive(Component, Reflect, Default, Clone, Debug, PartialEq)]
#[reflect(Component, Default, SceneComponent)]
pub struct LinkParent {
    pub path: LinkPath,
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
    pub path: LinkPath,
}

/// The outermost link `entity` was spawned by, following nested links outwards, and the path joined on the way.
pub fn owning_link(world: &World, entity: Entity) -> Option<OwnedPath> {
    owning_link_by(entity, |entity| {
        world.get_component_for_entity::<LinkedNode>(entity)
    })
}

/// [`owning_link`], reading each entity's [`LinkedNode`] through `node`.
pub fn owning_link_by<'a>(
    entity: Entity,
    node: impl Fn(Entity) -> Option<&'a LinkedNode>,
) -> Option<OwnedPath> {
    let first = node(entity)?;
    let mut owned = OwnedPath {
        link: first.link,
        path: LinkPath::node(first.node),
    };
    while let Some(outer) = node(owned.link) {
        owned = OwnedPath {
            link: outer.link,
            path: LinkPath::node(outer.node).join(&owned.path),
        };
    }
    Some(owned)
}

/// The name of the node `path` reaches from a link to `scene`, read from the scenes in `scenes`.
pub fn linked_node_name<'a>(
    scenes: &'a AssetStore<Scene>,
    scene: AssetId,
    path: &LinkPath,
) -> Option<&'a str> {
    let mut scene = scenes.get(&AssetHandle::weak(scene))?;
    let (last, through) = path.ids().split_last()?;
    for id in through {
        let node = scene.nodes.iter().find(|node| node.id == *id)?;
        scene = scenes.get(&AssetHandle::weak(node.link.as_ref()?.scene))?;
    }
    scene
        .nodes
        .iter()
        .find(|node| node.id == *last)
        .map(|node| node.name.as_str())
}

pub(crate) struct SpawnLinks {
    links: Vec<Entity>,
}

impl Signal for SpawnLinks {}

pub(crate) fn wait_for_reloaded_scenes(
    mut loaded: EventReader<AssetLoaded<Scene>>,
    links: Query<(&SceneLink, &mut SceneLinkState)>,
) {
    let reloaded: Vec<AssetId> = loaded.read().map(|event| event.id).collect();
    if reloaded.is_empty() {
        return;
    }
    for (link, mut state) in links.iter() {
        if **state != SceneLinkState::Waiting && reloaded.contains(&link.scene.id()) {
            **state = SceneLinkState::Waiting;
        }
    }
}

pub(crate) fn request_link_spawns(
    links: Query<(Entity, &mut SceneLink, &SceneLinkState)>,
    scenes: Res<AssetStore<Scene>>,
    server: Option<Res<AssetServer>>,
    mut cmd: CommandQueue,
) {
    let mut ready = Vec::new();
    for (entity, mut link, state) in links.iter() {
        if let (AssetHandle::Weak(..), Some(server)) = (&link.scene, server.as_deref()) {
            link.scene = server.load::<Scene>(link.scene.id());
        }
        if *state == SceneLinkState::Waiting && scenes.get(&link.scene).is_some() {
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
    path: LinkPath,
}

impl Crossing {
    fn find(
        scene: &Scene,
        index: &HashMap<NodeId, usize>,
        change: usize,
        path: &LinkPath,
    ) -> Option<Self> {
        let (first, rest) = path.ids().split_first()?;
        if rest.is_empty() {
            return None;
        }
        let node = *index.get(first)?;
        scene.nodes[node].link.as_ref()?;
        Some(Crossing {
            change,
            node,
            path: LinkPath::new(rest.to_vec()),
        })
    }
}

fn in_scene(index: &HashMap<NodeId, usize>, path: &LinkPath) -> Option<usize> {
    match path.ids() {
        [id] => index.get(id).copied(),
        _ => None,
    }
}

fn spawn_link(world: &mut World, link: Entity) -> Option<()> {
    refresh_added_values(world, link);
    refresh_handed_values(world, link);
    let scene_link = world.get_component_for_entity::<SceneLink>(link)?;
    let handle = scene_link.scene.clone();
    let own_count = scene_link.own.len();
    let changes: Vec<LinkChange> = scene_link
        .own
        .iter()
        .chain(&scene_link.inherited)
        .cloned()
        .collect();
    let previous = scene_link.roots.clone();
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

    lift_own_entities(world, link);
    for entity in previous {
        if world.entity_is_valid(entity) {
            world.despawn(entity);
        }
    }

    let mut index: HashMap<NodeId, usize> = HashMap::new();
    for (at, node) in scene.nodes.iter().enumerate() {
        if *index.entry(node.id).or_insert(at) != at {
            warn!(
                "{}: {scene_name} has more than one node with id {}; only the first can be changed",
                link_name(world, link),
                node.id
            );
        }
    }
    let mut outcomes = vec![ChangeOutcome::Applied; changes.len()];
    let mut removed_nodes = Vec::new();
    let mut removed_components = Vec::new();
    let mut crossings = Vec::new();
    for (at, change) in changes.iter().enumerate() {
        if let Some(node) = in_scene(&index, change.path()) {
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
        match Crossing::find(&scene, &index, at, change.path()) {
            Some(crossing) => crossings.push(crossing),
            None => {
                warn!(
                    "{}: no node {} in {scene_name}",
                    link_name(world, link),
                    change.path()
                );
                outcomes[at] = unapplied(change.path(), &scene_name);
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
    let mut spawned_nodes = HashMap::new();
    for (at, entity) in spawned.node_entities.iter().enumerate() {
        if live[at] {
            let node = scene.nodes[at].id;
            world.insert(LinkedNode { link, node }, *entity);
            if index.get(&node) == Some(&at) {
                spawned_nodes.insert(node, *entity);
            }
        }
    }
    if let Some(scene_link) = world.get_component_for_entity_mut::<SceneLink>(link) {
        scene_link.spawned = spawned_nodes;
        scene_link.roots = spawned.root_entities.clone();
    }

    let (own, handed) = changes.split_at(own_count);
    let target = |path: &LinkPath| {
        in_scene(&index, path)
            .filter(|&node| live[node])
            .map(|node| spawned.node_entities[node])
    };
    for change in own {
        apply_added(world, change, &target, &spawned.node_entities);
    }
    for change in handed {
        match change {
            LinkChange::RemoveComponent { path, type_name } => {
                if let Some(entity) = target(path) {
                    remove_named(world, type_name, entity);
                }
            }
            LinkChange::AddComponent { .. } => {
                apply_added(world, change, &target, &spawned.node_entities)
            }
            LinkChange::RemoveNode { .. } => {}
        }
    }

    let mut handoffs = Vec::new();
    for (at_node, node) in scene.nodes.iter().enumerate() {
        if node.link.is_none() || !live[at_node] {
            continue;
        }
        let inner = spawned.node_entities[at_node];
        let handed: Vec<&Crossing> = crossings
            .iter()
            .filter(|crossing| crossing.node == at_node)
            .collect();
        handoffs.extend(handed.iter().enumerate().map(|(at, crossing)| Handoff {
            outcome: crossing.change,
            inner,
            at,
            node_name: node_label(node),
            change_path: changes[crossing.change].path().clone(),
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
    place_own_entities(world, link, &scene_name);

    world.insert(SceneLinkState::Spawned, link);
    if let Some(channel) = world.get_resource_mut::<EventChannel<SceneLinkSpawned>>() {
        channel.push_event(SceneLinkSpawned { link });
    }
    settle_outer_links(world, link);
    Some(())
}

fn apply_added(
    world: &mut World,
    change: &LinkChange,
    target: &impl Fn(&LinkPath) -> Option<Entity>,
    node_entities: &[Entity],
) {
    let LinkChange::AddComponent { path, component } = change else {
        return;
    };
    if let Some(entity) = target(path) {
        world.apply_scene_component(
            &component.type_name,
            &component.value,
            entity,
            node_entities,
        );
    }
}

fn remove_named(world: &mut World, type_name: &str, entity: Entity) {
    let remove = world
        .get_resource::<AppTypeRegistry>()
        .and_then(|registry| {
            let registry = registry.read();
            let registration = registry.get_by_name(type_name)?;
            registration
                .data::<ReflectComponent>()
                .map(|component| component.remove)
        });
    if let Some(remove) = remove {
        remove(world, entity);
    }
}

fn refresh_added_values(world: &mut World, link: Entity) {
    let Some(scene_link) = world.get_component_for_entity::<SceneLink>(link) else {
        return;
    };
    let refreshed: Vec<Option<ReflectedValue>> = scene_link
        .own
        .iter()
        .map(|change| match change {
            LinkChange::AddComponent { path, component } => {
                live_value(world, link, path, &component.type_name)
            }
            _ => None,
        })
        .collect();
    let Some(scene_link) = world.get_component_for_entity_mut::<SceneLink>(link) else {
        return;
    };
    for (change, value) in scene_link.own.iter_mut().zip(refreshed) {
        if let (LinkChange::AddComponent { component, .. }, Some(value)) = (change, value) {
            component.value = value;
        }
    }
}

struct Refreshed {
    at: usize,
    value: ReflectedValue,
}

fn refresh_handed_values(world: &mut World, link: Entity) {
    let Some(scene_link) = world.get_component_for_entity::<SceneLink>(link) else {
        return;
    };
    let refreshed: Vec<Refreshed> = scene_link
        .inherited
        .iter()
        .enumerate()
        .filter_map(|(at, change)| match change {
            LinkChange::AddComponent { path, component } => {
                live_value(world, link, path, &component.type_name)
                    .map(|value| Refreshed { at, value })
            }
            _ => None,
        })
        .collect();
    for Refreshed { at, value } in refreshed {
        let mut inner = link;
        let mut handed = at;
        set_added_value(world, inner, Slot::Inherited(handed), &value);
        while let Some(outer) = linked_by(world, inner) {
            let Some(scene_link) = world.get_component_for_entity::<SceneLink>(outer) else {
                break;
            };
            let Some(outcome) = scene_link
                .handoffs
                .iter()
                .find(|handoff| handoff.inner == inner && handoff.at == handed)
                .map(|handoff| handoff.outcome)
            else {
                break;
            };
            let own = scene_link.own.len();
            if outcome < own {
                set_added_value(world, outer, Slot::Own(outcome), &value);
                break;
            }
            handed = outcome - own;
            set_added_value(world, outer, Slot::Inherited(handed), &value);
            inner = outer;
        }
    }
}

enum Slot {
    Own(usize),
    Inherited(usize),
}

fn set_added_value(world: &mut World, link: Entity, slot: Slot, value: &ReflectedValue) {
    let Some(scene_link) = world.get_component_for_entity_mut::<SceneLink>(link) else {
        return;
    };
    let change = match slot {
        Slot::Own(index) => scene_link.own.get_mut(index),
        Slot::Inherited(index) => scene_link.inherited.get_mut(index),
    };
    if let Some(LinkChange::AddComponent { component, .. }) = change {
        component.value = value.clone();
    }
}

fn live_value(
    world: &World,
    link: Entity,
    path: &LinkPath,
    type_name: &str,
) -> Option<ReflectedValue> {
    let scene = *world
        .get_resource::<AppTypeRegistry>()?
        .read()
        .get_by_name(type_name)?
        .data::<ReflectSceneComponent>()?;
    let target = find_linked(world, link, path)?;
    let mut capture = SceneCapture::default();
    (scene.capture)(world, target, &mut capture)?.ok()
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
        place_own_entities(world, outer, &scene_name);
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
            ChangeOutcome::Unapplied(format!("waiting for {} to load", handoff.node_name))
        }
        Some(SceneLinkState::Spawned) => match inner.inherited_outcomes.get(handoff.at) {
            Some(ChangeOutcome::Applied) => ChangeOutcome::Applied,
            _ => lost(),
        },
        _ => lost(),
    }
}

fn unapplied(path: &LinkPath, scene_name: &str) -> ChangeOutcome {
    ChangeOutcome::Unapplied(format!("no node {path} in {scene_name}"))
}

fn node_label(node: &SceneNode) -> String {
    match node.name.is_empty() {
        true => node.id.to_string(),
        false => node.name.clone(),
    }
}

fn with_path(change: &LinkChange, path: LinkPath) -> LinkChange {
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

struct Below {
    entity: Entity,
    parent: Entity,
}

fn lift_own_entities(world: &mut World, link: Entity) {
    let below = |world: &World, parent: Entity| -> Vec<Below> {
        children_of(world, parent)
            .into_iter()
            .map(|entity| Below { entity, parent })
            .collect()
    };
    let mut pending = below(world, link);
    while let Some(Below { entity, parent }) = pending.pop() {
        if path_from(world, link, entity).is_some() {
            pending.extend(below(world, entity));
            continue;
        }
        if parent == link {
            continue;
        }
        let unplaced = world
            .get_component_for_entity::<LinkParent>(entity)
            .is_none();
        if unplaced {
            let path =
                home_link(world, link, entity).and_then(|home| path_from(world, home, parent));
            if let Some(path) = path {
                world.insert(LinkParent { path }, entity);
            }
        }
        world.entity_mut(link).add_child(entity);
    }
}

fn home_link(world: &World, link: Entity, entity: Entity) -> Option<Entity> {
    let level = linked_by(world, entity);
    let mut home = link;
    while linked_by(world, home) != level {
        home = linked_by(world, home)?;
    }
    Some(home)
}

fn place_own_entities(world: &mut World, link: Entity, scene_name: &str) {
    for entity in children_of(world, link) {
        let Some(own_path) = world
            .get_component_for_entity::<LinkParent>(entity)
            .map(|parent| parent.path.clone())
        else {
            continue;
        };
        if path_from(world, link, entity).is_some() {
            continue;
        }
        let path = home_link(world, link, entity).and_then(|home| match home == link {
            true => Some(own_path),
            false => own_path.strip_prefix(&path_from(world, home, link)?),
        });
        let Some(path) = path else {
            continue;
        };
        match find_linked(world, link, &path) {
            Some(parent) => {
                world.entity_mut(parent).add_child(entity);
            }
            None if waits_on_inner_link(world, link, &path) => {}
            None => warn!(
                "{}: no node {path} in {scene_name} for {}",
                link_name(world, link),
                link_name(world, entity)
            ),
        }
    }
}

/// The entity `link` spawned for the node at `path`, following its inner links.
pub fn find_linked(world: &World, link: Entity, path: &LinkPath) -> Option<Entity> {
    let (first, rest) = path.ids().split_first()?;
    let entity = *world
        .get_component_for_entity::<SceneLink>(link)?
        .spawned
        .get(first)?;
    match rest.is_empty() {
        true => Some(entity),
        false => find_linked(world, entity, &LinkPath::new(rest.to_vec())),
    }
}

fn waits_on_inner_link(world: &World, link: Entity, path: &LinkPath) -> bool {
    let Some((first, rest)) = path.ids().split_first() else {
        return false;
    };
    let entity = world
        .get_component_for_entity::<SceneLink>(link)
        .and_then(|scene_link| scene_link.spawned.get(first).copied());
    let Some(entity) = entity.filter(|_| !rest.is_empty()) else {
        return false;
    };
    match world.get_component_for_entity::<SceneLinkState>(entity) {
        Some(SceneLinkState::Waiting) => true,
        Some(SceneLinkState::Spawned) => {
            waits_on_inner_link(world, entity, &LinkPath::new(rest.to_vec()))
        }
        _ => false,
    }
}

fn path_from(world: &World, link: Entity, entity: Entity) -> Option<LinkPath> {
    let node = |entity| world.get_component_for_entity::<LinkedNode>(entity);
    let first = node(entity)?;
    let mut owner = first.link;
    let mut path = LinkPath::node(first.node);
    while owner != link {
        let outer = node(owner)?;
        path = LinkPath::node(outer.node).join(&path);
        owner = outer.link;
    }
    Some(path)
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
    shown_name(entity, world.get_component_for_entity::<Name>(entity))
}

/// How a link or node is named in a message: its name, or its id when it has none.
pub fn shown_name(entity: Entity, name: Option<&Name>) -> String {
    name.map(|name| name.as_str().to_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| entity.to_id_string())
}

/// How a linked scene is named in a message: its address, or its id when the registry does not know it.
pub fn scene_address(world: &World, id: AssetId) -> String {
    world
        .get_resource::<AssetServer>()
        .and_then(AssetServer::registry)
        .and_then(|registry| registry.get(id).map(|entry| entry.address.clone()))
        .unwrap_or_else(|| id.simple_hex())
}
