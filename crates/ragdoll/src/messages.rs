use concerto_animation::{message::AnimationMessage, player::AnimationPlayer};
use concerto_ecs::{
    Component, Query, ResMut, Without,
    signal::{
        EntitySignal, On, Signal,
        listener::{IntoListener, Listener},
    },
};
use concerto_physics::{physics_state::PhysicsState, ragdoll::RagdollId};
use concerto_reflect::{Reflect, ReflectDefault};
use glam::Vec3;

use crate::{
    character::RagdollBody,
    dying::Settled,
    hit::{BodyHit, HitKind},
    shot::Wounds,
};

/// A blow on `part` of a character's ragdoll at the world `point`, with `impulse` in N·s, sent to its animation graph.
#[derive(Clone, Debug, Default, PartialEq, Reflect)]
#[reflect(Default)]
pub struct Hit {
    pub part: usize,
    pub point: Vec3,
    pub impulse: Vec3,
}

impl AnimationMessage for Hit {}

/// A gunshot on `part` of a character's ragdoll at the world `point`, with `impulse` in N·s, sent to its animation graph.
#[derive(Clone, Debug, Default, PartialEq, Reflect)]
#[reflect(Default)]
pub struct Shot {
    pub part: usize,
    pub point: Vec3,
    pub impulse: Vec3,
}

impl AnimationMessage for Shot {}

/// Tells a graph to stagger towards `direction`, with `severity` from 0 to 1.
#[derive(Clone, Debug, Default, PartialEq, Reflect)]
#[reflect(Default)]
pub struct Stagger {
    pub direction: Vec3,
    pub severity: f32,
}

impl AnimationMessage for Stagger {}

/// Tells a graph its character is dying, its strength draining over `seconds`.
#[derive(Clone, Debug, Default, PartialEq, Reflect)]
#[reflect(Default)]
pub struct Death {
    pub seconds: f32,
}

impl AnimationMessage for Death {}

/// Raised by a balance, stagger or shot behaviour once the body has stood still on its feet.
#[derive(Clone, Debug, Default, PartialEq, Reflect)]
#[reflect(Default)]
pub struct Steadied;

impl Signal for Steadied {}

impl EntitySignal for Steadied {}

impl AnimationMessage for Steadied {}

/// Raised by a balance, stagger or shot behaviour once it has given up keeping the body on its feet.
#[derive(Clone, Debug, Default, PartialEq, Reflect)]
#[reflect(Default)]
pub struct Toppled;

impl Signal for Toppled {}

impl EntitySignal for Toppled {}

impl AnimationMessage for Toppled {}

/// Raised by a flinch behaviour once the body has recovered from its hit.
#[derive(Clone, Debug, Default, PartialEq, Reflect)]
#[reflect(Default)]
pub struct Recovered;

impl Signal for Recovered {}

impl EntitySignal for Recovered {}

impl AnimationMessage for Recovered {}

/// Raised by a shot behaviour for the part it wounds: the part keeps `share` of its strength.
#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
pub struct Wounded {
    pub part: usize,
    pub share: f32,
}

impl Signal for Wounded {}

impl EntitySignal for Wounded {}

impl AnimationMessage for Wounded {}

/// Raised by a fall behaviour once the body has landed and relaxed, and triggered on the character.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
#[reflect(Default)]
pub struct BodyDown;

impl Signal for BodyDown {}

impl EntitySignal for BodyDown {}

impl AnimationMessage for BodyDown {}

/// Raised by a dying behaviour once it has drained the body's last strength, and triggered on the character.
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

/// Forwards a `BodyHit` to the character's animation graph as a `Hit` or a `Shot`, and takes its impulse, pending until the part is dynamic.
pub fn route_hits(
    on: On<BodyHit>,
    mut characters: Query<
        (
            &RagdollBody,
            &mut PendingImpulses,
            Option<&mut AnimationPlayer>,
        ),
        Without<Settled>,
    >,
) {
    let hit = *on.signal();
    let Some((body, mut pending, player)) = characters.get_entity_mut(hit.character) else {
        return;
    };
    if hit.part >= body.resolved().part_count() {
        return;
    }
    pending.waiting.push(PendingImpulse {
        part: hit.part,
        point: hit.point,
        impulse: hit.impulse,
        waited: 0,
    });
    let Some(mut player) = player else {
        return;
    };
    match hit.kind {
        HitKind::Blow => player.send(Hit {
            part: hit.part,
            point: hit.point,
            impulse: hit.impulse,
        }),
        HitKind::Shot => player.send(Shot {
            part: hit.part,
            point: hit.point,
            impulse: hit.impulse,
        }),
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

/// The listener every character ragdoll carries: keeps what a `Wounded` signal says on its `Wounds`.
pub fn keep_wounds() -> Listener<Wounded> {
    (|on: On<Wounded>, mut characters: Query<(&RagdollBody, &mut Wounds)>| {
        let wounded = *on.signal();
        let Some((body, mut wounds)) = characters.get_entity_mut(on.entity()) else {
            return;
        };
        wounds.weaken(wounded.part, wounded.share, body.resolved().part_count());
    })
    .into_listener()
}
