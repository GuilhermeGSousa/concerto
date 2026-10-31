use std::{
    any::{Any, TypeId},
    sync::Arc,
};

use concerto_app::App;
use concerto_ecs::{
    IntoSystemConfig, Query, Res, World,
    command::CommandQueue,
    signal::{On, Signal},
    system::input::{StaticSystemInput, SystemInputData},
};
use concerto_foundation::assets::{asset_server::AssetServer, handle::LiveLeaves};
use concerto_mcp::{
    McpRead, McpServe, PendingCall, Reply, ToolCalls, ToolError, answer_unanswered,
    receive_read_only_calls,
};
use concerto_reflect::{
    Reflect, ReflectedValue, apply_value_with, with_property, with_property_mut,
};
use serde_json::{Map, Value};

use super::{
    edit::{
        EditSource, ListPush, ListRemove, SetProperty, adapter_commit, find_path, list_commit_at,
        list_length, refusal_error, refused,
    },
    observe::{Inspect, insert_unavailable, insert_value, listed_paths},
};
use crate::{
    asset_editor::ActiveEditor,
    inspector::{
        InspectedValues, InspectorRegistry, ListEdit, PropertyAddress, PropertyCommit,
        PropertyTarget, apply_value_commit, report_value_edit,
    },
    tools::edits::{EditError, PropertyRefusal, applied, decode_property, dotted_name},
};

enum ValueRequest {
    Inspect,
    Set { path: String, value: Value },
    List { path: String, edit: ListEdit },
}

struct ValueCall {
    reply: Reply,
    target: Arc<dyn PropertyTarget>,
    request: ValueRequest,
}

impl Signal for ValueCall {}

enum ValueWrite {
    Adapter(PropertyCommit),
    Json {
        address: PropertyAddress,
        stored: Result<ReflectedValue, PropertyRefusal>,
        hook: LiveLeaves,
    },
}

type FrontValues = (
    Res<'static, ActiveEditor>,
    Query<'static, 'static, &'static InspectedValues>,
);

pub(super) fn build(app: &mut App) {
    app.add_system(McpServe, inspect_value.before(answer_unanswered))
        .add_system(
            McpRead,
            inspect_value
                .after(receive_read_only_calls)
                .before(answer_unanswered),
        )
        .add_system(McpServe, set_value_property.before(answer_unanswered))
        .add_system(McpServe, push_to_value.before(answer_unanswered))
        .add_system(McpServe, remove_from_value.before(answer_unanswered))
        .add_listener(answer_value_call);
}

fn front_values<'a>(
    active: &ActiveEditor,
    values: &'a Query<&InspectedValues>,
) -> &'a [Arc<dyn PropertyTarget>] {
    active
        .0
        .and_then(|document| values.get_entity(document))
        .map_or(&[], |values| values.0.as_slice())
}

/// The names of the front document's values, as `status` and `unknown_target` list them.
pub(super) fn value_names(active: &ActiveEditor, values: &Query<&InspectedValues>) -> String {
    let names: Vec<String> = front_values(active, values)
        .iter()
        .map(|target| target.name())
        .collect();
    if names.is_empty() {
        "none".to_owned()
    } else {
        names.join(", ")
    }
}

fn invalid(message: &str) -> ToolError {
    ToolError::new("invalid_arguments", message)
}

fn addressed<'a>(
    target: &'a Option<String>,
    entity: &Option<String>,
    component: &Option<String>,
) -> Result<Option<&'a str>, ToolError> {
    match (target, entity, component) {
        (Some(name), None, None) => Ok(Some(name)),
        (None, Some(_), Some(_)) => Ok(None),
        (Some(_), ..) => Err(invalid(
            "target names a value by itself; leave out entity and component.",
        )),
        (None, None, _) => Err(invalid("Give target, or entity and component.")),
        (None, Some(_), None) => Err(invalid("entity needs component.")),
    }
}

fn inspected(arguments: &Inspect) -> Result<Option<&str>, ToolError> {
    match (&arguments.target, &arguments.entity, &arguments.components) {
        (Some(name), None, None) => Ok(Some(name)),
        (None, Some(_), _) => Ok(None),
        (Some(_), ..) => Err(invalid(
            "target names a value by itself; leave out entity and components.",
        )),
        (None, None, _) => Err(invalid("Give target or entity.")),
    }
}

fn start_value_call(
    reply: &Reply,
    name: &str,
    request: ValueRequest,
    front: &SystemInputData<FrontValues>,
    cmd: &mut CommandQueue,
) {
    let (active, values) = front;
    let found = front_values(active, values)
        .iter()
        .find(|target| target.name() == name);
    match found {
        Some(target) => cmd.trigger(ValueCall {
            reply: PendingCall::new(reply, "the value").reply,
            target: target.clone(),
            request,
        }),
        None => reply.err(ToolError::new(
            "unknown_target",
            format!(
                "The front document has no value named {name}; its values are {}.",
                value_names(active, values)
            ),
        )),
    }
}

