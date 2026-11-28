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

impl PhysicsAmount {
    fn driven(player: Option<&AnimationPlayer>, bone: usize) -> f32 {
        player.map_or(0.0, |player| {
            player
                .drive_attributes()
                .get::<PhysicsAmount>(bone)
                .0
                .clamp(0.0, 1.0)
        })
    }
}

impl MotorCommand {
    fn driven(player: Option<&AnimationPlayer>, bone: usize) -> Self {
        player.map_or_else(Self::neutral, |player| {
            player.drive_attributes().get::<Self>(bone)
        })
    }
}

impl RagdollBody {
    pub(crate) fn drawn_amount(&self, player: Option<&AnimationPlayer>, part: usize) -> f32 {
        self.resolved()
            .bones()
            .get(part)
            .map_or(0.0, |bone| PhysicsAmount::driven(player, *bone))
    }

    /// Whether the body is drawn wholly from the animation: the drive pass of `player`'s graph gives none of its bones a [`PhysicsAmount`].
    pub fn is_animated(&self, player: &AnimationPlayer) -> bool {
        (0..self.resolved().part_count()).all(|part| self.drawn_amount(Some(player), part) <= 0.0)
    }

    fn follow(&self, profile: Option<&StrengthProfile>, part: usize) -> Option<Held> {
        let strength = profile?.strength_of(&self.bone_name(part)?);
        matches!(strength, PartStrength::Powered { .. }).then(|| Held::of(strength))
    }
}

pub(crate) struct PartDrive {
    pub(crate) held: Held,
    pub(crate) target: Quat,
    pub(crate) force: Vec3,
    pub(crate) torque: Vec3,
}

impl PartDrive {
    pub(crate) fn new(
        root: bool,
        amount: f32,
        command: MotorCommand,
        follow: Option<Held>,
        animated_target: Quat,
    ) -> Self {
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
}

/// Drives every ragdoll from its graph's physics attributes, each fixed step after the drive pass and before physics steps.
///
/// It reads [`AnimationPlayer::drive_attributes`] only, so every part of one
/// step is decided by one walk of the graph.
///
/// - A part whose bone has neither a [`MotorCommand`] cover nor a
///   [`PhysicsAmount`] is kinematic and moved to its bone; so is the root part
///   while it has no cover, whatever its amount.
/// - Any other part is dynamic. Its motor is aimed by the command and, for the
///   share of the bone the command does not cover, by the animation at the
///   strength of the character's `follow` profile, leading the target by the
///   animation's speed. The root part gets no such share, nor does a part the
///   profile leaves kinematic or limp; its command is the graph's alone.
/// - The command's force and torque are scaled by its cover.
///
/// A character with no `AnimationPlayer`, or whose player is paused, asks for
/// nothing, so its body follows its bones.
///
/// Triggers [`PoseLost`] when the root part has been too far from its
/// animated place for too long.
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
                PartDrive::new(
                    body.resolved().settings().parts[part].parent.is_none(),
                    PhysicsAmount::driven(player, *bone),
                    MotorCommand::driven(player, *bone),
                    body.follow(profile, part),
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
