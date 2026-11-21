use concerto_ecs::{Entity, Query};
use concerto_editor::hosted::EditWorld;
use concerto_mcp::{McpTool, Needs, ToolCalls, ToolError};
use concerto_ragdoll::{
    BodyDrive, BodySense, CharacterRagdoll, PhysicalState, RagdollBody, describe_body,
};
use schemars::JsonSchema;
use serde::Deserialize;

/// Reports the body of a character of the game that is playing: its ragdoll's mode, profile and weight, then its state, the behaviours it runs with their priority and the part bones they claim, and what it senses. Read-only; behaviours are components on the character, added and removed with the component tools.
#[derive(Deserialize, JsonSchema)]
pub struct BodyReport {
    /// The character, by id as scene_tree shows it while the game plays, such as 42v3; it carries a `CharacterRagdoll`.
    pub entity: String,
}

pub(crate) fn report_tool() -> McpTool {
    McpTool::read_only("body_report").needs(Needs::Playing)
}

type Characters = Query<
    'static,
    'static,
    (
        Option<&'static CharacterRagdoll>,
        Option<&'static RagdollBody>,
        Option<&'static PhysicalState>,
        Option<&'static BodyDrive>,
        Option<&'static BodySense>,
    ),
>;

pub(crate) fn body_report(mut calls: ToolCalls<BodyReport>, scene: EditWorld<Characters>) {
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
        let Some((character, body, physical, drive, sense)) = scene.get_entity(entity) else {
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
        reply.ok(format!(
            "ragdoll: mode {:?}, profile {}, weight {:.2}\n{}",
            character.mode,
            character.profile,
            character.weight,
            describe_body(physical, drive, sense, body)
        ));
    }
}
