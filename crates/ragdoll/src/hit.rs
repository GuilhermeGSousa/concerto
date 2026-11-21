use concerto_ecs::{
    CommandQueue, Component, Entity, Query, ResMut,
    reflect::{ReflectComponent, ReflectSceneComponent},
    signal::{On, Signal},
};
use concerto_foundation::time::Time;
use concerto_name::Name;
use concerto_physics::{physics_state::PhysicsState, ragdoll::RagdollId};
use concerto_reflect::{Reflect, ReflectDefault};
use glam::Vec3;

use crate::{
    character::{CharacterRagdoll, RagdollBody, RagdollMode},
    generate::FLINCH,
};

/// A character's body was hit: on this part of its ragdoll, at this world point, with this impulse in N·s. It says nothing about damage or death; the game decides those and reacts to the same signal. It is triggered for every listener, with the character it is about, so the engine's flinch and a game's own listeners all hear it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyHit {
    pub character: Entity,
    pub part: usize,
    pub point: Vec3,
    pub impulse: Vec3,
}

impl Signal for BodyHit {}

/// Makes a character flinch when it is hit while its ragdoll follows its animation: the body is powered at `profile` and takes the impulse, the muscles at the hit give and recover, and the body settles back on the animation.
#[derive(Component, Reflect, Clone, Debug, PartialEq)]
#[reflect(Component, Default, SceneComponent)]
pub struct HitReaction {
    /// The strength profile the body is powered with while it flinches.
    pub profile: Name,
    /// What is left of the hit part's strength at the moment of the hit, from 0 to 1.
    pub dip: f32,
    /// What is left of the strength of the parts joined to the hit part.
    pub neighbour_dip: f32,
    /// The seconds over which the dipped strength comes back.
    pub recover: f32,
    /// The seconds after the last hit during which the simulated body is drawn in full.
    pub hold: f32,
    /// The seconds over which the drawn body then returns to the animation; when they are over the parts are kinematic again.
    pub settle: f32,
}

impl Default for HitReaction {
    fn default() -> Self {
        Self {
            profile: FLINCH.into(),
            dip: 0.25,
            neighbour_dip: 0.6,
            recover: 0.5,
            hold: 0.5,
            settle: 0.4,
        }
    }
}

/// A flinch in progress, beside the `HitReaction` that started it.
#[derive(Component, Clone, Debug)]
pub struct Flinch {
    pending: Vec<BodyHit>,
    scales: Vec<f32>,
    since_hit: f32,
    lasted: f32,
    part: usize,
    peak: f32,
}

impl Flinch {
    /// What is left of each part's strength, from 0 to 1, in part order.
    pub fn scales(&self) -> &[f32] {
        &self.scales
    }
}

/// What a character's last flinch did, for tuning without a gun.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct FlinchRecord {
    /// The part that was hit last.
    pub part: usize,
    /// The furthest the hit part got from its animated place, in metres.
    pub peak_displacement: f32,
    /// The seconds from the first hit until the body was back on its animation; `None` while it still flinches.
    pub recovered_after: Option<f32>,
}

fn dip(scales: &mut [f32], body: &RagdollBody, reaction: &HitReaction, part: usize) {
    let parts = &body.resolved().settings().parts;
    for (index, scale) in scales.iter_mut().enumerate() {
        let joined = parts[index].parent == Some(part) || parts[part].parent == Some(index);
        let left = if index == part {
            reaction.dip
        } else if joined {
            reaction.neighbour_dip
        } else {
            continue;
        };
        *scale = scale.min(left.clamp(0.0, 1.0));
    }
}

