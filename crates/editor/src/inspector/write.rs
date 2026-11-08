use std::{
    any::{Any, TypeId},
    sync::Arc,
};

use concerto_ecs::{
    World,
    signal::{On, Signal},
};
use concerto_foundation::assets::{asset_server::AssetServer, handle::LiveLeaves};
use concerto_mcp::ToolError;
use concerto_reflect::{
    Reflect, ReflectedValue, apply_value_with, with_property, with_property_mut,
};
use serde_json::Value;

use super::{
    InspectorRegistry, PropertyAddress, PropertyCommit, PropertyTarget, apply_value_commit,
    report_value_edit,
};
use crate::{
    asset_editor::EditorDocument,
    mcp::{EditSource, adapter_commit, find_path, listed_paths, refusal_error, refused},
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
    if let Err(error) = set_value(world, target, path, json)
        && let Some(document) = target.document()
        && let Some(held) = world.get_component_for_entity_mut::<EditorDocument>(document)
    {
        held.status = error.message;
    }
}

enum ValueWrite {
    Adapter(PropertyCommit),
    Json {
        address: PropertyAddress,
        stored: Result<ReflectedValue, PropertyRefusal>,
        hook: LiveLeaves,
    },
}

/// The error an edit of a value that is gone answers.
pub(crate) fn gone(attempt: &str) -> ToolError {
    refused(&format!("{attempt} failed: {}", EditError::MissingTarget))
}

fn value_write(
    target: &Arc<dyn PropertyTarget>,
    path: &str,
    json: &Value,
    value: &dyn Reflect,
    source: &EditSource,
) -> Result<ValueWrite, ToolError> {
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
    let found = address.target.edit(world, &mut |value| {
        let _ = with_property_mut(value, &address.path, &mut |value| {
            warnings = apply_value_with(value, stored, hook);
        });
    });
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
) -> Result<(), ToolError> {
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
