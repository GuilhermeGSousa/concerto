use std::{
    any::{Any, TypeId},
    sync::Arc,
};

use concerto_app::{App, schedule_groups::LateUpdate};
use concerto_ecs::{
    Component, Entity, Query, Res, ResMut, command::CommandQueue, component::name::Name,
    events::event_reader::EventReader, query::filter::With, reflect::ReflectComponent,
};
use concerto_foundation::assets::{asset_server::AssetServer, handle::LiveLeaves};
use concerto_mcp::{AppMcpExt, McpTool, Needs, PendingCall, Reply, ToolCalls, ToolError};
use concerto_reflect::{
    PropertyPath, Reflect, ReflectDefault, ReflectJson, TypeRegistry, apply_value_with,
    json_to_value_with, to_value_with, with_property,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use super::{
    act::label,
    observe::{ListedPath, SceneEntities, component_json, listed_paths, names_component, resolve},
};
use crate::{
    hosted::{EditTarget, EditWorld, EditWorldSignals, Hosted},
    inspector::{
        ErasedEditor, InspectionSource, InspectorRegistry, PropertyCommit, PropertyCommits,
        current_editor,
    },
    tools::{
        edits::{
            ComponentEdit, ComponentEdited, EditComponent, PropertyCommitted, PropertyRefusal,
            SetValue,
        },
        entity_ops::{EntityEdit, EntityEdited},
        scene::SceneRoot,
        selection::Selection,
    },
};

/// Adds an empty entity named Entity as the last child of a parent, or at the scene's top level when no parent is given, and answers its id.
#[derive(Deserialize, JsonSchema)]
pub struct CreateEntity {
    /// The parent, such as 42v3, as scene_tree shows it.
    pub parent: Option<String>,
}

/// Deletes an entity and its descendants, and answers what is selected afterwards.
#[derive(Deserialize, JsonSchema)]
pub struct DeleteEntity {
    /// The entity, such as 42v3, as scene_tree shows it.
    pub entity: String,
}

/// Copies an entity and its descendants beside it, and answers the copy's id.
#[derive(Deserialize, JsonSchema)]
pub struct DuplicateEntity {
    /// The entity, such as 42v3, as scene_tree shows it.
    pub entity: String,
}

/// Renames an entity and answers the name it took, trimmed.
#[derive(Deserialize, JsonSchema)]
pub struct RenameEntity {
    /// The entity, such as 42v3, as scene_tree shows it.
    pub entity: String,
    /// The new name; it cannot be empty.
    pub name: String,
}

/// Writes a JSON value into one property of a component, through the inspector's adapter when one edits it, and answers the component as inspect shows it.
#[derive(Deserialize, JsonSchema)]
pub struct SetProperty {
    /// The entity, such as 42v3, as scene_tree shows it.
    pub entity: String,
    /// The component, by short name such as Light, or by full type path when the short name is ambiguous.
    pub component: String,
    /// The property, one of the paths inspect lists for the component, such as intensity or translation; "" for a component inspect lists as one value; a path inspect marks enum takes the whole enum, such as "Point" or {"Spot": {"cone_angle": 40}}.
    pub path: String,
    /// The value, as JSON in the shape inspect reports it; a struct may leave fields out to keep them.
    pub value: Value,
}

/// Adds a component at its default value and answers it as inspect shows it.
#[derive(Deserialize, JsonSchema)]
pub struct AddComponent {
    /// The entity, such as 42v3, as scene_tree shows it.
    pub entity: String,
    /// The component, by short name such as Light, or by full type path when the short name is ambiguous.
    pub component: String,
}

/// Removes a component from an entity.
#[derive(Deserialize, JsonSchema)]
pub struct RemoveComponent {
    /// The entity, such as 42v3, as scene_tree shows it.
    pub entity: String,
    /// The component, by short name such as Light, or by full type path when the short name is ambiguous.
    pub component: String,
}

#[derive(Component)]
struct EditingEntity {
    document: Entity,
    edit: EntityEdit,
    label: String,
}

#[derive(Component)]
struct SettingProperty {
    document: Entity,
    entity: Entity,
    component: TypeId,
    path: PropertyPath,
}

#[derive(Component)]
struct EditingComponent {
    document: Entity,
    edit: ComponentEdit,
    label: String,
}

pub(super) fn build(app: &mut App) {
    app.add_tool::<CreateEntity, _>(
        McpTool::new("create_entity").needs(Needs::Scene),
        create_entity,
    )
    .add_tool::<DeleteEntity, _>(
        McpTool::new("delete_entity").needs(Needs::Scene),
        delete_entity,
    )
    .add_tool::<DuplicateEntity, _>(
        McpTool::new("duplicate_entity").needs(Needs::Scene),
        duplicate_entity,
    )
    .add_tool::<RenameEntity, _>(
        McpTool::new("rename_entity").needs(Needs::Scene),
        rename_entity,
    )
    .add_tool::<SetProperty, _>(
        McpTool::new("set_property").needs(Needs::Scene),
        set_property,
    )
    .add_tool::<AddComponent, _>(
        McpTool::new("add_component").needs(Needs::Scene),
        add_component,
    )
    .add_tool::<RemoveComponent, _>(
        McpTool::new("remove_component").needs(Needs::Scene),
        remove_component,
    )
    .add_system(
        LateUpdate,
        (
            answer_entity_edits,
            answer_property_commits,
            answer_component_edits,
        ),
    );
}

fn refused(reason: &str) -> ToolError {
    ToolError::new("refused", reason)
}

fn root_refused(edit: &str) -> ToolError {
    refused(&format!("{edit} failed: the scene root cannot be changed"))
}

fn edit_entity(
    reply: &Reply,
    document: Entity,
    edit: EntityEdit,
    label: String,
    scene: &mut EditWorldSignals,
    cmd: &mut CommandQueue,
) {
    scene.trigger(edit.clone());
    cmd.spawn((
        PendingCall::new(reply, "the entity edit"),
        EditingEntity {
            document,
            edit,
            label,
        },
    ));
}

fn create_entity(
    mut calls: ToolCalls<CreateEntity>,
    entities: EditWorld<SceneEntities>,
    roots: EditWorld<Query<Entity, With<SceneRoot>>>,
    names: EditWorld<Query<&Name>>,
    targets: Query<Entity, With<EditTarget>>,
    mut scene: EditWorldSignals,
    mut cmd: CommandQueue,
) {
    let Some(document) = targets.iter().next() else {
        return;
    };
    for (arguments, reply) in calls.read() {
        let parent = match arguments.parent.as_deref().map(|id| resolve(id, &entities)) {
            Some(Ok(parent)) => Some(parent),
            Some(Err(error)) => {
                reply.err(error);
                continue;
            }
            None => roots.iter().next(),
        };
        let place = parent.map_or_else(
            || "at the top level".to_owned(),
            |parent| format!("in {}", label(parent, &names)),
        );
        edit_entity(
            reply,
            document,
            EntityEdit::Create { parent },
            place,
            &mut scene,
            &mut cmd,
        );
    }
}

fn delete_entity(
    mut calls: ToolCalls<DeleteEntity>,
    entities: EditWorld<SceneEntities>,
    names: EditWorld<Query<&Name>>,
    targets: Query<Entity, With<EditTarget>>,
    mut scene: EditWorldSignals,
    mut cmd: CommandQueue,
) {
    let Some(document) = targets.iter().next() else {
        return;
    };
    for (arguments, reply) in calls.read() {
        match resolve(&arguments.entity, &entities) {
            Ok(entity) => edit_entity(
                reply,
                document,
                EntityEdit::Delete(entity),
                label(entity, &names),
                &mut scene,
                &mut cmd,
            ),
            Err(error) => reply.err(error),
        }
    }
}

fn duplicate_entity(
    mut calls: ToolCalls<DuplicateEntity>,
    entities: EditWorld<SceneEntities>,
    names: EditWorld<Query<&Name>>,
    targets: Query<Entity, With<EditTarget>>,
    mut scene: EditWorldSignals,
    mut cmd: CommandQueue,
) {
    let Some(document) = targets.iter().next() else {
        return;
    };
    for (arguments, reply) in calls.read() {
        match resolve(&arguments.entity, &entities) {
            Ok(entity) => edit_entity(
                reply,
                document,
                EntityEdit::Duplicate(entity),
                label(entity, &names),
                &mut scene,
                &mut cmd,
            ),
            Err(error) => reply.err(error),
        }
    }
}

fn rename_entity(
    mut calls: ToolCalls<RenameEntity>,
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
            Ok(entity) => edit_entity(
                reply,
                document,
                EntityEdit::Rename {
                    entity,
                    name: arguments.name,
                },
                entity.to_id_string(),
                &mut scene,
                &mut cmd,
            ),
            Err(error) => reply.err(error),
        }
    }
}

