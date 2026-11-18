//! Scene links in the editor: which edits a linked node refuses, the changes edits record on their link, how links and linked nodes show in the tree and the inspector, and reloading the files a save or an import rewrote.
use std::any::TypeId;

use anyhow::{Context, bail};
use concerto_app::{
    App, Plugin,
    edit_time::AtEditTime,
    schedule_groups::{LateUpdate, Update},
};
use concerto_ecs::{
    Component, Entity, IntoSystemConfig, Query, Ref, Res, World,
    command::CommandQueue,
    component::{name::Name, removed::RemovedComponents},
    entity::{
        EntityHashMap,
        hierarchy::{ChildOf, Children},
    },
    events::{Event, event_reader::EventReader},
    query::{
        change_detection::DetectChanges,
        filter::{Added, Changed, Or, With, Without},
    },
    reflect::{AppTypeRegistry, ReflectSceneComponent, SceneCapture},
    resource::ResMut,
    signal::{
        On, Signal,
        listener::{IntoListener, Listener},
    },
};
use concerto_foundation::assets::{
    AssetId,
    asset_store::AssetStore,
    registry::{AssetEntry, AssetRegistry},
};
use concerto_mcp::{AppMcpExt, McpTool, Needs, PendingCall, ToolCalls, ToolRegistry};
use concerto_reflect::TypeRegistry;
use concerto_scene::{
    link::{
        ChangeOutcome, LinkParent, LinkedNode, OwnedPath, SceneLink, SceneLinkState,
        linked_node_name, owning_link_by, scene_address, shown_name,
    },
    link_path::LinkPath,
    scene::{LinkChange, Scene, SerializedComponent},
};
use concerto_ui::{
    elements::{Surface, prelude::*},
    interaction::UIClick,
    node::{UINode, UIRect},
    theme::UITheme,
};
use concerto_window::input::MouseButton;
use log::warn;
use schemars::JsonSchema;
use serde::Deserialize;
use taffy::AlignItems;

use crate::{
    fonts::{button_icon, glyph, icon_button},
    hierarchy::{RowLook, RowLooks, reconcile_content},
    hosted::{
        EditTarget, EditWorld, EditWorldSignals, HostRole, Hosted, HostedApp, HostedEventsAppExt,
        ParkedEditApp, PendingApp,
    },
    import::{AssetExtracted, ImportFinished},
    inspector::{InspectedComponent, InspectorHeader, ReadOnlyCard, refused},
    mcp::{SceneEntities, resolve},
    scene::SceneSaved,
    tools::{
        edits::{ComponentEdit, ComponentEdited},
        entity_ops::{EntityEdit, EntityEdited, in_open_scene, scene_member, scene_member_by},
        scene::{SceneRoot, SceneStatus, report_edit, write},
    },
};

/// Removes the change at `index` from a link's own changes, and spawns the link again without it. Trigger it in the edit `App`.
#[derive(Clone, Debug, PartialEq)]
pub struct RemoveLinkChange {
    pub link: Entity,
    pub index: usize,
}

impl Signal for RemoveLinkChange {}

/// What one `RemoveLinkChange` did, or the status text saying why it was refused.
#[derive(Event, Clone)]
pub struct LinkChangeRemoved {
    pub request: RemoveLinkChange,
    pub result: Result<(), String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinkedEdit {
    Rename,
    Duplicate,
    SetOwnComponent(TypeId),
    ChangeLink,
}

/// Shows links and their linked nodes in the tree and the inspector, and reloads, in every `App` but a playing game's, the assets of the files a save, an import or an extraction rewrote, so the links to their scenes spawn again.
pub struct LinksPlugin;

impl Plugin for LinksPlugin {
    fn build(&self, app: &mut App) {
        app.forward_hosted_event::<LinkChangeRemoved>()
            .add_system(Update, reload_rewritten_files)
            .add_system(LateUpdate, look_linked_rows.before(reconcile_content))
            .add_system(LateUpdate, lock_linked_cards)
            .add_system(LateUpdate, fill_inspector_headers);
    }