/// Starts or restarts a character's flinch when it is hit. A character without a `HitReaction`, or whose ragdoll is limp or powered by something else, only takes the impulse.
pub fn flinch_when_hit(
    on: On<BodyHit>,
    characters: Query<(
        &mut CharacterRagdoll,
        &RagdollId,
        &RagdollBody,
        Option<&HitReaction>,
        Option<&mut Flinch>,
    )>,
    mut state: ResMut<PhysicsState>,
    mut cmd: CommandQueue,
) {
    let hit = *on.signal();
    let entity = hit.character;
    let Some((mut character, id, body, reaction, flinch)) = characters.get_entity(entity) else {
        return;
    };
    if hit.part >= body.resolved().part_count() {
        return;
    }
    match (reaction, flinch) {
        (Some(reaction), Some(mut flinch)) => {
            flinch.pending.push(hit);
            flinch.since_hit = 0.0;
            flinch.part = hit.part;
            dip(&mut flinch.scales, body, reaction, hit.part);
            character.weight = 1.0;
        }
        (Some(reaction), None) if character.mode == RagdollMode::FollowsAnimation => {
            let mut scales = vec![1.0; body.resolved().part_count()];
            dip(&mut scales, body, reaction, hit.part);
            character.power(reaction.profile);
            cmd.insert(
                Flinch {
                    pending: vec![hit],
                    scales,
                    since_hit: 0.0,
                    lasted: 0.0,
                    part: hit.part,
                    peak: 0.0,
                },
                entity,
            );
        }
        _ => {
            if let Some(part) = state.ragdoll_bodies(*id).get(hit.part).copied() {
                state.add_impulse_at(part, hit.impulse, hit.point);
            }
        }
    }
}

/// Each fixed step, after the ragdolls are driven and before physics steps: gives every flinching body the impulses of its hits, lets its dipped strength recover, and once it has held long enough returns the drawn body to the animation and the parts to kinematic.
pub fn update_flinches(
    characters: Query<(
        Entity,
        &mut CharacterRagdoll,
        &RagdollId,
        &RagdollBody,
        &HitReaction,
        &mut Flinch,
    )>,
    mut state: ResMut<PhysicsState>,
    mut cmd: CommandQueue,
) {
    let dt = Time::fixed_delta_time().as_secs_f32();
    let mut pose = Vec::new();
    for (entity, mut character, id, body, reaction, mut flinch) in characters.iter() {
        let flinch = &mut *flinch;
        flinch.pending.retain(|hit| {
            if character.mode != RagdollMode::Powered {
                return false;
            }
            if body.is_kinematic(hit.part) {
                return body.is_changing(hit.part);
            }
            if let Some(part) = state.ragdoll_bodies(*id).get(hit.part).copied() {
                state.add_impulse_at(part, hit.impulse, hit.point);
            }
            false
        });
        flinch.since_hit += dt;
        flinch.lasted += dt;
        let regained = if reaction.recover > 0.0 {
            dt / reaction.recover
        } else {
            1.0
        };
        for scale in &mut flinch.scales {
            *scale = (*scale + regained).min(1.0);
        }
        state.ragdoll_pose(*id, &mut pose);
        if let (Some(simulated), Some(animated)) =
            (pose.get(flinch.part), body.animated_pose().get(flinch.part))
        {
            flinch.peak = flinch
                .peak
                .max(simulated.translation.distance(animated.translation));
        }
        let over = flinch.since_hit - reaction.hold;
        let weight = if over <= 0.0 {
            1.0
        } else if reaction.settle > 0.0 {
            1.0 - over / reaction.settle
        } else {
            0.0
        };
        let done = weight <= 0.0 || character.mode != RagdollMode::Powered;
        if character.mode == RagdollMode::Powered {
            character.weight = weight.clamp(0.0, 1.0);
        }
        cmd.insert(
            FlinchRecord {
                part: flinch.part,
                peak_displacement: flinch.peak,
                recovered_after: done.then_some(flinch.lasted),
            },
            entity,
        );
        if done {
            if character.mode == RagdollMode::Powered {
                character.mode = RagdollMode::FollowsAnimation;
                character.weight = 0.0;
            }
            cmd.remove::<Flinch>(entity);
        }
    }
}
