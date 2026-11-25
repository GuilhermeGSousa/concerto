use std::{
    any::{Any, TypeId},
    sync::Arc,
};

use concerto_ecs::{
    World,
    component::Tick,
    signal::{On, Signal},
};
use concerto_foundation::assets::{asset_server::AssetServer, handle::LiveLeaves};
use concerto_reflect::{
    Reflect, ReflectDefault, ReflectJson, ReflectedValue, TypeRegistry, apply_value_with,
    json_to_value_with, to_value_with, with_property, with_property_mut,
};
use serde_json::Value;

use super::{
    ErasedEditor, InspectorRegistry, Property, PropertyAddress, PropertyCommit, PropertyTarget,
    apply_value_commit, current_editor,
    paths::{find_path, listed_paths, unknown_path},
    report_value_edit,
};
use crate::{
    asset_editor::EditorDocument,
    tools::edits::{EditError, PropertyRefusal, applied, decode_property, dotted_name},
};

/// Writes `json` at `path` in `target`, as MCP `set_property` does, reporting the result as a `PropertyCommitted`.
pub struct WriteValue {
    pub target: Arc<dyn PropertyTarget>,
    pub path: String,
    pub json: Value,
}

impl Signal for WriteValue {}

pub(super) fn write_value_at(on: On<WriteValue>, world: &mut World) {
    let WriteValue { target, path, json } = on.signal();
    if let Err(refusal) = set_value(world, target, path, json)
        && let Some(document) = target.document()
        && let Some(held) = world.get_component_for_entity_mut::<EditorDocument>(document)
    {
        held.status = refusal.message;
    }
}