    fn finish(&self, app: &mut App) {
        if app.get_resource::<ToolRegistry>().is_none() {
            return;
        }
        app.add_tool::<RemoveLinkChangeArgs, _>(
            McpTool::new("remove_link_change").needs(Needs::Scene),
            call_remove_link_change,
        )
        .add_system(LateUpdate, answer_link_change_removals);
    }
}

/// Removes one change from a link, by its index in the changes inspect lists for the link, and spawns the link again without it.
#[derive(Deserialize, JsonSchema)]
pub struct RemoveLinkChangeArgs {
    /// The link, such as 42v3, as scene_tree shows it with [link → …].
    pub entity: String,
    /// The change's index, from 0, as inspect lists it under changes.
    pub index: usize,
}

#[derive(Component)]
struct RemovingLinkChange {
    document: Entity,
    request: RemoveLinkChange,
}

fn call_remove_link_change(
    mut calls: ToolCalls<RemoveLinkChangeArgs>,
    entities: EditWorld<SceneEntities>,
    targets: Query<Entity, With<EditTarget>>,
    mut scene: EditWorldSignals,
    mut cmd: CommandQueue,
) {
    let Some(document) = targets.iter().next() else {
        return;
    };
    for (arguments, reply) in calls.read() {
        match resolve(&arguments.entity, &entities) {
            Ok(link) => {
                let request = RemoveLinkChange {
                    link,
                    index: arguments.index,
                };
                scene.trigger(request.clone());
                cmd.spawn((
                    PendingCall::new(reply, "the change to be removed"),
                    RemovingLinkChange { document, request },
                ));
            }
            Err(error) => reply.err(error),
        }
    }
}

fn answer_link_change_removals(
    mut removed: EventReader<Hosted<LinkChangeRemoved>>,
    waiting: Query<(Entity, &PendingCall, &RemovingLinkChange)>,
    names: EditWorld<Query<&Name>>,
    mut cmd: CommandQueue,
) {
    for Hosted { document, event } in removed.read() {
        for (call, pending, removing) in waiting.iter() {
            if removing.document != *document || removing.request != event.request {
                continue;
            }
            match &event.result {
                Ok(()) => {
                    let link = event.request.link;
                    let name = shown_name(link, names.get_entity(link));
                    pending.reply.ok(format!(
                        "Removed change {} from {name}.",
                        event.request.index
                    ));
                }
                Err(reason) => pending.reply.err(refused(reason).into()),
            }
            cmd.despawn(call);
        }
    }
}

fn linked_in_open_scene_with<'a>(
    entity: Entity,
    node: impl Fn(Entity) -> Option<&'a LinkedNode>,
    member: impl FnOnce(Entity) -> anyhow::Result<bool>,
) -> Option<OwnedPath> {
    let owned = owning_link_by(entity, node)?;
    member(owned.link).ok()?;
    Some(owned)
}

pub(crate) fn linked_in_open_scene_by(
    entity: Entity,
    nodes: &Query<&LinkedNode>,
    members: &Query<(Option<&ChildOf>, Option<&SceneRoot>)>,
) -> Option<OwnedPath> {
    linked_in_open_scene_with(
        entity,
        |entity| nodes.get_entity(entity),
        |link| {
            scene_member_by(link, |entity| {
                members
                    .get_entity(entity)
                    .map(|(parent, root)| (root.is_some(), parent.map(ChildOf::parent)))
            })
        },
    )
}

fn added_component(
    link: &SceneLink,
    owned: &OwnedPath,
    component: TypeId,
    types: &TypeRegistry,
) -> Option<usize> {
    link.changes().iter().position(|change| {
        matches!(change, LinkChange::AddComponent { path, component: added }
            if *path == owned.path
                && types.get_by_name(&added.type_name).map(|registration| registration.type_id())
                    == Some(component))
    })
}

