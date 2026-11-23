//! Scene entity ids as MCP tools name them, resolved against the open scene.
use concerto_ecs::{Entity, Query, entity::hierarchy::ChildOf, system::input::SystemInputData};
use concerto_mcp::ToolError;
use concerto_scene::link::LinkedNode;

use crate::tools::{entity_ops::scene_member_by, scene::SceneRoot};

pub(crate) type SceneEntities = Query<
    'static,
    'static,
    (
        Entity,
        Option<&'static ChildOf>,
        Option<&'static SceneRoot>,
        Option<&'static LinkedNode>,
    ),
>;

fn outside_open_scene(entity: Entity, entities: &SystemInputData<SceneEntities>) -> bool {
    entities.iter().any(|(_, _, root, _)| root.is_some())
        && scene_member_by(entity, |entity| {
            entities
                .get_entity(entity)
                .map(|(_, parent, root, _)| (root.is_some(), parent.map(ChildOf::parent)))
        })
        .is_err()
}

pub(crate) fn resolve(
    id: &str,
    entities: &SystemInputData<SceneEntities>,
) -> Result<Entity, ToolError> {
    let Some(entity) = Entity::parse_id(id) else {
        return Err(ToolError::new(
            "unknown_entity",
            format!("`{id}` is not an entity id; ids look like 42v3, as scene_tree shows them."),
        ));
    };
    if entities.get_entity(entity).is_some() {
        if outside_open_scene(entity, entities) {
            Err(ToolError::new(
                "unknown_entity",
                format!("{id} is not part of the open scene; call scene_tree for the scene's ids."),
            ))
        } else {
            Ok(entity)
        }
    } else if entities
        .iter()
        .any(|(live, ..)| live.index() == entity.index())
    {
        Err(ToolError::new(
            "stale_entity",
            format!(
                "{id} is gone and its slot holds another entity; call scene_tree for current ids."
            ),
        ))
    } else {
        Err(ToolError::new(
            "unknown_entity",
            format!("No entity {id}; call scene_tree for current ids."),
        ))
    }
}
