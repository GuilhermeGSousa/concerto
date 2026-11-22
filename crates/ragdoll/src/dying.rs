use concerto_ecs::{
    CommandQueue, Component, Entity, Query, ResMut, Without,
    reflect::{ReflectComponent, ReflectSceneComponent},
    signal::{On, Signal},
};
use concerto_foundation::time::Time;
use concerto_physics::{physics_state::PhysicsState, ragdoll::RagdollId};
use concerto_reflect::{Reflect, ReflectDefault};

use crate::{
    asset::Segment,
    balance::Balance,
    behaviour::{BodyState, PhysicalState},
    character::{CharacterRagdoll, RagdollBody, RagdollMode},
    fall::Fall,
    hit::{BodyHit, Flinch},
    sensing::BodySense,
    shot::Shot,
    stagger::Stagger,
};

/// The game says a character is dying: its strength drains over `seconds`, from the hands and feet inwards, and it is `Dead` when none is left. Zero seconds is an instant kill. The engine has no health; the game decides when.
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

/// A character whose strength is draining. Adding it is the same as triggering `Die`; it leaves when the character is `Dead`.
#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq)]
#[reflect(Component, Default, SceneComponent)]
#[require(BodySense, PhysicalState)]
pub struct Dying {
    /// The seconds over which the strength drains; zero kills at once.
    pub seconds: f32,
    #[reflect(skip)]
    elapsed: f32,
}

impl Default for Dying {
    fn default() -> Self {
        Self::new(1.0)
    }
}

impl Dying {
    pub fn new(seconds: f32) -> Self {
        Self {
            seconds,
            elapsed: 0.0,
        }
    }

    /// The share of its strength a part of `segment` has left, from 1 to 0: hands and feet are spent first, the trunk last.
    pub fn left(&self, segment: Segment) -> f32 {
        if self.seconds <= 0.0 {
            return 0.0;
        }
        let spent_at = match segment {
            Segment::Hand | Segment::Foot => 0.4,
            Segment::LowerArm | Segment::Shin => 0.6,
            Segment::UpperArm | Segment::Thigh | Segment::Head => 0.8,
            Segment::Pelvis | Segment::Spine | Segment::Other => 1.0,
        };
        (1.0 - self.elapsed / self.seconds / spent_at).clamp(0.0, 1.0)
    }
}

/// A dead body that lies at rest: its parts are static where they lie, nothing drives or senses it, and its capsule stays where it was left, out of every collision layer. A `BodyHit` of at least `wake_impulse` wakes it, and it settles again.
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

type Behaviours<'w, 's> = Query<
    'w,
    's,
    (
        Option<&'static Balance>,
        Option<&'static Stagger>,
        Option<&'static Shot>,
        Option<&'static Fall>,
        Option<&'static Flinch>,
    ),
>;

fn kill(
    entity: Entity,
    character: &mut CharacterRagdoll,
    physical: Option<&mut PhysicalState>,
    behaviours: &Behaviours,
    cmd: &mut CommandQueue,
) {
    character.go_limp();
    if let Some(physical) = physical
        && physical.state != BodyState::Dead
    {
        physical.state = BodyState::Dead;
    }
    let Some((balance, stagger, shot, fall, flinch)) = behaviours.get_entity(entity) else {
        return;
    };
    if balance.is_some() {
        cmd.remove::<Balance>(entity);
    }
    if stagger.is_some() {
        cmd.remove::<Stagger>(entity);
    }
    if shot.is_some() {
        cmd.remove::<Shot>(entity);
    }
    if fall.is_some() {
        cmd.remove::<Fall>(entity);
    }
    if flinch.is_some() {
        cmd.remove::<Flinch>(entity);
    }
}

/// Starts a character dying when the game says so; with zero seconds it is limp and `Dead` at once. A character that is already dying dies no slower.
pub fn die_when_told(
    on: On<Die>,
    mut characters: Query<(
        &mut CharacterRagdoll,
        Option<&mut PhysicalState>,
        Option<&mut Dying>,
    )>,
    behaviours: Behaviours,
    mut cmd: CommandQueue,
) {
    let die = *on.signal();
    let Some((mut character, mut physical, dying)) = characters.get_entity_mut(die.character)
    else {
        return;
    };
    if physical
        .as_ref()
        .is_some_and(|physical| physical.state == BodyState::Dead)
    {
        return;
    }
    if die.seconds <= 0.0 {
        if let Some(physical) = physical.as_mut() {
            physical.state = BodyState::Dead;
        }
        kill(die.character, &mut character, None, &behaviours, &mut cmd);
    }
    match dying {
        Some(mut dying) => {
            let remaining = (dying.seconds - dying.elapsed).max(0.0);
            if die.seconds < remaining {
                dying.seconds = die.seconds;
                dying.elapsed = 0.0;
            }
        }
        None => cmd.insert(Dying::new(die.seconds), die.character),
    }
}

/// Each fixed step, in `BehaviourSet`: drains every dying character, whose parts the drive weakens by `Dying::left`, and when nothing is left makes it limp and `Dead` and ends the behaviours still on it.
pub fn drain_dying(
    mut characters: Query<(
        Entity,
        &mut Dying,
        &mut CharacterRagdoll,
        &mut PhysicalState,
    )>,
    behaviours: Behaviours,
    mut cmd: CommandQueue,
) {
    let dt = Time::fixed_delta_time().as_secs_f32();
    for (entity, mut dying, mut character, mut physical) in characters.iter_mut() {
        if character.mode == RagdollMode::FollowsAnimation {
            character.mode = RagdollMode::Powered;
            character.weight = 1.0;
        }
        dying.elapsed += dt;
        if dying.elapsed >= dying.seconds {
            kill(
                entity,
                &mut character,
                Some(&mut physical),
                &behaviours,
                &mut cmd,
            );
            cmd.remove::<Dying>(entity);
        }
    }
}

/// Each fixed step, after the body is sensed: freezes every dead body that lies at rest, marks it `Settled` and triggers `BodySettled`.
pub fn settle_dead_bodies(
    characters: Query<(Entity, &PhysicalState, &BodySense, &RagdollId), Without<Settled>>,
    mut state: ResMut<PhysicsState>,
    mut cmd: CommandQueue,
) {
    for (entity, physical, sense, id) in characters.iter() {
        if physical.state != BodyState::Dead || !sense.at_rest {
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