fn look_linked_rows(
    mut scene: EditWorld<(
        Query<Entity, With<LinkedNode>>,
        Query<&LinkedNode>,
        Query<(Entity, &SceneLink)>,
        Query<&Children>,
        Query<(Option<&ChildOf>, Option<&SceneRoot>)>,
        Query<
            Entity,
            Or<(
                Changed<LinkedNode>,
                Changed<SceneLink>,
                Changed<Children>,
                Added<SceneRoot>,
            )>,
        >,
        RemovedComponents<Children>,
    )>,
    targeted: Query<Entity, Added<EditTarget>>,
    looks: Option<ResMut<RowLooks>>,
) {
    let Some(mut looks) = looks else {
        return;
    };
    let (linked, nodes, links, children, members, changed, removed) = &mut *scene;
    let removed = removed.read().count() > 0;
    if !removed && changed.iter().next().is_none() && targeted.iter().next().is_none() {
        return;
    }
    let mut wanted: EntityHashMap<RowLook> = links
        .iter()
        .map(|(link, _)| {
            (
                link,
                RowLook {
                    icon: Some(glyph::LINK_SIMPLE),
                    ..RowLook::default()
                },
            )
        })
        .collect();
    for entity in linked.iter() {
        let Some(owned) = linked_in_open_scene_by(entity, nodes, members) else {
            continue;
        };
        let changed = links.get_entity(owned.link).is_some_and(|(_, link)| {
            link.changes()
                .iter()
                .any(|change| *change.path() == owned.path)
        });
        let holds_own = children.get_entity(entity).is_some_and(|children| {
            children
                .iter()
                .any(|child| nodes.get_entity(*child).is_none())
        });
        let look = wanted.entry(entity).or_default();
        look.muted = true;
        if changed || holds_own {
            look.marker = Some(glyph::PENCIL_SIMPLE);
        }
    }
    if looks.0 != wanted {
        looks.0 = wanted;
    }
}

fn lock_linked_cards(
    scene: EditWorld<(
        Query<&LinkedNode>,
        Query<&SceneLink>,
        Query<(Option<&ChildOf>, Option<&SceneRoot>)>,
        Res<AppTypeRegistry>,
    )>,
    cards: Query<(Entity, &InspectedComponent), Without<ReadOnlyCard>>,
    mut cmd: CommandQueue,
) {
    let (nodes, links, members, types) = &*scene;
    let types = types.read();
    for (card, inspected) in cards.iter() {
        let Some(owned) = linked_in_open_scene_by(inspected.scene_entity, nodes, members) else {
            continue;
        };
        let added = links
            .get_entity(owned.link)
            .is_some_and(|link| added_component(link, &owned, inspected.type_id, &types).is_some());
        if !added {
            cmd.insert(ReadOnlyCard, card);
        }
    }
}

fn fill_inspector_headers(
    scene: EditWorld<(
        Query<(Ref<SceneLink>, Ref<SceneLinkState>)>,
        Query<&LinkedNode>,
        Query<(Option<&ChildOf>, Option<&SceneRoot>)>,
        Res<AppTypeRegistry>,
        Option<Res<AssetStore<Scene>>>,
    )>,
    assets: Option<Res<AssetRegistry>>,
    headers: Query<(Entity, Ref<InspectorHeader>, &mut UINode, Option<&Children>)>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    let (links, nodes, members, types, scenes) = &*scene;
    let address = |link: &SceneLink| {
        let id = link.scene().id();
        assets
            .as_deref()
            .and_then(|assets| assets.get(id))
            .map_or_else(|| id.simple_hex(), |entry| entry.address.clone())
    };
    for (header, inspected, mut layout, children) in headers.iter() {
        let scene_entity = inspected.scene_entity;
        let link = links.get_entity(scene_entity).filter(|_| {
            scene_member_by(scene_entity, |entity| {
                members
                    .get_entity(entity)
                    .map(|(parent, root)| (root.is_some(), parent.map(ChildOf::parent)))
            })
            .is_ok()
        });
        let changed = link
            .as_ref()
            .is_some_and(|(link, state)| link.is_changed() || state.is_changed());
        if !inspected.is_added() && !changed {
            continue;
        }
        if children.is_some() {
            cmd.entity(header).despawn_children();
        }
        let linked = linked_in_open_scene_by(scene_entity, nodes, members);
        let visible = link.is_some() || linked.is_some();
        if layout.visible != visible {
            layout.visible = visible;
        }
        if let Some(owned) = linked {
            let scene = links
                .get_entity(owned.link)
                .map(|(link, _)| address(&link))
                .unwrap_or_default();
            spawn_linked_note(&mut cmd, header, &scene, &theme);
        } else if let Some((link, state)) = link {
            let changes = shown_changes(&link, &state, &types.read(), scenes.as_deref());
            spawn_link_changes(
                &mut cmd,
                header,
                LinkCard {
                    link: scene_entity,
                    scene: address(&link),
                    state: state.clone(),
                    changes,
                },
                &theme,
            );
        }
    }
}

