use concerto_animation::{
    attribute::PoseAttribute, graph::AnimationGraph, node::DriveNodeInputs, player::AnimationPlayer,
};
use concerto_ecs::{CommandQueue, Entity, Query, Res, ResMut, Without, system::input::InputSet};
use concerto_foundation::{assets::asset_store::AssetStore, time::Time};
use concerto_mesh::{Skeleton, SkeletonComponent};
use concerto_physics::{physics_state::PhysicsState, ragdoll::RagdollId};
use glam::{Quat, Vec3};

use crate::{
    asset::{PartStrength, Ragdoll, StrengthProfile},
    attributes::{MotorCommand, PhysicsAmount},
    behaviour::BodyDrive,
    character::{
        Asked, CharacterRagdoll, Driven, Held, PoseLost, RagdollBody, RagdollMode, drive_part,
    },
    dying::Settled,
    sensing::BodySense,
    shot::Wounds,
    view::BodyView,
};

pub(crate) fn old_path_owns(character: &CharacterRagdoll, claimed: bool) -> bool {
    character.mode != RagdollMode::FollowsAnimation || claimed
}

pub(crate) fn physics_amount(player: Option<&AnimationPlayer>, bone: usize) -> f32 {
    player
        .filter(|player| !player.is_paused())
        .map_or(0.0, |player| {
            player
                .attributes()
                .get::<PhysicsAmount>(bone)
                .0
                .clamp(0.0, 1.0)
        })
}

pub(crate) fn drawn_amount(
    character: &CharacterRagdoll,
    player: Option<&AnimationPlayer>,
    body: &RagdollBody,
    part: usize,
) -> f32 {
    let graph = body
        .resolved()
        .bones()
        .get(part)
        .map_or(0.0, |bone| physics_amount(player, *bone));
    let old = body
        .bone_name(part)
        .map_or(0.0, |bone| character.weight_of(&bone));
    graph.max(old)
}

fn command_of(player: Option<&AnimationPlayer>, bone: usize) -> MotorCommand {
    player
        .filter(|player| !player.is_paused())
        .map_or_else(MotorCommand::neutral, |player| {
            player.drive_attributes().get::<MotorCommand>(bone)
        })
}

fn follow_of(profile: Option<&StrengthProfile>, body: &RagdollBody, part: usize) -> Option<Held> {
    body.resolved().settings().parts[part].parent?;
    let strength = profile?.strength_of(&body.bone_name(part)?);
    matches!(strength, PartStrength::Powered { .. }).then(|| Held::of(strength))
}

struct PartDrive {
    held: Held,
    target: Quat,
    force: Vec3,
    torque: Vec3,
}

fn part_drive(
    amount: f32,
    command: MotorCommand,
    follow: Option<Held>,
    animated_target: Quat,
) -> PartDrive {
    let cover = command.cover.clamp(0.0, 1.0);
    if amount <= 0.0 && cover <= 0.0 {
        return PartDrive {
            held: Held::KINEMATIC,
            target: animated_target,
            force: Vec3::ZERO,
            torque: Vec3::ZERO,
        };
    }
    let commanded = Held {
        kinematic: false,
        stiffness: command.stiffness,
        damping: command.damping,
        torque_limit: command.torque_limit,
    };
    match follow {
        Some(follow) if cover < 1.0 => {
            let rest = 1.0 - cover;
            PartDrive {
                held: Held {
                    kinematic: false,
                    stiffness: commanded.stiffness * cover + follow.stiffness * rest,
                    damping: commanded.damping * cover + follow.damping * rest,
                    torque_limit: commanded.torque_limit * cover + follow.torque_limit * rest,
                },
                target: animated_target.slerp(command.target, cover).normalize(),
                force: command.force * cover,
                torque: command.torque * cover,
            }
        }
        _ => PartDrive {
            held: commanded,
            target: command.target,
            force: command.force,
            torque: command.torque,
        },
    }
}

