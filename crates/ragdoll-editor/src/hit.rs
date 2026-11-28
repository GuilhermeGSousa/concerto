use std::time::Duration;

use concerto_animation::player::AnimationPlayer;
use concerto_ecs::{
    Component, Entity, Query, Res, command::CommandQueue, query::change_detection::DetectChanges,
};
use concerto_editor::hosted::{EditWorld, EditWorldSignals};
use concerto_foundation::time::Time;
use concerto_mcp::{McpTool, Needs, PendingCall, ToolCalls, ToolError};
use concerto_physics::{physics_state::PhysicsState, ragdoll::RagdollId};
use concerto_ragdoll::{BodyHit, CharacterRagdoll, RagdollBody, active_states};
use glam::Vec3;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::report::{Graphs, no_player, states_text};

const DEFAULT_IMPULSE: Vec3 = Vec3::new(0.0, 0.0, -30.0);
const PATIENCE: f32 = 8.0;
const REACTION: f32 = 1.0;
const WALL_LIMIT: Duration = Duration::from_secs(30);

/// Hits a character of the game that is playing the way a bullet does, by sending `BodyHit` for the part on `bone`, and answers once the body is back on its animation: the bone hit, how far it got from its animated place at most, in metres, and the seconds it took. It answers at once that the graph did not react when no bone of the body is drawn from physics within a second, which is what a character with no reaction in its animation graph does, and with the graph's states after 8 seconds when the body never comes back, as a body that is down or dead does.
#[derive(Deserialize, JsonSchema)]
pub struct HitCharacter {
    /// The character, by id as scene_tree shows it while the game plays, such as 42v3; it carries a `CharacterRagdoll` and an `AnimationPlayer`.
    pub entity: String,
    /// The bone of the part that is hit, by name as ragdoll_parts shows it.
    pub bone: String,
    /// The impulse in N·s along x, y and z; [0, 0, -30] without it.
    pub impulse: Option<[f32; 3]>,
    /// Where the hit lands, in world space; the part's current position without it.
    pub point: Option<[f32; 3]>,
    /// True marks the hit as a gunshot, which the graph's shot behaviour answers to; a blow without it.
    pub shot: Option<bool>,
}

pub(crate) fn hit_tool() -> McpTool {
    McpTool::new("hit_character").needs(Needs::Playing)
}

#[derive(Component)]
pub(crate) struct HitWait {
    scene_entity: Entity,
    part: usize,
    bone: String,
    seconds: f32,
    started: bool,
    reacted: bool,
    peak: f32,
}

type Characters = Query<
    'static,
    'static,
    (
        Option<&'static CharacterRagdoll>,
        Option<&'static RagdollId>,
        Option<&'static RagdollBody>,
        Option<&'static AnimationPlayer>,
    ),
>;

type Waited = (
    Query<'static, 'static, (&'static RagdollBody, &'static AnimationPlayer)>,
    Graphs,
    Option<Res<'static, Time>>,
);

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
        let Some((character, id, body, player)) = characters.get_entity(entity) else {
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
                    "{} has no CharacterRagdoll; hit the character entity that carries it, which scene_tree shows.",
                    arguments.entity
                ),
            ));
            continue;
        }
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
        if player.is_none() {
            reply.err(no_player(&arguments.entity));
            continue;
        }
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
        let hit = BodyHit::new(entity, part, point, impulse);
        if arguments.shot == Some(true) {
            signals.trigger(hit.shot());
        } else {
            signals.trigger(hit);
        }
        let mut pending =
            PendingCall::new(reply, "the body to return to its animation").timeout(WALL_LIMIT);
        pending.at_deadline = Some(format!(
            "Hit {} of {} with impulse {} N·s at {}; the game did not step for {} s of real time, so nothing was measured. Is it paused? Call status, then play.",
            arguments.bone,
            arguments.entity,
            vec3_text(impulse),
            vec3_text(point),
            WALL_LIMIT.as_secs()
        ));
        cmd.spawn((
            pending,
            HitWait {
                scene_entity: entity,
                part,
                bone: arguments.bone,
                seconds: 0.0,
                started: false,
                reacted: false,
                peak: 0.0,
            },
        ));
    }
}

impl HitWait {
    fn watch(&mut self, body: &RagdollBody, animated: bool, stepped: f32) {
        if let (Some(simulated), Some(wanted)) = (
            body.simulated_pose().get(self.part),
            body.animated_pose().get(self.part),
        ) {
            self.peak = self
                .peak
                .max(simulated.translation.distance(wanted.translation));
        }
        if !animated {
            self.reacted = true;
        }
        if self.started {
            self.seconds += stepped;
        }
        self.started = true;
    }
}

pub(crate) fn answer_hits(
    mut waiting: Query<(Entity, &PendingCall, &mut HitWait)>,
    scene: EditWorld<Waited>,
    mut cmd: CommandQueue,
) {
    let (characters, graphs, time) = &*scene;
    let stepped = time
        .as_ref()
        .filter(|time| time.is_changed())
        .map_or(0.0, |time| time.delta().as_secs_f32());
    for (entity, pending, mut wait) in waiting.iter_mut() {
        let id = wait.scene_entity.to_id_string();
        let Some((body, player)) = characters.get_entity(wait.scene_entity) else {
            pending.reply.err(ToolError::new(
                "unknown_entity",
                "The character left the game before its body returned.",
            ));
            cmd.despawn(entity);
            continue;
        };
        let animated = body.is_animated(player);
        wait.watch(body, animated, stepped);
        let elapsed = wait.seconds;
        if wait.reacted && animated {
            pending.reply.ok(format!(
                "Hit {} of {id}; the part got {:.3} m from its animated place at most and the body was back on its animation after {:.2} s.",
                wait.bone,
                wait.peak,
                elapsed
            ));
            cmd.despawn(entity);
        } else if !wait.reacted && elapsed >= REACTION {
            pending.reply.ok(format!(
                "Hit {} of {id}, but the graph did not react: no bone was drawn from physics within {} s. Add a flinch or shot entry to a slot node of its graph that listens for the Hit message, which graph_inspect and body_report show.",
                wait.bone,
                REACTION
            ));
            cmd.despawn(entity);
        } else if elapsed >= PATIENCE {
            let states = graphs
                .as_ref()
                .map_or_else(Vec::new, |graphs| active_states(player, graphs));
            pending.reply.ok(format!(
                "Hit {} of {id}, but the body did not return to its animation within {PATIENCE} s; the graph is in {}; call body_report for each limb.",
                wait.bone,
                states_text(&states)
            ));
            cmd.despawn(entity);
        }
    }
}
