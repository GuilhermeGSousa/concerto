use concerto_animation::{attribute::PoseAttribute, player::AnimationPlayer};
use concerto_ecs::{CommandQueue, Entity, Query, Res, ResMut, Without};
use concerto_foundation::{assets::asset_store::AssetStore, time::Time};
use concerto_physics::{physics_state::PhysicsState, ragdoll::RagdollId};
use glam::{Quat, Vec3};

use crate::{
    asset::{PartStrength, Ragdoll, StrengthProfile},
    attributes::{MotorCommand, PhysicsAmount},
    character::{Asked, CharacterRagdoll, Driven, Held, PoseLost, RagdollBody, drive_part},
    dying::Settled,
};

pub(crate) fn physics_amount(player: Option<&AnimationPlayer>, bone: usize) -> f32 {
    player.map_or(0.0, |player| {
        player
            .attributes()
            .get::<PhysicsAmount>(bone)
            .0
            .clamp(0.0, 1.0)
    })
}

pub(crate) fn drawn_amount(
    player: Option<&AnimationPlayer>,
    body: &RagdollBody,
    part: usize,
) -> f32 {
    body.resolved()
        .bones()
        .get(part)
        .map_or(0.0, |bone| physics_amount(player, *bone))
}

fn command_of(player: Option<&AnimationPlayer>, bone: usize) -> MotorCommand {
    player
        .filter(|player| !player.is_paused())
        .map_or_else(MotorCommand::neutral, |player| {
            player.drive_attributes().get::<MotorCommand>(bone)
        })
}

fn follow_of(profile: Option<&StrengthProfile>, body: &RagdollBody, part: usize) -> Option<Held> {
    let strength = profile?.strength_of(&body.bone_name(part)?);
    matches!(strength, PartStrength::Powered { .. }).then(|| Held::of(strength))
}

pub(crate) struct PartDrive {
    pub(crate) held: Held,
    pub(crate) target: Quat,
    pub(crate) force: Vec3,
    pub(crate) torque: Vec3,
}

pub(crate) fn part_drive(
    root: bool,
    amount: f32,
    command: MotorCommand,
    follow: Option<Held>,
    animated_target: Quat,
) -> PartDrive {
    let cover = command.cover.clamp(0.0, 1.0);
    let kinematic = cover <= 0.0 && (root || amount <= 0.0);
    if kinematic {
        return PartDrive {
            held: Held::KINEMATIC,
            target: animated_target,
            force: Vec3::ZERO,
            torque: Vec3::ZERO,
        };
    }
    let force = command.force * cover;
    let torque = command.torque * cover;
    match follow.filter(|_| !root && cover < 1.0) {
        Some(follow) => {
            let rest = 1.0 - cover;
            PartDrive {
                held: Held {
                    kinematic: false,
                    stiffness: command.stiffness * cover + follow.stiffness * rest,
                    damping: command.damping * cover + follow.damping * rest,
                    torque_limit: command.torque_limit * cover + follow.torque_limit * rest,
                },
                target: animated_target.slerp(command.target, cover).normalize(),
                force,
                torque,
            }
        }
        None => PartDrive {
            held: Held {
                kinematic: false,
                stiffness: command.stiffness,
                damping: command.damping,
                torque_limit: command.torque_limit,
            },
            target: command.target,
            force,
            torque,
        },
    }
}

/// Each fixed step, before physics steps, drives every ragdoll from its graph's physics attributes: a part whose bone has no `MotorCommand` cover is kinematic and moved to its bone while it has no `PhysicsAmount` either, and the root part whatever its amount; any other part is dynamic, its motor aimed by the behaviours' command and, for the cover they leave, by the animation at the strength of the character's `follow` profile (never for the root part, nor a part that profile leaves kinematic or limp), leading the target by its speed, with the command's force and torque scaled by its cover. A paused player is not driven, so its parts hold no command but keep their drawn amount.
pub fn drive_from_graph(
    mut characters: Query<
        (
            Entity,
            &CharacterRagdoll,
            &RagdollId,
            &mut RagdollBody,
            Option<&AnimationPlayer>,
        ),
        Without<Settled>,
    >,
    ragdolls: Res<AssetStore<Ragdoll>>,
    mut state: ResMut<PhysicsState>,
    mut cmd: CommandQueue,
) {
    let dt = Time::fixed_delta_time().as_secs_f32();
    let mut simulated = Vec::new();
    for (entity, character, id, mut body, player) in characters.iter_mut() {
        let profile = ragdolls
            .get(&character.ragdoll)
            .and_then(|ragdoll| ragdoll.profile(&character.follow));
        let targets = body.animated_targets();
        let drives: Vec<PartDrive> = body
            .resolved()
            .bones()
            .iter()
            .enumerate()
            .map(|(part, bone)| {
                part_drive(
                    body.resolved().settings().parts[part].parent.is_none(),
                    physics_amount(player, *bone),
                    command_of(player, *bone),
                    follow_of(profile, &body, part),
                    targets[part],
                )
            })
            .collect();
        let wanted: Vec<Held> = drives.iter().map(|drive| drive.held).collect();
        let asked = Asked {
            targets: drives.iter().map(|drive| drive.target).collect(),
            forces: drives.iter().map(|drive| drive.force).collect(),
            torques: drives.iter().map(|drive| drive.torque).collect(),
        };
        let animated = body.animated_pose().to_vec();
        let body = &mut *body;
        let driven = Driven {
            id: *id,
            animated: &animated,
            wanted: &wanted,
            asked: &asked,
            dt,
        };
        for part in 0..drives.len() {
            drive_part(&driven, part, body, &mut state);
        }
        if body.track_lost(character, *id, &state, &mut simulated, dt) {
            cmd.entity(entity).trigger(PoseLost);
        }
    }
}