fn info_card(theme: &UITheme) -> Surface {
    theme
        .card()
        .column()
        .fixed()
        .gap(theme.spacing_xs)
        .padding(UIRect::axes(theme.spacing_xs + 2.0, theme.spacing_sm))
}

fn spawn_linked_note(cmd: &mut CommandQueue, header: Entity, scene: &str, theme: &UITheme) {
    cmd.entity(header)
        .add_child_with(info_card(theme), |mut card| {
            card.add_child(theme.label(format!("Linked from {scene}")).single_line())
                .add_child(
                    theme
                        .label(format!(
                            "Its own components are read-only. Add a component to change it, or edit {scene}."
                        ))
                        .small()
                        .muted(),
                );
        });
}

struct LinkCard {
    link: Entity,
    scene: String,
    state: SceneLinkState,
    changes: Vec<ShownChange>,
}

fn spawn_link_changes(cmd: &mut CommandQueue, header: Entity, shown: LinkCard, theme: &UITheme) {
    let scene_link = shown.link;
    let card = cmd
        .entity(header)
        .spawn_child(info_card(theme))
        .add_child(theme.label(format!("Link → {}", shown.scene)).single_line())
        .entity();
    match &shown.state {
        SceneLinkState::Spawned => {}
        SceneLinkState::Waiting => {
            cmd.entity(card)
                .add_child(theme.label(state_text(&shown.state)).small().muted());
        }
        SceneLinkState::Refused(_) => {
            cmd.entity(card).add_child(
                theme
                    .label(state_text(&shown.state))
                    .small()
                    .color(theme.error),
            );
        }
    }
    if shown.changes.is_empty() {
        cmd.entity(card)
            .add_child(theme.label("No changes").small().muted());
    }
    for (index, change) in shown.changes.into_iter().enumerate() {
        let what = theme.label(change.what).small();
        let outcome = match change.outcome {
            Shown::Applied => theme.label("applied").small().muted(),
            Shown::Unapplied(reason) => theme
                .label(format!("unapplied: {reason}"))
                .small()
                .color(theme.error),
            Shown::NotApplied(state) => theme
                .label(format!("not applied: link {state}"))
                .small()
                .muted(),
        };
        let text = cmd
            .spawn(theme.column().grow().shrink(1.0).gap(2.0))
            .add_child(what)
            .add_child(outcome)
            .entity();
        let button = cmd
            .spawn((
                icon_button(theme).on_click(remove_change_on_click(scene_link, index)),
                button_icon(theme, glyph::X),
            ))
            .entity();
        let row = cmd
            .spawn(
                theme
                    .row()
                    .fixed()
                    .gap(theme.spacing_xs)
                    .align_items(AlignItems::Center),
            )
            .entity();
        cmd.add_child(row, text);
        cmd.add_child(row, button);
        cmd.add_child(card, row);
    }
}

fn remove_change_on_click(scene_link: Entity, index: usize) -> Listener<UIClick> {
    (move |on: On<UIClick>, mut scene: EditWorldSignals| {
        if on.signal().button == MouseButton::Left {
            scene.trigger(RemoveLinkChange {
                link: scene_link,
                index,
            });
        }
    })
    .into_listener()
}

