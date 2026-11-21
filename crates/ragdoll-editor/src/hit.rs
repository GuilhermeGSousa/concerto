use std::time::Duration;

use concerto_ecs::{Component, Entity, Query, Res, command::CommandQueue};
use concerto_editor::hosted::{EditWorld, EditWorldSignals};
use concerto_mcp::{McpTool, Needs, PendingCall, ToolCalls, ToolError};
use concerto_physics::{physics_state::PhysicsState, ragdoll::RagdollId};
use concerto_ragdoll::{
    BodyHit, CharacterRagdoll, Flinch, FlinchRecord, HitReaction, RagdollBody, RagdollMode,
};
use glam::Vec3;
use schemars::JsonSchema;
use serde::Deserialize;

const DEFAULT_IMPULSE: Vec3 = Vec3::new(0.0, 0.0, -30.0);
const PATIENCE: Duration = Duration::from_secs(8);

/// Hits a character of the game that is playing the way a bullet does, by sending `BodyHit` for the part on `bone`, and answers once its flinch is over: the bone hit, how far it got from its animated place at most, in metres, and the seconds until the body was back on its animation. A character with no `HitReaction` only takes the impulse, and the tool says so at once.
#[derive(Deserialize, JsonSchema)]
pub struct HitCharacter {
    /// The character, by id as scene_tree shows it while the game plays, such as 42v3; it carries a `CharacterRagdoll`.
    pub entity: String,
    /// The bone of the part that is hit, by name as ragdoll_parts shows it.
    pub bone: String,
    /// The impulse in N·s along x, y and z; [0, 0, -30] without it.
    pub impulse: Option<[f32; 3]>,
    /// Where the hit lands, in world space; the part's current position without it.
    pub point: Option<[f32; 3]>,
}

pub(crate) fn hit_tool() -> McpTool {
    McpTool::new("hit_character").needs(Needs::Playing)
}

#[derive(Component)]
pub(crate) struct HitWait {
    scene_entity: Entity,
    started: bool,
}

type Characters = Query<
    'static,
    'static,
    (
        Option<&'static CharacterRagdoll>,
        Option<&'static RagdollId>,
        Option<&'static RagdollBody>,
        Option<&'static HitReaction>,
        Option<&'static Flinch>,
    ),
>;

type Records = Query<
    'static,
    'static,
    (
        Option<&'static Flinch>,
        Option<&'static FlinchRecord>,
        Option<&'static RagdollBody>,
    ),
>;

enum Outcome {
    Waits,
    OnlyImpulse(&'static str),
}

fn outcome(
    character: &CharacterRagdoll,
    reaction: Option<&HitReaction>,
    flinching: bool,
) -> Outcome {
    match (reaction, flinching, character.mode) {
        (None, ..) => Outcome::OnlyImpulse("it has no HitReaction"),
        (Some(_), true, _) | (Some(_), false, RagdollMode::FollowsAnimation) => Outcome::Waits,
        (Some(_), false, _) => Outcome::OnlyImpulse(
            "its ragdoll is limp or powered by something else, so it does not flinch",
        ),
    }
}

fn unknown_bone(body: &RagdollBody, bone: &str) -> ToolError {
    let resolved = body.resolved();
    let bones: Vec<String> = (0..resolved.part_count())
        .filter_map(|part| resolved.bone_name(part))
        .map(|name| name.to_string())
        .collect();
    ToolError::new(
        "unknown_bone",
        format!(
            "The ragdoll has no part on bone `{bone}`; its parts are on: {}.",
            bones.join(", ")
        ),
    )
}

fn vec3_text(value: Vec3) -> String {
    format!("[{:.1}, {:.1}, {:.1}]", value.x, value.y, value.z)
}

fn flinch_text(bone: &str, entity: &str, record: &FlinchRecord, after: f32) -> String {
    format!(
        "Hit {bone} of {entity}; the flinch peaked {:.3} m from the animated place and the body was back on its animation after {after:.2} s.",
        record.peak_displacement
    )
}

pub(crate) fn hit_character(
    mut calls: ToolCalls<HitCharacter>,
    scene: EditWorld<(Characters, Option<Res<PhysicsState>>)>,
    mut signals: EditWorldSignals,
    mut cmd: CommandQueue,
) {
    let (characters, physics) = &*scene;
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
        let Some((character, id, body, reaction, flinch)) = characters.get_entity(entity) else {
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
                    "{} has no CharacterRagdoll; hit the character entity that carries it, which scene_tree shows.",
                    arguments.entity
                ),
            ));
            continue;
        };
        let (Some(id), Some(body)) = (id, body) else {
            reply.err(ToolError::new(
                "ragdoll_not_built",
                format!(
                    "The ragdoll of {} is not built yet; wait for the game to finish loading and call again.",
                    arguments.entity
                ),
            ));
            continue;
        };
        let Some(part) = body.resolved().part_of(&arguments.bone) else {
            reply.err(unknown_bone(body, &arguments.bone));
            continue;
        };
        let impulse = arguments.impulse.map_or(DEFAULT_IMPULSE, Vec3::from_array);
        let point = match arguments.point {
            Some(point) => Vec3::from_array(point),
            None => {
                let mut pose = Vec::new();
                if let Some(physics) = physics {
                    physics.ragdoll_pose(*id, &mut pose);
                }
                match pose.get(part) {
                    Some(pose) => pose.translation,
                    None => {
                        reply.err(ToolError::new(
                            "ragdoll_not_built",
                            format!(
                                "The body of {} has no pose yet; wait a moment and call again, or pass `point`.",
                                arguments.entity
                            ),
                        ));
                        continue;
                    }
                }
            }
        };
        signals.trigger(BodyHit {
            character: entity,
            part,
            point,
            impulse,
        });
        match outcome(character, reaction, flinch.is_some()) {
            Outcome::OnlyImpulse(why) => reply.ok(format!(
                "Hit {} of {} with impulse {} N·s at {}; {why}, so the body only took the impulse.",
                arguments.bone,
                arguments.entity,
                vec3_text(impulse),
                vec3_text(point)
            )),
            Outcome::Waits => {
                let mut pending = PendingCall::new(reply, "the flinch to end").timeout(PATIENCE);
                pending.at_deadline = Some(format!(
                    "Hit {} of {}, but no flinch started; the body may be kinematic at the hit part, or the game is not advancing.",
                    arguments.bone, arguments.entity
                ));
                cmd.spawn((
                    pending,
                    HitWait {
                        scene_entity: entity,
                        started: false,
                    },
                ));
            }
        }
    }
}

pub(crate) fn answer_hits(
    waiting: Query<(Entity, &PendingCall, &mut HitWait)>,
    scene: EditWorld<Records>,
    mut cmd: CommandQueue,
) {
    for (entity, pending, mut wait) in waiting.iter() {
        let Some((flinch, record, body)) = scene.get_entity(wait.scene_entity) else {
            pending.reply.err(ToolError::new(
                "unknown_entity",
                "The character left the game before its flinch ended.",
            ));
            cmd.despawn(entity);
            continue;
        };
        if flinch.is_some() {
            wait.started = true;
        }
        let Some(record) = record.filter(|_| wait.started && flinch.is_none()) else {
            continue;
        };
        let Some(after) = record.recovered_after else {
            continue;
        };
        let bone = body
            .and_then(|body| body.resolved().bone_name(record.part))
            .map_or_else(|| format!("part {}", record.part), |name| name.to_string());
        pending.reply.ok(flinch_text(
            &bone,
            &wait.scene_entity.to_id_string(),
            record,
            after,
        ));
        cmd.despawn(entity);
    }
}
