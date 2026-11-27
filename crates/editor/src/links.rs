//! Scene links in the editor: how links and linked nodes show in the tree and the inspector, the tool that removes a link's change, and reloading the files a save or an import rewrote.
use concerto_app::{
    App, Plugin,
    schedule_groups::{LateUpdate, Update},
};
use concerto_ecs::{
    Component, Entity, IntoSystemConfig, Query, Ref, Res,
    command::CommandQueue,
    component::{name::Name, removed::RemovedComponents},
    entity::{
        EntityHashMap,
        hierarchy::{ChildOf, Children},
    },
    events::event_reader::EventReader,
    query::{
        change_detection::DetectChanges,
        filter::{Added, Changed, Or, With, Without},
    },
    reflect::AppTypeRegistry,
    resource::ResMut,
    signal::{
        On,
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
    link::{ChangeOutcome, LinkedNode, SceneLink, SceneLinkState, linked_node_name, shown_name},
    link_path::LinkPath,
    scene::{LinkChange, Scene},
};
use concerto_ui::{
    elements::{Surface, prelude::*},
    interaction::UIClick,
    node::{UINode, UIRect},
    theme::UITheme,
};
use concerto_window::input::MouseButton;
use schemars::JsonSchema;
use serde::Deserialize;
use taffy::AlignItems;

use crate::{
    entity_ids::{SceneEntities, resolve},
    fonts::{button_icon, glyph, icon_button},
    hierarchy::{RowLook, RowLooks, reconcile_content},
    hosted::{
        EditTarget, EditWorld, EditWorldSignals, HostRole, Hosted, HostedApp, HostedEventsAppExt,
        ParkedEditApp, PendingApp,
    },
    import::ImportFinished,
    inspector::{InspectedComponent, InspectorHeader, ReadOnlyCard, refused},
    scene::SceneSaved,
    tools::{
        entity_ops::scene_member_by,
        links::{added_component, linked_in_open_scene_by},
        scene::SceneRoot,
    },
};

pub use crate::tools::links::{LinkChangeRemoved, RemoveLinkChange};

/// Shows links and their linked nodes in the tree and the inspector, and reloads what a save or
/// an import rewrote.
///
/// In every `App` but a playing game's, the assets of the rewritten files are reloaded, so the
/// links to their scenes spawn again.
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
    mut headers: Query<(Entity, Ref<InspectorHeader>, &mut UINode, Option<&Children>)>,
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
    for (header, inspected, mut layout, children) in headers.iter_mut() {
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
) -> Vec<AssetId> {
    let saved = saved.flat_map(|saved| std::iter::once(&saved.asset).chain(&saved.written));
    let imported = imported
        .filter_map(|finished| finished.result.as_ref().ok())
        .flat_map(|source| &source.assets);
    let mut files: Vec<AssetId> = saved.chain(imported).map(AssetEntry::file).collect();
    files.sort_unstable();
    files.dedup();
    files
}

fn reload_rewritten_files(
    mut saved: EventReader<SceneSaved>,
    mut imported: EventReader<ImportFinished>,
    hosted: Query<&HostedApp>,
    parked: Query<&ParkedEditApp>,
    pending: Query<&PendingApp>,
) {
    let files = rewritten_files(saved.read(), imported.read());
    if files.is_empty() {
        return;
    }
    let apps = hosted
        .iter()
        .chain(parked.iter().map(|parked| &parked.0))
        .chain(pending.iter().filter_map(PendingApp::held));
    for app in apps.filter(|app| app.role() != HostRole::Game) {
        app.reload_files(&files);
    }
}