pub(crate) fn build_tools(app: &mut App) {
    app.register_event::<LinkChangeRemoved>()
        .add_listener(record_link_edits.at_edit_time())
        .add_listener(remove_link_change.at_edit_time())
        .add_system(Update, note_link_edits.at_edit_time());
}

pub(crate) fn refusal(world: &World, entity: Entity, edit: LinkedEdit) -> Option<String> {
    let owned = linked_in_open_scene(world, entity)?;
    if let LinkedEdit::SetOwnComponent(component) = edit
        && own_addition(world, &owned, component).is_some()
    {
        return None;
    }
    let link = world.get_component_for_entity::<SceneLink>(owned.link)?;
    let scene = scene_address(world, link.scene().id());
    Some(format!(
        "{} is a linked node of {scene}; add a component to it, or edit {scene}",
        shown_name(entity, world.get_component_for_entity::<Name>(entity))
    ))
}

fn linked_in_open_scene(world: &World, entity: Entity) -> Option<OwnedPath> {
    linked_in_open_scene_with(
        entity,
        |entity| world.get_component_for_entity::<LinkedNode>(entity),
        |link| scene_member(world, link),
    )
}

fn own_addition(world: &World, owned: &OwnedPath, component: TypeId) -> Option<usize> {
    let link = world.get_component_for_entity::<SceneLink>(owned.link)?;
    let types = world.get_resource::<AppTypeRegistry>()?.read();
    added_component(link, owned, component, &types)
}

fn push_change(world: &mut World, link: Entity, change: LinkChange) {
    if let Some(link) = world.get_component_for_entity_mut::<SceneLink>(link) {
        link.push_change(change);
    }
}

pub(crate) fn record_removal(world: &mut World, entity: Entity) {
    if let Some(owned) = linked_in_open_scene(world, entity) {
        push_change(
            world,
            owned.link,
            LinkChange::RemoveNode { path: owned.path },
        );
    }
}

enum LinkEdit {
    Added { entity: Entity, component: TypeId },
    Removed { entity: Entity, component: TypeId },
    Created { entity: Entity, parent: Entity },
}

struct RecordLinkEdits(Vec<LinkEdit>);

impl Signal for RecordLinkEdits {}

fn note_link_edits(
    mut components: EventReader<ComponentEdited>,
    mut entities: EventReader<EntityEdited>,
    linked: Query<&LinkedNode>,
    mut cmd: CommandQueue,
) {
    let is_linked = |entity: Entity| linked.get_entity(entity).is_some();
    let mut edits: Vec<LinkEdit> = components
        .read()
        .filter(|edited| edited.result.is_ok())
        .filter_map(|edited| match edited.edit {
            ComponentEdit::Add { entity, component } if is_linked(entity) => {
                Some(LinkEdit::Added { entity, component })
            }
            ComponentEdit::Remove { entity, component } if is_linked(entity) => {
                Some(LinkEdit::Removed { entity, component })
            }
            _ => None,
        })
        .collect();
    edits.extend(
        entities
            .read()
            .filter_map(|edited| match (&edited.edit, &edited.result) {
                (
                    EntityEdit::Create {
                        parent: Some(parent),
                    },
                    Ok(Some(entity)),
                ) if is_linked(*parent) => Some(LinkEdit::Created {
                    entity: *entity,
                    parent: *parent,
                }),
                _ => None,
            }),
    );
    if !edits.is_empty() {
        cmd.trigger(RecordLinkEdits(edits));
    }
}

fn record_link_edits(on: On<RecordLinkEdits>, world: &mut World) {
    for edit in &on.signal().0 {
        match *edit {
            LinkEdit::Added { entity, component } => record_added(world, entity, component),
            LinkEdit::Removed { entity, component } => record_removed(world, entity, component),
            LinkEdit::Created { entity, parent } => {
                if let Some(owned) = linked_in_open_scene(world, parent)
                    && world.entity_is_valid(entity)
                {
                    world.insert(LinkParent { path: owned.path }, entity);
                }
            }
        }
    }
}