/// Why a write of a value by path was refused: a stable code, as MCP answers it, and a message that says what to do next.
pub(crate) struct WriteRefusal {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

impl WriteRefusal {
    pub(super) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

pub(crate) fn refused(reason: &str) -> WriteRefusal {
    WriteRefusal::new("refused", reason)
}

pub(crate) fn not_editable(name: &str) -> WriteRefusal {
    WriteRefusal::new("not_editable", format!("{name} has no editor."))
}

/// The refusal a refused property write answers.
pub(crate) fn refusal_error(refusal: &PropertyRefusal) -> WriteRefusal {
    match refusal {
        PropertyRefusal::Undecodable(reason) => WriteRefusal::new("invalid_value", reason),
        PropertyRefusal::NotEditable(reason) => WriteRefusal::new("not_editable", reason),
        PropertyRefusal::Refused(reason) => refused(reason),
    }
}

/// The refusal an edit of a value that is gone answers.
pub(crate) fn gone(attempt: &str) -> WriteRefusal {
    refused(&format!("{attempt} failed: {}", EditError::MissingTarget))
}

fn decoded(
    field: &dyn Reflect,
    value: &Value,
    types: &TypeRegistry,
    hook: &mut LiveLeaves,
) -> Result<Box<dyn Reflect>, WriteRefusal> {
    let type_id = (field as &dyn Any).type_id();
    let invalid = |reason: String| WriteRefusal::new("invalid_value", reason);
    let mut built = if let Some(codec) = types.get_type_data::<ReflectJson>(type_id) {
        (codec.from_json_with)(value, types).map_err(invalid)?
    } else {
        let Some(default) = types.get_type_data::<ReflectDefault>(type_id) else {
            return Err(WriteRefusal::new(
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
) -> Result<Arc<dyn Any + Send + Sync>, WriteRefusal> {
    let built = decoded(field, value, types, hook).map_err(|error| {
        WriteRefusal::new(
            error.code,
            format!("{value} does not fit {property}: {}", error.message),
        )
    })?;
    match editor.edit_to(built.as_ref()) {
        Ok(Some(edit)) => Ok(edit),
        Ok(None) => Err(WriteRefusal::new(
            "not_editable",
            format!(
                "{property}'s inspector adapter offers no value edit, so set_property cannot write it."
            ),
        )),
        Err(error) => Err(WriteRefusal::new(
            "invalid_value",
            format!("{value} does not fit {property}: {error}"),
        )),
    }
}

/// What building an edit of a property reads.
pub(crate) struct EditSource<'a> {
    pub(crate) types: &'a TypeRegistry,
    pub(crate) registry: &'a InspectorRegistry,
    pub(crate) changed: Tick,
    pub(crate) server: Option<&'a AssetServer>,
}

/// The commit that writes `json` to `property` of `value`, through the editor that claims it.
pub(crate) fn adapter_commit(
    property: &Property,
    target: Arc<dyn PropertyTarget>,
    owner: &str,
    value: &dyn Reflect,
    json: &Value,
    source: &EditSource,
) -> Result<PropertyCommit, WriteRefusal> {
    let name = dotted_name(owner, &property.path);
    let row = property
        .row(target, source.changed)
        .ok_or_else(|| not_editable(&name))?;
    let editor = current_editor(source.registry, Some(source.types), source.changed, &row)
        .map_err(|error| refused(&format!("Set {name} failed: {error}")))?;
    let mut edit = Err(unknown_path(owner, "", &[]));
    let mut hook = LiveLeaves::from_parts(source.types, source.server);
    let _ = with_property(value, &row.address.path, &mut |field| {
        edit = value_edit(
            field,
            json,
            source.types,
            &mut hook,
            editor.adapter.as_ref(),
            &name,
        );
    });
    Ok(PropertyCommit { row, edit: edit? })
}

enum ValueWrite {
    Adapter(PropertyCommit),
    Json {
        address: PropertyAddress,
        stored: Result<ReflectedValue, PropertyRefusal>,
        hook: LiveLeaves,
    },
}

fn value_write(
    target: &Arc<dyn PropertyTarget>,
    path: &str,
    json: &Value,
    value: &dyn Reflect,
    source: &EditSource,
) -> Result<ValueWrite, WriteRefusal> {
    let name = target.name();
    let rows = source.registry.collect(value, source.types);
    let listed = listed_paths(value, &rows);
    let found = find_path(&name, path, &listed)?;
    if let Some(property) = found.adapter() {
        let target = target.clone();
        let commit = adapter_commit(property, target, &name, value, json, source)?;
        return Ok(ValueWrite::Adapter(commit));
    }
    let mut field = TypeId::of::<()>();
    let _ = with_property(value, &found.path, &mut |value| {
        field = (value as &dyn Any).type_id();
    });
    let root = (value as &dyn Any).type_id();
    let mut hook = LiveLeaves::from_parts(source.types, source.server);
    let stored = if source.types.get(root).is_some() {
        decode_property(
            json,
            value,
            root,
            &found.path,
            field,
            source.types,
            &mut hook,
        )
    } else {
        Err(PropertyRefusal::Refused(
            "its type is not registered, so JSON cannot build it".to_owned(),
        ))
    };
    Ok(ValueWrite::Json {
        address: PropertyAddress {
            target: target.clone(),
            path: found.path.clone(),
            type_id: field,
        },
        stored,
        hook,
    })
}

fn write_json(
    world: &mut World,
    address: &PropertyAddress,
    stored: &ReflectedValue,
    hook: &mut LiveLeaves,
) -> Result<(), PropertyRefusal> {
    let mut warnings = Vec::new();
    let found = address
        .target
        .try_edit(world, &mut |value| {
            let _ = with_property_mut(value, &address.path, &mut |value| {
                warnings = apply_value_with(value, stored, hook);
            });
        })
        .map_err(PropertyRefusal::Refused)?;
    if found {
        applied(&warnings)
    } else {
        Err(PropertyRefusal::Refused(
            EditError::MissingTarget.to_string(),
        ))
    }
}

/// Writes `json` at the `path` MCP shows into `target`, reporting the edit to its document.
pub(crate) fn set_value(
    world: &mut World,
    target: &Arc<dyn PropertyTarget>,
    path: &str,
    json: &Value,
) -> Result<(), WriteRefusal> {
    let attempt = format!("Set {}.{path}", target.name());
    let write = reading(world, target.as_ref(), |value, source| {
        value_write(target, path, json, value, source)
    })
    .unwrap_or_else(|| Err(gone(&attempt)))?;
    let result = match write {
        ValueWrite::Adapter(commit) => apply_value_commit(world, &commit),
        ValueWrite::Json {
            address,
            stored,
            mut hook,
        } => {
            let result = stored
                .and_then(|stored| write_json(world, &address, &stored, &mut hook))
                .map_err(|refusal| {
                    let property = dotted_name(&target.name(), &address.path);
                    refusal.map(|reason| format!("Set {property} failed: {reason}"))
                });
            report_value_edit(world, &address, &result);
            result
        }
    };
    result.map_err(|refusal| refusal_error(&refusal))
}

/// Calls `read` with `target`'s value and what building an edit of it reads; `None` when the value is gone.
pub(crate) fn reading<R>(
    world: &World,
    target: &dyn PropertyTarget,
    mut read: impl FnMut(&dyn Reflect, &EditSource) -> R,
) -> Option<R> {
    let registry = world.get_resource::<InspectorRegistry>()?;
    let changed = world.resource_changed_tick::<InspectorRegistry>()?;
    let types = target.types(world).unwrap_or_default();
    let server = world.get_resource::<AssetServer>();
    let mut result = None;
    target.read(world, &mut |value| {
        let table = types.read();
        let source = EditSource {
            types: &table,
            registry,
            changed,
            server,
        };
        result = Some(read(value, &source));
    });
    result
}
