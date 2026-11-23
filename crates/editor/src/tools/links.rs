//! Scene links in the edit `App`: which edits a linked node refuses, and the changes edits record on their link.
use std::any::TypeId;

use anyhow::{Context, bail};
use concerto_app::{App, edit_time::AtEditTime, schedule_groups::Update};
use concerto_ecs::{
    Entity, Query, World,
    command::CommandQueue,
    component::name::Name,
    entity::hierarchy::ChildOf,
    events::{Event, event_reader::EventReader},
    reflect::{AppTypeRegistry, ReflectSceneComponent, SceneCapture},
    signal::{On, Signal},
};
use concerto_reflect::TypeRegistry;
use concerto_scene::{
    link::{
        LinkParent, LinkedNode, OwnedPath, SceneLink, SceneLinkState, owning_link_by,
        scene_address, shown_name,
    },
    scene::{LinkChange, SerializedComponent},
};
use log::warn;

use crate::tools::{
    edits::{ComponentEdit, ComponentEdited},
    entity_ops::{EntityEdit, EntityEdited, in_open_scene, scene_member, scene_member_by},
    scene::{SceneRoot, SceneStatus, report_edit, write},
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

pub(crate) fn added_component(
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
                    Ok(made),
                ) if is_linked(*parent) => made.first().map(|entity| LinkEdit::Created {
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