fn view_of(
    entity: Entity,
    id: RagdollId,
    body: &RagdollBody,
    character: &CharacterRagdoll,
    ragdolls: &AssetStore<Ragdoll>,
    sense: Option<&BodySense>,
    wounds: Option<&Wounds>,
) -> BodyView {
    BodyView::of(
        entity,
        id,
        body,
        ragdolls.get(&character.ragdoll),
        sense,
        wounds,
    )
}

/// Each frame, before the animation players update, hands every physical character's player a `BodyView` of its body.
pub fn view_bodies(
    mut characters: Query<(
        Entity,
        &RagdollId,
        &RagdollBody,
        &CharacterRagdoll,
        &mut AnimationPlayer,
        Option<&BodySense>,
        Option<&Wounds>,
    )>,
    ragdolls: Res<AssetStore<Ragdoll>>,
) {
    for (entity, id, body, character, mut player, sense, wounds) in characters.iter_mut() {
        player.set_extra(view_of(
            entity, *id, body, character, &ragdolls, sense, wounds,
        ));
    }
}

/// Each fixed step, before the ragdolls are driven, hands every physical character's player a `BodyView` of its body and walks its graph's drive pass for the step.
pub fn drive_graphs(
    mut characters: Query<
        (
            Entity,
            &RagdollId,
            &RagdollBody,
            &CharacterRagdoll,
            &mut AnimationPlayer,
            &SkeletonComponent,
            Option<&BodySense>,
            Option<&Wounds>,
        ),
        Without<Settled>,
    >,
    ragdolls: Res<AssetStore<Ragdoll>>,
    graphs: Option<Res<AssetStore<AnimationGraph>>>,
    skeletons: Res<AssetStore<Skeleton>>,
    inputs: InputSet<DriveNodeInputs>,
) {
    let Some(graphs) = graphs else {
        return;
    };
    let dt = Time::fixed_delta_time().as_secs_f32();
    for (entity, id, body, character, mut player, skeleton, sense, wounds) in characters.iter_mut()
    {
        player.set_extra(view_of(
            entity, *id, body, character, &ragdolls, sense, wounds,
        ));
        player.drive(&graphs, skeletons.get(skeleton.skeleton()), &inputs, dt);
    }
}

/// Each fixed step, before physics steps, drives every ragdoll from its graph's physics attributes: a part whose bone has no `PhysicsAmount` and no `MotorCommand` cover is kinematic and moved to its bone; any other part is dynamic, its motor aimed by the behaviours' command and, for the cover they leave, by the animation at the strength of the character's `follow` profile (never for the root part, nor a part that profile leaves kinematic or limp), leading the target by its speed, with the command's force and torque. A character the old modes or behaviour claims hold is left to `drive_character_ragdolls`.
pub fn drive_from_graph(
    mut characters: Query<
        (
            Entity,
            &CharacterRagdoll,
            &RagdollId,
            &mut RagdollBody,
            Option<&AnimationPlayer>,
            Option<&BodyDrive>,
        ),
        Without<Settled>,
    >,
    ragdolls: Res<AssetStore<Ragdoll>>,
    mut state: ResMut<PhysicsState>,
    mut cmd: CommandQueue,
) {
    let dt = Time::fixed_delta_time().as_secs_f32();
    let mut simulated = Vec::new();
    for (entity, character, id, mut body, player, drive) in characters.iter_mut() {
        let claimed = drive.is_some_and(|drive| !drive.running().is_empty());
        if old_path_owns(character, claimed) {
            continue;
        }
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
            claimed: vec![true; drives.len()],
        };
        body.note_changes(&wanted);
        let animated = body.animated_pose().to_vec();
        let body = &mut *body;
        let driven = Driven {
            id: *id,
            animated: &animated,
            wanted: &wanted,
            asked: &asked,
            step: 1.0,
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
