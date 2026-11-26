use concerto_animation::player::AnimationPlayer;
use concerto_ecs::{
    CommandQueue, Component, Entity, Query, ResMut, With, Without,
    reflect::{ReflectComponent, ReflectSceneComponent},
    signal::{
        On, Signal,
        listener::{IntoListener, Listener},
    },
};
use concerto_physics::{physics_state::PhysicsState, ragdoll::RagdollId};
use concerto_reflect::{Reflect, ReflectDefault};

use crate::{
    character::RagdollBody,
    hit::BodyHit,
    messages::{BodyDead, Death},
    sensing::BodySense,
};

/// The game says a character is dying: its strength drains over `seconds`, from the hands and feet inwards, and it is `Dead` when none is left. Zero seconds is an instant kill. The engine has no health; the game decides when. A `Die` sent to a `Settled` body is dropped, even if a hit wakes it later.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Die {
    pub character: Entity,
    pub seconds: f32,
}

impl Signal for Die {}

/// A character's dead body has come to rest and left the simulation; triggered for every listener, each time it settles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BodySettled {
    pub character: Entity,
}

impl Signal for BodySettled {}

/// A character whose strength a behaviour drained to nothing; inserted when `BodyDead` triggers on it, and read by the settling of its body. It is never removed: death is final.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Dead;

/// The listener every character ragdoll carries: marks the character `Dead` when a `BodyDead` signal reaches it.
pub fn mark_dead() -> Listener<BodyDead> {
    (|on: On<BodyDead>, mut cmd: CommandQueue| {
        cmd.insert(Dead, on.entity());
    })
    .into_listener()
}

/// A dead body that lies at rest: its parts are static where they lie, nothing moves or senses it, and its capsule stays where it was left, out of every collision layer. Its graph still runs, but nothing it asks reaches physics. A `BodyHit` of at least `wake_impulse` wakes it, and it settles again.
#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq)]
#[reflect(Component, Default, SceneComponent)]
pub struct Settled {
    /// The least impulse of a `BodyHit` that wakes the body, in N·s.
    pub wake_impulse: f32,
}

impl Default for Settled {
    fn default() -> Self {
        Self { wake_impulse: 50.0 }
    }
}

/// Sends a dying character's animation graph a `Death` message; a `Settled` body is skipped and the death is lost.
pub fn forward_death(on: On<Die>, mut players: Query<&mut AnimationPlayer, Without<Settled>>) {
    let die = *on.signal();
    if let Some(mut player) = players.get_entity_mut(die.character) {
        player.send(Death {
            seconds: die.seconds,
        });
    }
}

/// Each fixed step, after the body is sensed: freezes every dead body that lies at rest, marks it `Settled` and triggers `BodySettled`.
pub fn settle_dead_bodies(
    characters: Query<(Entity, &BodySense, &RagdollId), (With<Dead>, Without<Settled>)>,
    mut state: ResMut<PhysicsState>,
    mut cmd: CommandQueue,
) {
    for (entity, sense, id) in characters.iter() {
        if !sense.at_rest {
            continue;
        }
        state.set_ragdoll_frozen(*id, true);
        cmd.insert(Settled::default(), entity);
        cmd.trigger(BodySettled { character: entity });
    }
}

/// Wakes a settled body that is hit hard enough: its parts are simulated again, the struck part takes the impulse, and it settles once it is at rest again.
pub fn wake_when_hit(
    on: On<BodyHit>,
    mut characters: Query<(&Settled, &RagdollId, &RagdollBody, &mut BodySense)>,
    mut state: ResMut<PhysicsState>,
    mut cmd: CommandQueue,
) {
    let hit = *on.signal();
    let Some((settled, id, body, mut sense)) = characters.get_entity_mut(hit.character) else {
        return;
    };
    if hit.impulse.length() < settled.wake_impulse || hit.part >= body.resolved().part_count() {
        return;
    }
    state.set_ragdoll_frozen(*id, false);
    let part = state.ragdoll_bodies(*id)[hit.part];
    state.add_impulse_at(part, hit.impulse, hit.point);
    sense.disturb();
    cmd.remove::<Settled>(hit.character);
}
