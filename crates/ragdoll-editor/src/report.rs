use concerto_animation::{graph::AnimationGraph, player::AnimationPlayer};
use concerto_ecs::{Entity, Query, Res};
use concerto_editor::hosted::EditWorld;
use concerto_foundation::assets::asset_store::AssetStore;
use concerto_mcp::{McpTool, Needs, ToolCalls, ToolError};
use concerto_ragdoll::{BodySense, CharacterRagdoll, RagdollBody, report_body};
use schemars::JsonSchema;
use serde::Deserialize;

/// Reports the body of a character of the game that is playing: for each limb the physics amount its animation graph gives it and how much of it the behaviours cover, the states of the graph and what its behaviour nodes say, then what the body senses. Read-only; behaviours are nodes of the character's animation graph, which the animation graph tools edit.
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
        if character.is_none() {
            reply.err(ToolError::new(
                "no_ragdoll",
                format!(
                    "{} has no CharacterRagdoll; report on the character entity that carries it, which scene_tree shows.",
                    arguments.entity
                ),
            ));
            continue;
        }
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
        let Some(player) = player else {
            reply.err(ToolError::new(
                "no_player",
                format!(
                    "{} has no AnimationPlayer, so no graph drives its body and it never reacts; give it an AnimationHandleComponent with a graph.",
                    arguments.entity
                ),
            ));
            continue;
        };
        let text = report_body(body, player, graphs.as_deref(), sense);
        reply.ok(text);
    }
}