fn record_added(world: &mut World, entity: Entity, component: TypeId) {
    let Some(owned) = linked_in_open_scene(world, entity) else {
        return;
    };
    let Some(registration) = world
        .get_resource::<AppTypeRegistry>()
        .and_then(|registry| registry.read().get(component))
    else {
        return;
    };
    let captured = registration
        .data::<ReflectSceneComponent>()
        .and_then(|scene| (scene.capture)(world, entity, &mut SceneCapture::default()));
    match captured {
        Some(Ok(value)) => push_change(
            world,
            owned.link,
            LinkChange::AddComponent {
                path: owned.path,
                component: SerializedComponent {
                    type_name: registration.type_path().to_owned(),
                    value,
                },
            },
        ),
        _ => warn!(
            "{} added to {} cannot be written to a scene, so its link does not keep it",
            registration.short_name(),
            shown_name(entity, world.get_component_for_entity::<Name>(entity))
        ),
    }
}

fn record_removed(world: &mut World, entity: Entity, component: TypeId) {
    let Some(owned) = linked_in_open_scene(world, entity) else {
        return;
    };
    if let Some(index) = own_addition(world, &owned, component) {
        if let Some(link) = world.get_component_for_entity_mut::<SceneLink>(owned.link) {
            link.remove_change(index);
        }
        return;
    }
    let Some(type_name) = world
        .get_resource::<AppTypeRegistry>()
        .and_then(|registry| registry.read().get(component))
        .map(|registration| registration.type_path().to_owned())
    else {
        return;
    };
    push_change(
        world,
        owned.link,
        LinkChange::RemoveComponent {
            path: owned.path,
            type_name,
        },
    );
}

fn remove_link_change(on: On<RemoveLinkChange>, world: &mut World) {
    let request = on.signal().clone();
    let result = remove_change(world, &request).map_err(|error| {
        let status = format!("Remove change failed: {error:#}");
        write(world, SceneStatus(status.clone()));
        status
    });
    if result.is_ok() {
        report_edit(world);
    }
    write(world, LinkChangeRemoved { request, result });
}

fn remove_change(world: &mut World, request: &RemoveLinkChange) -> anyhow::Result<()> {
    if !world.entity_is_valid(request.link) {
        bail!("the entity no longer exists");
    }
    in_open_scene(world, request.link)?;
    if let Some(reason) = refusal(world, request.link, LinkedEdit::ChangeLink) {
        bail!(reason);
    }
    let name = shown_name(
        request.link,
        world.get_component_for_entity::<Name>(request.link),
    );
    let link = world
        .get_component_for_entity_mut::<SceneLink>(request.link)
        .with_context(|| format!("{name} is not a link; scene_tree marks links with [link → …]"))?;
    let count = link.changes().len();
    if link.remove_change(request.index).is_none() {
        match count {
            0 => bail!("{name} has no changes"),
            1 => bail!(
                "{name} has only change 0; there is no change {}",
                request.index
            ),
            _ => bail!(
                "{name} has changes 0–{}; there is no change {}",
                count - 1,
                request.index
            ),
        }
    }
    world.insert(SceneLinkState::Waiting, request.link);
    Ok(())
}

struct ShownChange {
    what: String,
    outcome: Shown,
}

enum Shown {
    Applied,
    Unapplied(String),
    NotApplied(String),
}

pub(crate) fn state_text(state: &SceneLinkState) -> String {
    match state {
        SceneLinkState::Spawned => "spawned".to_owned(),
        SceneLinkState::Waiting => "waiting".to_owned(),
        SceneLinkState::Refused(reason) => format!("refused: {reason}"),
    }
}

pub(crate) fn link_row(
    link: &SceneLink,
    state: &SceneLinkState,
    registry: Option<&AssetRegistry>,
) -> String {
    let id = link.scene().id();
    let scene = registry
        .and_then(|registry| registry.get(id))
        .map_or_else(|| id.simple_hex(), |entry| entry.address.clone());
    let changes = match link.changes().len() {
        1 => "1 change".to_owned(),
        count => format!("{count} changes"),
    };
    match state {
        SceneLinkState::Spawned => format!("[link → {scene}] ({changes})"),
        _ => format!("[link → {scene}] ({changes}) — {}", state_text(state)),
    }
}

