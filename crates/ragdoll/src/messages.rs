use concerto_animation::{message::AnimationMessage, player::AnimationPlayer};
use concerto_ecs::{
    Component, Query, ResMut, Without,
    signal::{EntitySignal, On, Signal},
};
use concerto_physics::{physics_state::PhysicsState, ragdoll::RagdollId};
use concerto_reflect::{Reflect, ReflectDefault};
use glam::Vec3;

use crate::{
    character::{CharacterRagdoll, RagdollBody, RagdollMode},
    dying::Settled,
    hit::{BodyHit, HitKind},
};

/// A blow on `part` of a character's ragdoll at the world `point`, with `impulse` in N·s, sent to its animation graph.
#[derive(Clone, Debug, Default, PartialEq, Reflect)]
#[reflect(Default)]
pub struct Hit {
    /// The index of the struck part in the character's ragdoll.
    pub part: usize,
    /// The world-space point the blow landed on.
    pub point: Vec3,
    /// The blow's impulse, in N·s in world space.
    pub impulse: Vec3,
}

impl AnimationMessage for Hit {}

/// Tells a graph its character is dying, its strength draining over `seconds`.
#[derive(Clone, Debug, Default, PartialEq, Reflect)]
#[reflect(Default)]
pub struct Death {
    /// The seconds over which the strength drains; zero drains it at once.
    pub seconds: f32,
}

impl AnimationMessage for Death {}

/// Raised by a [`BalanceNode`](crate::node::BalanceNode) once the body has stood still on its feet, and triggered on the character.
#[derive(Clone, Debug, Default, PartialEq, Reflect)]
#[reflect(Default)]
pub struct Steadied;

impl Signal for Steadied {}

impl EntitySignal for Steadied {}

impl AnimationMessage for Steadied {}

/// Raised by a [`BalanceNode`](crate::node::BalanceNode) once it has given up keeping the body on its feet, and triggered on the character.
#[derive(Clone, Debug, Default, PartialEq, Reflect)]
#[reflect(Default)]
pub struct Toppled;

impl Signal for Toppled {}

impl EntitySignal for Toppled {}

impl AnimationMessage for Toppled {}

/// Raised by a [`FlinchNode`](crate::node::FlinchNode) once the body has recovered from its hit, and triggered on the character.
#[derive(Clone, Debug, Default, PartialEq, Reflect)]
#[reflect(Default)]
pub struct Recovered;

impl Signal for Recovered {}

impl EntitySignal for Recovered {}

impl AnimationMessage for Recovered {}

/// Raised by a [`DyingNode`](crate::node::DyingNode) once it has drained the body's last strength, and triggered on the character.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
#[reflect(Default)]
pub struct BodyDead;

impl Signal for BodyDead {}

impl EntitySignal for BodyDead {}

impl AnimationMessage for BodyDead {}

const PATIENCE: u32 = 10;

#[derive(Clone, Debug)]
struct PendingImpulse {
    part: usize,
    point: Vec3,
    impulse: Vec3,
    waited: u32,
}

/// The impulses of a character's hits that wait for their part to be dynamic.
#[derive(Component, Clone, Debug, Default)]
pub struct PendingImpulses {
    waiting: Vec<PendingImpulse>,
}

/// Forwards a blow's [`BodyHit`] on a character in [`RagdollMode::Graph`] to its animation graph as a [`Hit`], and takes its impulse, pending until the part is dynamic.
///
/// A gunshot takes its impulse the same way but sends no message.
pub fn route_hits(
    on: On<BodyHit>,
    mut characters: Query<
        (
            &CharacterRagdoll,
            &RagdollBody,
            &mut PendingImpulses,
            Option<&mut AnimationPlayer>,
        ),
        Without<Settled>,
    >,
) {
    let hit = *on.signal();
    let Some((character, body, mut pending, player)) = characters.get_entity_mut(hit.character)
    else {
        return;
    };
    if character.mode != RagdollMode::Graph || hit.part >= body.resolved().part_count() {
        return;
    }
    pending.waiting.push(PendingImpulse {
        part: hit.part,
        point: hit.point,
        impulse: hit.impulse,
        waited: 0,
    });
    if let (HitKind::Blow, Some(mut player)) = (hit.kind, player) {
        player.send(Hit {
            part: hit.part,
            point: hit.point,
            impulse: hit.impulse,
        });
    }
}

/// Each fixed step, before physics steps, gives each part the impulses waiting on it once it is dynamic, and drops those that waited ten steps in vain.
pub fn apply_impulses(
    mut characters: Query<(&RagdollId, &RagdollBody, &mut PendingImpulses), Without<Settled>>,
    mut state: ResMut<PhysicsState>,
) {
    for (id, body, mut pending) in characters.iter_mut() {
        if pending.waiting.is_empty() {
            continue;
        }
        pending.waiting.retain_mut(|waiting| {
            if body.is_kinematic(waiting.part) {
                waiting.waited += 1;
                return waiting.waited < PATIENCE;
            }
            if let Some(part) = state.ragdoll_bodies(*id).get(waiting.part).copied() {
                state.add_impulse_at(part, waiting.impulse, waiting.point);
            }
            false
        });
    }
}