fn inspect_value(
    mut calls: ToolCalls<Inspect>,
    front: StaticSystemInput<FrontValues>,
    mut cmd: CommandQueue,
) {
    for (arguments, reply) in calls.read() {
        match inspected(&arguments) {
            Ok(Some(name)) => {
                start_value_call(reply, name, ValueRequest::Inspect, &front, &mut cmd);
            }
            Ok(None) => {}
            Err(error) => reply.err(error),
        }
    }
}

fn set_value_property(
    mut calls: ToolCalls<SetProperty>,
    front: StaticSystemInput<FrontValues>,
    mut cmd: CommandQueue,
) {
    for (arguments, reply) in calls.read() {
        match addressed(&arguments.target, &arguments.entity, &arguments.component) {
            Ok(Some(name)) => {
                let request = ValueRequest::Set {
                    path: arguments.path,
                    value: arguments.value,
                };
                start_value_call(reply, name, request, &front, &mut cmd);
            }
            Ok(None) => {}
            Err(error) => reply.err(error),
        }
    }
}

fn push_to_value(
    mut calls: ToolCalls<ListPush>,
    front: StaticSystemInput<FrontValues>,
    mut cmd: CommandQueue,
) {
    for (arguments, reply) in calls.read() {
        match addressed(&arguments.target, &arguments.entity, &arguments.component) {
            Ok(Some(name)) => {
                let request = ValueRequest::List {
                    path: arguments.path,
                    edit: ListEdit::Push,
                };
                start_value_call(reply, name, request, &front, &mut cmd);
            }
            Ok(None) => {}
            Err(error) => reply.err(error),
        }
    }
}

fn remove_from_value(
    mut calls: ToolCalls<ListRemove>,
    front: StaticSystemInput<FrontValues>,
    mut cmd: CommandQueue,
) {
    for (arguments, reply) in calls.read() {
        match addressed(&arguments.target, &arguments.entity, &arguments.component) {
            Ok(Some(name)) => {
                let request = ValueRequest::List {
                    path: arguments.path,
                    edit: ListEdit::Remove(arguments.index),
                };
                start_value_call(reply, name, request, &front, &mut cmd);
            }
            Ok(None) => {}
            Err(error) => reply.err(error),
        }
    }
}

fn answer_value_call(on: On<ValueCall>, world: &mut World) {
    let ValueCall {
        reply,
        target,
        request,
    } = on.signal();
    let answer = match request {
        ValueRequest::Inspect => Ok(value_answer(world, target)),
        ValueRequest::Set { path, value } => set_value(world, target, path, value),
        ValueRequest::List { path, edit } => edit_list(world, target, path, *edit),
    };
    match answer {
        Ok(text) => reply.ok(text),
        Err(error) => reply.err(error),
    }
}

fn reading<R>(
    world: &World,
    target: &dyn PropertyTarget,
    mut read: impl FnMut(&dyn Reflect, &EditSource) -> R,
) -> Option<R> {
    let registry = world.get_resource::<InspectorRegistry>()?;
    let changed = world.resource_changed_tick::<InspectorRegistry>()?;
    let types = target.types(world).unwrap_or_default();
    let types = types.read();
    let source = EditSource {
        types: &types,
        registry,
        changed,
        server: world.get_resource::<AssetServer>(),
    };
    let mut result = None;
    target.read(world, &mut |value| result = Some(read(value, &source)));
    result
}

fn value_answer(world: &World, target: &Arc<dyn PropertyTarget>) -> String {
    let mut answer = Map::new();
    answer.insert("name".into(), target.name().into());
    let shown = reading(world, target.as_ref(), |value, source| {
        let mut shown = Map::new();
        shown.insert("type".into(), value.type_name().into());
        insert_value(&mut shown, value, source.types, source.registry);
        shown
    });
    match shown {
        Some(shown) => answer.extend(shown),
        None => insert_unavailable(&mut answer),
    }
    Value::Object(answer).to_string()
}

fn gone(attempt: &str) -> ToolError {
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

fn set_value(
    world: &mut World,
    target: &Arc<dyn PropertyTarget>,
    path: &str,
    json: &Value,
) -> Result<String, ToolError> {
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
    result.map_err(|refusal| refusal_error(&refusal))?;
    Ok(value_answer(world, target))
}

fn edit_list(
    world: &mut World,
    target: &Arc<dyn PropertyTarget>,
    path: &str,
    edit: ListEdit,
) -> Result<String, ToolError> {
    let name = target.name();
    let attempt = format!("Change {name}.{path}");
    let (commit, done) = reading(world, target.as_ref(), |value, source| {
        let rows = source.registry.collect(value, source.types);
        let listed = listed_paths(value, &rows);
        let target = target.clone();
        list_commit_at(&name, path, &listed, false, edit, target, source.changed)
    })
    .unwrap_or_else(|| Err(gone(&attempt)))?;
    apply_value_commit(world, &commit).map_err(|refusal| refusal_error(&refusal))?;
    let path = &commit.row.address.path;
    let length = reading(world, target.as_ref(), |value, _| list_length(value, path)).unwrap_or(0);
    Ok(format!("{done}; its length is {length}."))
}