fn entity_answer(
    editing: &EditingEntity,
    made: Option<Entity>,
    names: &Query<&Name>,
    selection: Option<&Selection>,
) -> Result<String, ToolError> {
    let place = &editing.label;
    match (&editing.edit, made) {
        (EntityEdit::Create { .. }, Some(made)) => {
            Ok(format!("Created {} {place}.", label(made, names)))
        }
        (EntityEdit::Duplicate(_), Some(made)) => {
            Ok(format!("Duplicated {place} as {}.", label(made, names)))
        }
        (EntityEdit::Create { .. } | EntityEdit::Duplicate(_), None) => Err(ToolError::new(
            "internal",
            "The edit was applied but made no entity.",
        )),
        (EntityEdit::Delete(_), _) => Ok(match selection.and_then(Selection::entity) {
            Some(selected) => format!(
                "Deleted {place}; the selection is {}.",
                label(selected, names)
            ),
            None => format!("Deleted {place}; nothing is selected."),
        }),
        (EntityEdit::Rename { name, .. }, _) => Ok(format!("Renamed {place} to {}.", name.trim())),
    }
}

fn answer_entity_edits(
    mut edited: EventReader<Hosted<EntityEdited>>,
    waiting: Query<(Entity, &PendingCall, &EditingEntity)>,
    names: EditWorld<Query<&Name>>,
    selection: EditWorld<Option<Res<Selection>>>,
    mut cmd: CommandQueue,
) {
    for Hosted { document, event } in edited.read() {
        for (call, pending, editing) in waiting.iter() {
            if editing.document != *document || editing.edit != event.edit {
                continue;
            }
            let answer = match &event.result {
                Ok(made) => entity_answer(editing, *made, &names, selection.as_deref()),
                Err(reason) => Err(refused(reason)),
            };
            match answer {
                Ok(text) => pending.reply.ok(text),
                Err(error) => pending.reply.err(error),
            }
            cmd.despawn(call);
        }
    }
}