fn shown_changes(
    link: &SceneLink,
    state: &SceneLinkState,
    types: &TypeRegistry,
    scenes: Option<&AssetStore<Scene>>,
) -> Vec<ShownChange> {
    let short = |type_name: &str| {
        types.get_by_name(type_name).map_or_else(
            || type_name.to_owned(),
            |registration| registration.short_name().to_owned(),
        )
    };
    let node = |path: &LinkPath| {
        scenes
            .and_then(|scenes| linked_node_name(scenes, link.scene().id(), path))
            .filter(|name| !name.is_empty())
            .map_or_else(|| path.to_string(), str::to_owned)
    };
    link.changes()
        .iter()
        .enumerate()
        .map(|(index, change)| ShownChange {
            what: match change {
                LinkChange::AddComponent { path, component } => {
                    format!(
                        "AddComponent {} [{}]",
                        node(path),
                        short(&component.type_name)
                    )
                }
                LinkChange::RemoveComponent { path, type_name } => {
                    format!("RemoveComponent {} [{}]", node(path), short(type_name))
                }
                LinkChange::RemoveNode { path } => format!("RemoveNode {}", node(path)),
            },
            outcome: match (state, link.outcomes().get(index)) {
                (SceneLinkState::Spawned, Some(ChangeOutcome::Unapplied(reason))) => {
                    Shown::Unapplied(reason.clone())
                }
                (SceneLinkState::Spawned, _) => Shown::Applied,
                _ => Shown::NotApplied(state_text(state)),
            },
        })
        .collect()
}

pub(crate) fn change_lines(
    link: &SceneLink,
    state: &SceneLinkState,
    types: &TypeRegistry,
    scenes: Option<&AssetStore<Scene>>,
) -> Vec<String> {
    shown_changes(link, state, types, scenes)
        .into_iter()
        .enumerate()
        .map(|(index, change)| match change.outcome {
            Shown::Applied => format!("{index}: {} — applied", change.what),
            Shown::Unapplied(reason) => format!("{index}: {} — unapplied: {reason}", change.what),
            Shown::NotApplied(state) => {
                format!("{index}: {} — not applied: link {state}", change.what)
            }
        })
        .collect()
}

fn rewritten_files<'a>(
    saved: impl Iterator<Item = &'a SceneSaved>,
    imported: impl Iterator<Item = &'a ImportFinished>,
    extracted: impl Iterator<Item = &'a AssetExtracted>,
) -> Vec<AssetId> {
    let saved = saved.flat_map(|saved| std::iter::once(&saved.asset).chain(&saved.written));
    let imported = imported
        .filter_map(|finished| finished.result.as_ref().ok())
        .flat_map(|source| &source.assets);
    let mut files: Vec<AssetId> = saved.chain(imported).map(AssetEntry::file).collect();
    files.extend(extracted.flat_map(|extracted| [extracted.entry.id, extracted.host]));
    files.sort_unstable();
    files.dedup();
    files
}

fn reload_rewritten_files(
    mut saved: EventReader<SceneSaved>,
    mut imported: EventReader<ImportFinished>,
    mut extracted: EventReader<AssetExtracted>,
    registry: Res<AssetRegistry>,
    hosted: Query<&HostedApp>,
    parked: Query<&ParkedEditApp>,
    pending: Query<&PendingApp>,
) {
    let files = rewritten_files(saved.read(), imported.read(), extracted.read());
    if files.is_empty() {
        return;
    }
    let apps = hosted
        .iter()
        .chain(parked.iter().map(|parked| &parked.0))
        .chain(pending.iter().filter_map(PendingApp::held));
    for app in apps.filter(|app| app.role() != HostRole::Game) {
        app.reload_files(&files, &registry);
    }
}
