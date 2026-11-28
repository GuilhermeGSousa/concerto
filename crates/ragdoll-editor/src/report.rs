use concerto_animation::{graph::AnimationGraph, player::AnimationPlayer};
use concerto_ecs::{Entity, Query, Res};
use concerto_editor::hosted::EditWorld;
use concerto_foundation::assets::asset_store::AssetStore;
use concerto_mcp::{McpTool, Needs, ToolCalls, ToolError};
use concerto_ragdoll::{BodySense, CharacterRagdoll, RagdollBody, RagdollMode};
use schemars::JsonSchema;
use serde::Deserialize;

/// Reports the body of a character of the game that is playing, whose animation graph drives its ragdoll: for each limb the physics amount its graph gives it and how much of it the behaviours cover, the states of the graph and what its behaviour nodes say, then what the body senses. Read-only; behaviours are nodes of the character's animation graph, which the animation graph tools edit.
#[derive(Deserialize, JsonSchema)]
pub struct BodyReport {
    /// The character, by id as scene_tree shows it while the game plays, such as 42v3; it carries a `CharacterRagdoll`.
    pub entity: String,
}

pub(crate) fn report_tool() -> McpTool {
    McpTool::read_only("body_report").needs(Needs::Playing)
}

pub(crate) type Graphs = Option<Res<'static, AssetStore<AnimationGraph>>>;

type Characters = (
    Query<
        'static,
        'static,
        (
            Option<&'static CharacterRagdoll>,
            Option<&'static RagdollBody>,
            Option<&'static AnimationPlayer>,
            Option<&'static BodySense>,
        ),
    >,
    Graphs,
);

pub(crate) fn states_text(states: &[String]) -> String {
    if states.is_empty() {
        "none".to_owned()
    } else {
        states.join(", ")
    }
}

pub(crate) fn not_graph_driven(entity: &str, mode: RagdollMode) -> ToolError {
    ToolError::new(
        "not_graph_driven",
        format!(
            "The ragdoll of {entity} is in mode {mode:?}, so no animation graph drives it; set its CharacterRagdoll's mode to Graph and give it a graph with behaviour nodes."
        ),
    )
}

pub(crate) fn no_player(entity: &str) -> ToolError {
    ToolError::new(
        "no_player",
        format!(
            "{entity} has no AnimationPlayer, so no graph drives its body; give it an AnimationHandleComponent with a graph."
        ),
    )
}

pub(crate) fn body_report(mut calls: ToolCalls<BodyReport>, scene: EditWorld<Characters>) {
    let (characters, graphs) = &*scene;
    for (arguments, reply) in calls.read() {
        let Some(entity) = Entity::parse_id(&arguments.entity) else {
            reply.err(ToolError::new(
                "unknown_entity",
                format!(
                    "`{}` is not an entity id; ids look like 42v3, as scene_tree shows them while the game plays.",
                    arguments.entity
                ),
            ));
            continue;
        };
        let Some((character, body, player, sense)) = characters.get_entity(entity) else {
            reply.err(ToolError::new(
                "unknown_entity",
                format!(
                    "No entity {} in the playing game; call scene_tree for current ids.",
                    arguments.entity
                ),
            ));
            continue;
        };
        let Some(character) = character else {
            reply.err(ToolError::new(
                "no_ragdoll",
                format!(
                    "{} has no CharacterRagdoll; report on the character entity that carries it, which scene_tree shows.",
                    arguments.entity
                ),
            ));
            continue;
        };
        let Some(body) = body else {
            reply.err(ToolError::new(
                "ragdoll_not_built",
                format!(
                    "The ragdoll of {} is not built yet; wait for the game to finish loading and call again.",
                    arguments.entity
                ),
            ));
            continue;
        };
        if character.mode != RagdollMode::Graph {
            reply.err(not_graph_driven(&arguments.entity, character.mode));
            continue;
        }
        let Some(player) = player else {
            reply.err(no_player(&arguments.entity));
            continue;
        };
        reply.ok(body.report(player, graphs.as_deref(), sense));
    }
}