type Named = (TypeId, &'static str, &'static str);

fn pick(
    name: &str,
    candidates: &[Named],
    unknown: impl FnOnce() -> String,
) -> Result<Named, ToolError> {
    let matching: Vec<&Named> = candidates
        .iter()
        .filter(|(_, short, full)| names_component(short, full, name))
        .collect();
    match matching.as_slice() {
        [one] => Ok(**one),
        [] => Err(ToolError::new("unknown_component", unknown())),
        many => Err(ToolError::new(
            "ambiguous_component",
            format!(
                "`{name}` names {}; give the full path.",
                many.iter()
                    .map(|(_, _, full)| *full)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )),
    }
}

fn carried_component(
    source: &InspectionSource,
    entity: Entity,
    name: &str,
) -> Result<Named, ToolError> {
    let mut carried: Vec<Named> = source
        .visible_components(entity)
        .into_iter()
        .filter_map(|(id, short)| Some((id, short, source.types().get(id)?.type_path())))
        .collect();
    carried.sort_unstable_by_key(|(_, short, full)| (*short, *full));
    pick(name, &carried, || {
        let shorts: Vec<&str> = carried.iter().map(|(_, short, _)| *short).collect();
        format!(
            "{} carries no {name}; it carries {}.",
            entity.to_id_string(),
            if shorts.is_empty() {
                "none".to_owned()
            } else {
                shorts.join(", ")
            }
        )
    })
}

fn registered_component(types: &TypeRegistry, name: &str) -> Result<Named, ToolError> {
    let mut registered: Vec<Named> = types
        .iter()
        .filter(|registration| registration.data::<ReflectComponent>().is_some())
        .map(|registration| {
            (
                registration.type_id(),
                registration.short_name(),
                registration.type_path(),
            )
        })
        .collect();
    registered.sort_unstable_by_key(|(_, short, full)| (*short, *full));
    pick(name, &registered, || {
        let wanted = name.to_lowercase();
        let close: Vec<&str> = registered
            .iter()
            .map(|(_, short, _)| *short)
            .filter(|short| {
                let short = short.to_lowercase();
                short.contains(&wanted) || wanted.contains(&short)
            })
            .collect();
        if close.is_empty() {
            format!("No component named {name} is registered in the scene's App.")
        } else {
            format!(
                "No component named {name} is registered in the scene's App; close matches: {}.",
                close.join(", ")
            )
        }
    })
}

fn shown_path(path: &PropertyPath) -> String {
    path.segments().join(".")
}

fn property_name(component: &str, path: &PropertyPath) -> String {
    std::iter::once(component)
        .chain(path.segments().iter().copied())
        .collect::<Vec<_>>()
        .join(".")
}

fn unknown_path(component: &str, path: &str, listed: &[ListedPath]) -> ToolError {
    if listed.is_empty() {
        return ToolError::new(
            "unknown_path",
            format!(
                "{component} has no reflection data the editor can write, so it has no paths; inspect shows it unavailable."
            ),
        );
    }
    let paths: Vec<String> = listed
        .iter()
        .map(|listed| match shown_path(&listed.path) {
            shown if shown.is_empty() => "\"\"".to_owned(),
            shown => shown,
        })
        .collect();
    ToolError::new(
        "unknown_path",
        format!(
            "{component} has no path `{path}`; its paths are {}.",
            paths.join(", ")
        ),
    )
}

fn decoded(
    field: &dyn Reflect,
    value: &Value,
    types: &TypeRegistry,
    hook: &mut LiveLeaves,
) -> Result<Box<dyn Reflect>, ToolError> {
    let type_id = (field as &dyn Any).type_id();
    let invalid = |reason: String| ToolError::new("invalid_value", reason);
    let mut built = if let Some(codec) = types.get_type_data::<ReflectJson>(type_id) {
        (codec.from_json)(value).map_err(invalid)?
    } else {
        let Some(default) = types.get_type_data::<ReflectDefault>(type_id) else {
            return Err(ToolError::new(
                "not_editable",
                format!(
                    "{} has no JSON form and no registered Default, so a value cannot be built from JSON.",
                    field.type_name()
                ),
            ));
        };
        let mut built = default.default();
        if let Ok(current) = to_value_with(field, hook) {
            apply_value_with(built.as_mut(), &current, hook);
        }
        built
    };
    let stored = json_to_value_with(value, type_id, types, hook).map_err(invalid)?;
    let warnings = apply_value_with(built.as_mut(), &stored, hook);
    if warnings.is_empty() {
        Ok(built)
    } else {
        Err(invalid(
            warnings
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; "),
        ))
    }
}

fn value_edit(
    field: &dyn Reflect,
    value: &Value,
    types: &TypeRegistry,
    hook: &mut LiveLeaves,
    editor: &dyn ErasedEditor,
    property: &str,
) -> Result<Arc<dyn Any + Send + Sync>, ToolError> {
    let built = decoded(field, value, types, hook).map_err(|error| {
        ToolError::new(
            error.code,
            format!("{value} does not fit {property}: {}", error.message),
        )
    })?;
    match editor.edit_to(built.as_ref()) {
        Ok(Some(edit)) => Ok(edit),
        Ok(None) => Err(ToolError::new(
            "not_editable",
            format!(
                "{property}'s inspector adapter offers no value edit, so set_property cannot write it."
            ),
        )),
        Err(error) => Err(ToolError::new(
            "invalid_value",
            format!("{value} does not fit {property}: {error}"),
        )),
    }
}

enum Write {
    Adapter(PropertyCommit),
    Value(SetValue),
}

fn property_write(
    source: &InspectionSource,
    registry: &Res<InspectorRegistry>,
    server: Option<&AssetServer>,
    entity: Entity,
    root: bool,
    arguments: SetProperty,
) -> Result<Write, ToolError> {
    let (component, short, _) = carried_component(source, entity, &arguments.component)?;
    let changed = registry.changed_tick();
    let rows = source
        .collect_component(registry, changed, entity, component)
        .unwrap_or_default();
    let listed = source
        .read(entity, component)
        .map(|value| listed_paths(value, &rows))
        .unwrap_or_default();
    let Some(found) = listed
        .iter()
        .find(|listed| shown_path(&listed.path) == arguments.path)
    else {
        return Err(unknown_path(short, &arguments.path, &listed));
    };
    let name = property_name(short, &found.path);
    if root {
        return Err(root_refused(&format!("Set {name}")));
    }
    let Some(property) = found
        .row
        .filter(|property| !found.names_enum && property.has_editor())
    else {
        return Ok(Write::Value(SetValue {
            entity,
            component,
            path: found.path.clone(),
            value: arguments.value,
        }));
    };
    let row = property.row(entity, component);
    let editor = current_editor(registry, Some(source.types()), changed, &row)
        .map_err(|error| refused(&format!("Set {name} failed: {error}")))?;
    let mut edit = Err(unknown_path(short, &arguments.path, &[]));
    let mut hook = LiveLeaves::from_parts(source.types(), server);
    if let Some(value) = source.read(entity, component) {
        let _ = with_property(value, &row.path, &mut |field| {
            edit = value_edit(
                field,
                &arguments.value,
                source.types(),
                &mut hook,
                editor.adapter.as_ref(),
                &name,
            );
        });
    }
    Ok(Write::Adapter(PropertyCommit { row, edit: edit? }))
}

fn set_property(
    mut calls: ToolCalls<SetProperty>,
    source: EditWorld<InspectionSource>,
    entities: EditWorld<SceneEntities>,
    roots: EditWorld<Query<Entity, With<SceneRoot>>>,
    server: EditWorld<Option<Res<AssetServer>>>,
    registry: Res<InspectorRegistry>,
    targets: Query<Entity, With<EditTarget>>,
    mut commits: ResMut<PropertyCommits>,
    mut scene: EditWorldSignals,
    mut cmd: CommandQueue,
) {
    let Some(document) = targets.iter().next() else {
        return;
    };
    for (arguments, reply) in calls.read() {
        let write = resolve(&arguments.entity, &entities).and_then(|entity| {
            let root = roots.get_entity(entity).is_some();
            property_write(
                &source,
                &registry,
                server.as_deref(),
                entity,
                root,
                arguments,
            )
        });
        let row = match write {
            Ok(Write::Adapter(commit)) => {
                let row = commit.row.clone();
                commits.0.push(commit);
                (row.entity, row.component, row.path)
            }
            Ok(Write::Value(set)) => {
                let row = (set.entity, set.component, set.path.clone());
                scene.trigger(set);
                row
            }
            Err(error) => {
                reply.err(error);
                continue;
            }
        };
        let (entity, component, path) = row;
        cmd.spawn((
            PendingCall::new(reply, "the property to be set"),
            SettingProperty {
                document,
                entity,
                component,
                path,
            },
        ));
    }
}

fn component_answer(
    source: &InspectionSource,
    registry: &InspectorRegistry,
    entity: Entity,
    component: TypeId,
) -> String {
    let (short, full) = source
        .types()
        .get(component)
        .map_or(("", ""), |registration| {
            (registration.short_name(), registration.type_path())
        });
    component_json(source, registry, entity, short, full, component).to_string()
}

fn answer_property_commits(
    mut committed: EventReader<Hosted<PropertyCommitted>>,
    waiting: Query<(Entity, &PendingCall, &SettingProperty)>,
    source: EditWorld<InspectionSource>,
    registry: Res<InspectorRegistry>,
    mut cmd: CommandQueue,
) {
    for Hosted { document, event } in committed.read() {
        for (call, pending, setting) in waiting.iter() {
            let row = &event.row;
            if setting.document != *document
                || setting.entity != row.entity
                || setting.component != row.component
                || setting.path != row.path
            {
                continue;
            }
            match &event.result {
                Ok(()) => pending.reply.ok(component_answer(
                    &source,
                    &registry,
                    setting.entity,
                    setting.component,
                )),
                Err(PropertyRefusal::Undecodable(reason)) => {
                    pending.reply.err(ToolError::new("invalid_value", reason))
                }
                Err(PropertyRefusal::Refused(reason)) => pending.reply.err(refused(reason)),
            }
            cmd.despawn(call);
        }
    }
}

fn add_component(
    mut calls: ToolCalls<AddComponent>,
    source: EditWorld<InspectionSource>,
    entities: EditWorld<SceneEntities>,
    roots: EditWorld<Query<Entity, With<SceneRoot>>>,
    targets: Query<Entity, With<EditTarget>>,
    mut scene: EditWorldSignals,
    mut cmd: CommandQueue,
) {
    let Some(document) = targets.iter().next() else {
        return;
    };
    for (arguments, reply) in calls.read() {
        let edit = resolve(&arguments.entity, &entities).and_then(|entity| {
            let (component, short, _) = registered_component(source.types(), &arguments.component)?;
            if roots.get_entity(entity).is_some() {
                return Err(root_refused(&format!("Add {short}")));
            }
            Ok(ComponentEdit::Add { entity, component })
        });
        match edit {
            Ok(edit) => {
                scene.trigger(EditComponent(edit));
                cmd.spawn((
                    PendingCall::new(reply, "the component to be added"),
                    EditingComponent {
                        document,
                        edit,
                        label: String::new(),
                    },
                ));
            }
            Err(error) => reply.err(error),
        }
    }
}

fn remove_component(
    mut calls: ToolCalls<RemoveComponent>,
    source: EditWorld<InspectionSource>,
    entities: EditWorld<SceneEntities>,
    roots: EditWorld<Query<Entity, With<SceneRoot>>>,
    names: EditWorld<Query<&Name>>,
    targets: Query<Entity, With<EditTarget>>,
    mut scene: EditWorldSignals,
    mut cmd: CommandQueue,
) {
    let Some(document) = targets.iter().next() else {
        return;
    };
    for (arguments, reply) in calls.read() {
        let found = resolve(&arguments.entity, &entities).and_then(|entity| {
            let (component, short, _) = carried_component(&source, entity, &arguments.component)?;
            if roots.get_entity(entity).is_some() {
                return Err(root_refused(&format!("Remove {short}")));
            }
            Ok((entity, component, short))
        });
        match found {
            Ok((entity, component, short)) => {
                let edit = ComponentEdit::Remove { entity, component };
                scene.trigger(EditComponent(edit));
                cmd.spawn((
                    PendingCall::new(reply, "the component to be removed"),
                    EditingComponent {
                        document,
                        edit,
                        label: format!("Removed {short} from {}.", label(entity, &names)),
                    },
                ));
            }
            Err(error) => reply.err(error),
        }
    }
}

fn answer_component_edits(
    mut edited: EventReader<Hosted<ComponentEdited>>,
    waiting: Query<(Entity, &PendingCall, &EditingComponent)>,
    source: EditWorld<InspectionSource>,
    registry: Res<InspectorRegistry>,
    mut cmd: CommandQueue,
) {
    for Hosted { document, event } in edited.read() {
        for (call, pending, editing) in waiting.iter() {
            if editing.document != *document || editing.edit != event.edit {
                continue;
            }
            match (&event.result, editing.edit) {
                (Ok(()), ComponentEdit::Add { entity, component }) => pending
                    .reply
                    .ok(component_answer(&source, &registry, entity, component)),
                (Ok(()), ComponentEdit::Remove { .. }) => pending.reply.ok(editing.label.clone()),
                (Err(reason), _) => pending.reply.err(refused(reason)),
            }
            cmd.despawn(call);
        }
    }
}
