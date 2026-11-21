use std::sync::Arc;

use concerto_app::App;
use concerto_ecs::{
    IntoSystemConfig, Query, Res, World,
    command::CommandQueue,
    signal::{On, Signal},
    system::input::{StaticSystemInput, SystemInputData},
};
use concerto_mcp::{
    McpRead, McpServe, PendingCall, Reply, ToolCalls, ToolError, answer_unanswered,
    receive_read_only_calls,
};
use serde_json::{Map, Value};

use super::{
    edit::{ListPush, ListRemove, SetProperty, list_commit_at, list_length},
    observe::{Inspect, insert_unavailable, insert_value},
};
use crate::{
    asset_editor::ActiveEditor,
    inspector::{
        InspectedValues, ListEdit, PropertyTarget, apply_value_commit, gone, listed_paths, reading,
        refusal_error, set_value,
    },
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
    let found: Vec<_> = front_values(active, values)
        .iter()
        .filter(|target| target.name() == name)
        .collect();
    match found[..] {
        [target] => cmd.trigger(ValueCall {
            reply: PendingCall::new(reply, "the value").reply,
            target: target.clone(),
            request,
        }),
        [] => reply.err(ToolError::new(
            "unknown_target",
            format!(
                "The front document has no value named {name}; its values are {}.",
                value_names(active, values)
            ),
        )),
        _ => reply.err(ToolError::new(
            "ambiguous_target",
            format!(
                "The front document has {} values named {name}, so a name can't reach one.",
                found.len()
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
        ValueRequest::Set { path, value } => set_value(world, target, path, value)
            .map(|()| value_answer(world, target))
            .map_err(ToolError::from),
        ValueRequest::List { path, edit } => edit_list(world, target, path, *edit),
    };
    match answer {
        Ok(text) => reply.ok(text),
        Err(error) => reply.err(error),
    }
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
    .unwrap_or_else(|| Err(gone(&attempt).into()))?;
    apply_value_commit(world, &commit).map_err(|refusal| refusal_error(&refusal))?;
    let path = &commit.row.address.path;
    let length = reading(world, target.as_ref(), |value, _| list_length(value, path)).unwrap_or(0);
    Ok(format!("{done}; its length is {length}."))
}
