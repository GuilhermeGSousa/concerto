use concerto_ecs::{
    CommandQueue, Component, Entity, Query, ResMut,
    reflect::{ReflectComponent, ReflectSceneComponent},
    signal::On,
};
use concerto_foundation::time::Time;
use concerto_name::Name;
use concerto_physics::{
    physics_state::PhysicsState,
    ragdoll::{BodyPose, RagdollId},
};
use concerto_reflect::{Reflect, ReflectDefault};
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::{
    asset::{PartStrength, Segment, Side},
    balance::{BALANCE, Balance},
    behaviour::{BehaviourEnded, BodyDrive, BodyState, Outcome, PartCommand, PhysicalState},
    character::{CharacterRagdoll, RagdollBody, RagdollMode},
    hit::{BodyHit, HitKind},
    ik::{Chain, bent_spine, reach_hinged},
    node::balance::{ClaimsOut, Hindrance},
    resolve::{Limbs, ResolvedRagdoll},
    sensing::BodySense,
    view::BodyView,
};

/// The name the shot behaviour claims parts and reports under.
pub const SHOT: &str = "shot";
/// The priority of the shot behaviour's claims.
pub const SHOT_PRIORITY: i32 = 30;

/// How a gunshot reaction plays out: how hard the muscles at a wound snap and how far they give, what a wounded part keeps, how the spine folds, and when a hand goes to the wound and the head turns to it.
#[derive(Clone, Debug, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Default)]
pub struct ShotSettings {
    /// The stiffness of the muscles the reaction drives, in N·m/rad.
    pub stiffness: f32,
    /// The most torque one of them applies, in N·m.
    pub torque: f32,
    /// How many times stronger the muscles at the wound are during the snap.
    pub spike: f32,
    /// The seconds the snap lasts.
    pub snap_seconds: f32,
    /// The share of their strength the muscles at the wound have right after the snap.
    pub slack: f32,
    /// The seconds over which they come back from `slack` to `wounded`.
    pub recover: f32,
    /// The share of its strength a wounded part keeps afterwards.
    pub wounded: f32,
    /// The share of its strength a shot arm keeps: it hangs.
    pub hanging_arm: f32,
    /// The share of its strength a shot leg keeps; under a half it is never the leg stood on.
    pub lame_leg: f32,
    /// How far the spine folds over a wound in the trunk, in radians.
    pub fold: f32,
    /// The seconds over which the fold straightens.
    pub fold_seconds: f32,
    /// The seconds after a hit before a hand goes to the wound.
    pub reach_delay: f32,
    /// The seconds the head looks at the wound before it turns to where the shot came from.
    pub look_seconds: f32,
    /// The furthest the head turns from where the animation has it, in radians.
    pub max_look: f32,
}

impl Default for ShotSettings {
    fn default() -> Self {
        Self {
            stiffness: 400.0,
            torque: 120.0,
            spike: 3.0,
            snap_seconds: 0.1,
            slack: 0.3,
            recover: 0.8,
            wounded: 0.6,
            hanging_arm: 0.05,
            lame_leg: 0.08,
            fold: 0.6,
            fold_seconds: 1.5,
            reach_delay: 0.25,
            look_seconds: 0.7,
            max_look: 0.8,
        }
    }
}

impl ShotSettings {
    /// Sets the snap: how many times stronger the muscles at the wound get, for how long, and the share of strength left right after.
    pub fn with_snap(mut self, spike: f32, snap_seconds: f32, slack: f32) -> Self {
        self.spike = spike;
        self.snap_seconds = snap_seconds;
        self.slack = slack;
        self
    }

    /// Sets the shares of strength a wounded part, a shot arm and a shot leg keep.
    /// Sets the shares of strength a wounded part, a shot arm and a shot leg keep.
    pub fn with_wounds(mut self, wounded: f32, hanging_arm: f32, lame_leg: f32) -> Self {
        self.wounded = wounded;
        self.hanging_arm = hanging_arm;
        self.lame_leg = lame_leg;
        self
    }

    /// Sets how far the spine folds over a wound in the trunk, in radians, and the seconds over
    /// which it straightens.
    pub fn with_fold(mut self, fold: f32, fold_seconds: f32) -> Self {
        self.fold = fold;
        self.fold_seconds = fold_seconds;
        self
    }
}

impl ShotSettings {
    pub(crate) fn lasting_share(&self, segment: Segment) -> Option<f32> {
        match segment {
            Segment::Head => None,
            _ if is_arm(segment) => Some(self.hanging_arm),
            _ if is_leg(segment) => Some(self.lame_leg),
            _ => Some(self.wounded),
        }
    }
}

/// Makes a character answer a `BodyHit` of `HitKind::Shot` with its whole body, by where it struck, instead of flinching.
///
/// The muscles at the wound snap tight and then give, a hand goes to the
/// wound, the head turns to it and then to where the shot came from, and
/// balance keeps the body up or loses it, as its [`ShotSettings`] say.
#[derive(Component, Reflect, Clone, Debug, Default, PartialEq)]
#[reflect(Component, Default, SceneComponent)]
pub struct ShotReaction {
    /// How the reaction plays out.
    pub settings: ShotSettings,
}

impl ShotReaction {
    /// Sets `settings`.
    pub fn with_settings(mut self, settings: ShotSettings) -> Self {
        self.settings = settings;
        self
    }
}

/// One wound of a shot in progress.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wound {
    /// The index of the struck part.
    pub part: usize,
    /// Where the bullet struck, in the part's own space.
    pub point: Vec3,
    /// The impulse of the bullet, in world space.
    pub impulse: Vec3,
    /// The seconds since it struck.
    pub age: f32,
    waiting: Option<u32>,
}

impl Wound {
    pub(crate) fn struck(part: usize, point: Vec3, impulse: Vec3) -> Self {
        Self {
            part,
            point,
            impulse,
            age: 0.0,
            waiting: None,
        }
    }
}

/// The share of its strength each wounded part of a character keeps.
///
/// Every character ragdoll carries it. The shot behaviour, and a
/// [`ShotNode`](crate::node::ShotNode) through its
/// [`Wounded`](crate::messages::Wounded) signals, leave it on the character
/// for the game to read: a limp, a guarded arm.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct Wounds {
    strength: Vec<f32>,
}

impl Wounds {
    pub(crate) fn none() -> &'static Wounds {
        static NONE: Wounds = Wounds {
            strength: Vec::new(),
        };
        &NONE
    }

    /// The share of its strength `part` keeps, from 0 to 1; 1 for a part that was never hit.
    pub fn strength_of(&self, part: usize) -> f32 {
        self.strength.get(part).copied().unwrap_or(1.0)
    }

    pub(crate) fn weaken(&mut self, part: usize, share: f32, parts: usize) {
        self.strength.resize(parts, 1.0);
        if let Some(strength) = self.strength.get_mut(part) {
            *strength = strength.min(share.clamp(0.0, 1.0));
        }
    }
}

/// A shot reaction in progress, beside the [`ShotReaction`] that started it.
///
/// It runs on a `Balance` and ends when that ends, with the same outcome.
#[derive(Component, Clone, Debug, Default, PartialEq)]
#[require(Balance, BodyDrive, BodySense, PhysicalState, Wounds)]
pub struct Shot {
    wounds: Vec<Wound>,
    ended: Option<Outcome>,
}

impl Shot {
    /// The wounds so far, oldest first.
    pub fn wounds(&self) -> &[Wound] {
        &self.wounds
    }
}

const PATIENCE: u32 = 10;

fn joined(resolved: &ResolvedRagdoll, a: usize, b: usize) -> bool {
    let parts = &resolved.settings().parts;
    parts[a].parent == Some(b) || parts[b].parent == Some(a)
}

fn is_arm(segment: Segment) -> bool {
    matches!(
        segment,
        Segment::UpperArm | Segment::LowerArm | Segment::Hand
    )
}

fn is_leg(segment: Segment) -> bool {
    matches!(segment, Segment::Thigh | Segment::Shin | Segment::Foot)
}

fn is_trunk(segment: Segment) -> bool {
    matches!(segment, Segment::Pelvis | Segment::Spine)
}

/// Starts a character's shot reaction, or adds a wound to the one in progress, when it is hit by a shot while it is on its feet.
///
/// A character with no [`ShotReaction`], or one that is limp, falling, down
/// or dead, is left to `flinch_when_hit`.
pub fn react_when_shot(
    on: On<BodyHit>,
    mut characters: Query<(
        &CharacterRagdoll,
        &RagdollId,
        &RagdollBody,
        Option<&ShotReaction>,
        Option<&PhysicalState>,
        Option<&mut Shot>,
    )>,
    state: ResMut<PhysicsState>,
    mut cmd: CommandQueue,
) {
    let hit = *on.signal();
    let Some((character, id, body, reaction, physical, shot)) =
        characters.get_entity_mut(hit.character)
    else {
        return;
    };
    if !takes_shot(&hit, character, reaction.is_some(), physical)
        || hit.part >= body.resolved().part_count()
    {
        return;
    }
    let mut pose = Vec::new();
    state.ragdoll_pose(*id, &mut pose);
    let Some(struck) = pose.get(hit.part) else {
        return;
    };
    let wound = Wound {
        waiting: Some(0),
        ..Wound::struck(
            hit.part,
            struck.inverse().transform_point(hit.point),
            hit.impulse,
        )
    };
    match shot {
        Some(mut shot) => shot.wounds.push(wound),
        None => cmd.insert(
            Shot {
                wounds: vec![wound],
                ended: None,
            },
            hit.character,
        ),
    }
}

/// Whether a hit is answered by the shot behaviour and not by the flinch.
pub(crate) fn takes_shot(
    hit: &BodyHit,
    character: &CharacterRagdoll,
    reacts: bool,
    physical: Option<&PhysicalState>,
) -> bool {
    hit.kind == HitKind::Shot
        && reacts
        && character.mode != RagdollMode::Limp
        && physical.is_none_or(|physical| {
            matches!(physical.state, BodyState::Animated | BodyState::Reacting)
        })
}

/// When a shot character's balance ends, notes its outcome on the shot.
pub fn note_shot_balance_ended(on: On<BehaviourEnded>, mut shots: Query<&mut Shot>) {
    let ended = on.signal();
    if ended.behaviour == Name::from(BALANCE)
        && let Some(mut shot) = shots.get_entity_mut(ended.character)
    {
        shot.ended = Some(ended.outcome);
    }
}

struct Arm {
    side: Side,
    upper: usize,
    lower: usize,
    hand: usize,
}

impl Arm {
    fn of(limbs: &Limbs, side: Side) -> Option<Self> {
        Some(Self {
            side,
            upper: limbs.upper_arm?,
            lower: limbs.lower_arm?,
            hand: limbs.hand?,
        })
    }
}

/// Drives every shot body, each fixed step in `BehaviourSet` before `balance_bodies`.
///
/// It gives the struck parts their impulses once they are simulated, snaps
/// and slackens the muscles at each wound, folds the spine over a wound in
/// the trunk, lets a shot arm hang and weakens a shot leg through balance.
/// It sends the nearer free hand to the newest wound and turns the head to
/// it and then to where the shot came from.
pub fn shot_bodies(
    mut characters: Query<(
        Entity,
        &ShotReaction,
        &mut Shot,
        &mut Wounds,
        &mut Balance,
        &RagdollId,
        &RagdollBody,
        &mut BodyDrive,
    )>,
    mut state: ResMut<PhysicsState>,
) {
    let dt = Time::fixed_delta_time().as_secs_f32();
    let mut claims = ClaimsOut::default();
    for (entity, reaction, mut shot, mut wounds, mut balance, id, body, mut drive) in
        characters.iter_mut()
    {
        let shot = &mut *shot;
        shot.strike_wounds(*id, body, &mut state);
        let resolved = body.resolved();
        let count = resolved.part_count();
        for wound in &shot.wounds {
            if let Some(share) = reaction
                .settings
                .lasting_share(resolved.segment(wound.part))
            {
                wounds.weaken(wound.part, share, count);
            }
        }
        let view = BodyView::new(entity, *id, body, None, None);
        let Some(legs) = (ShotController {
            settings: &reaction.settings,
            wounds: &mut shot.wounds,
        })
        .step(view, &state, dt, &mut claims) else {
            continue;
        };
        balance.hinder(legs);
        for part in 0..count {
            if let Some(claim) = claims.get(part) {
                drive.claim(SHOT, part, SHOT_PRIORITY, claim.weight, claim.command);
            }
        }
    }
}

pub(crate) struct ShotController<'a> {
    pub(crate) settings: &'a ShotSettings,
    pub(crate) wounds: &'a mut [Wound],
}

impl ShotController<'_> {
    pub(crate) fn step(
        &mut self,
        view: BodyView<'_>,
        physics: &PhysicsState,
        dt: f32,
        claims: &mut ClaimsOut,
    ) -> Option<Hindrance> {
        let resolved = view.resolved();
        let landmarks = resolved.landmarks();
        let settings = &resolved.settings().parts;
        let count = settings.len();
        claims.reset(count);
        let animated = view.animated();
        let mut pose = Vec::new();
        physics.ragdoll_pose(view.ragdoll_id(), &mut pose);
        if pose.len() != count || animated.len() != count {
            return None;
        }
        let reaction = self.settings;
        for wound in self.wounds.iter_mut() {
            wound.age += dt;
        }
        let wounds = &*self.wounds;

        let mut muscles = Muscles {
            scale: vec![1.0_f32; count],
            touched: vec![false; count],
            targets: vec![None; count],
        };
        let legs = weaken_wounded(reaction, wounds, resolved, &mut muscles);

        let pelvis = landmarks.pelvis.unwrap_or(0);
        let turned = pose[pelvis].rotation * animated[pelvis].rotation.inverse();
        let carried: Vec<Quat> = animated
            .iter()
            .map(|part| (turned * part.rotation).normalize())
            .collect();
        let rest: Vec<BodyPose> = settings.iter().map(|part| part.pose).collect();

        let fold = fold_over_wounds(reaction, wounds, resolved, dt);
        for aimed in bent_spine(
            &landmarks.spine,
            settings,
            animated,
            turned.inverse() * fold,
        ) {
            muscles.targets[aimed.part] = Some(aimed.target);
            muscles.touched[aimed.part] = true;
        }

        let across = (pose[pelvis].rotation * rest[pelvis].rotation.inverse() * Vec3::X)
            .try_normalize()
            .unwrap_or(Vec3::X);
        let forward = across.cross(Vec3::Y).try_normalize().unwrap_or(Vec3::Z);
        let posed = Posed {
            resolved,
            pose: &pose,
            rest: &rest,
            carried: &carried,
            across,
            forward,
        };
        hand_to_wound(reaction, wounds, &posed, &mut muscles);
        look_at_wound(reaction, wounds, &posed, &mut muscles);
        claim_muscles(reaction, &muscles, claims);
        Some(legs)
    }
}

struct Muscles {
    scale: Vec<f32>,
    touched: Vec<bool>,
    targets: Vec<Option<Quat>>,
}

struct Posed<'a> {
    resolved: &'a ResolvedRagdoll,
    pose: &'a [BodyPose],
    rest: &'a [BodyPose],
    carried: &'a [Quat],
    across: Vec3,
    forward: Vec3,
}

impl Shot {
    fn strike_wounds(&mut self, id: RagdollId, body: &RagdollBody, state: &mut PhysicsState) {
        let mut pose = Vec::new();
        state.ragdoll_pose(id, &mut pose);
        for wound in &mut self.wounds {
            let Some(waited) = wound.waiting else {
                continue;
            };
            if !body.is_kinematic(wound.part) {
                if let (Some(part), Some(struck)) = (
                    state.ragdoll_bodies(id).get(wound.part).copied(),
                    pose.get(wound.part),
                ) {
                    state.add_impulse_at(part, wound.impulse, struck.transform_point(wound.point));
                }
                wound.waiting = None;
            } else if waited >= PATIENCE {
                wound.waiting = None;
            } else {
                wound.waiting = Some(waited + 1);
            }
        }
    }
}

fn weaken_wounded(
    reaction: &ShotSettings,
    struck: &[Wound],
    resolved: &ResolvedRagdoll,
    muscles: &mut Muscles,
) -> Hindrance {
    let mut legs = Hindrance::default();
    for wound in struck {
        let segment = resolved.segment(wound.part);
        let Some(lasting) = reaction.lasting_share(segment) else {
            continue;
        };
        let healed = if reaction.recover > 0.0 {
            ((wound.age - reaction.snap_seconds) / reaction.recover).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let now = if wound.age < reaction.snap_seconds {
            reaction.spike
        } else {
            reaction.slack.min(lasting) + (lasting - reaction.slack.min(lasting)) * healed
        };
        if is_leg(segment) {
            let share = now.min(1.0);
            match resolved.side(wound.part) {
                Side::Left => legs.left_leg = legs.left_leg.min(share),
                Side::Right => legs.right_leg = legs.right_leg.min(share),
                Side::Centre => {}
            }
            continue;
        }
        spread_wound(reaction, wound, resolved, now, muscles);
    }
    legs
}

fn spread_wound(
    reaction: &ShotSettings,
    wound: &Wound,
    resolved: &ResolvedRagdoll,
    now: f32,
    muscles: &mut Muscles,
) {
    let segment = resolved.segment(wound.part);
    let pelvis = resolved.landmarks().pelvis;
    for part in 0..muscles.scale.len() {
        let near = part == wound.part || joined(resolved, part, wound.part);
        let limb = is_arm(segment)
            && is_arm(resolved.segment(part))
            && resolved.side(part) == resolved.side(wound.part);
        if !(near || limb) || is_leg(resolved.segment(part)) || Some(part) == pelvis {
            continue;
        }
        let share = if part == wound.part || limb {
            now
        } else {
            now.max(reaction.wounded).min(now.max(1.0))
        };
        muscles.scale[part] = if muscles.touched[part] {
            if share > 1.0 || muscles.scale[part] > 1.0 {
                muscles.scale[part].max(share)
            } else {
                muscles.scale[part].min(share)
            }
        } else {
            share
        };
        muscles.touched[part] = true;
    }
}

fn fold_over_wounds(
    reaction: &ShotSettings,
    struck: &[Wound],
    resolved: &ResolvedRagdoll,
    dt: f32,
) -> Vec3 {
    let mut fold = Vec3::ZERO;
    for wound in struck {
        if !is_trunk(resolved.segment(wound.part)) || reaction.fold_seconds <= 0.0 {
            continue;
        }
        let from = Vec3::new(-wound.impulse.x, 0.0, -wound.impulse.z);
        let Some(from) = from.try_normalize() else {
            continue;
        };
        let left = (1.0 - wound.age / reaction.fold_seconds).clamp(0.0, 1.0);
        let risen = (wound.age / reaction.snap_seconds.max(dt)).clamp(0.0, 1.0);
        fold += Vec3::Y.cross(from) * reaction.fold * left * risen;
    }
    fold.clamp_length_max(reaction.fold)
}

fn hand_to_wound(reaction: &ShotSettings, struck: &[Wound], posed: &Posed, muscles: &mut Muscles) {
    let resolved = posed.resolved;
    let landmarks = resolved.landmarks();
    let settings = &resolved.settings().parts;
    let pose = posed.pose;
    let newest = struck
        .iter()
        .rev()
        .find(|wound| !matches!(resolved.segment(wound.part), Segment::Head | Segment::Hand));
    let Some(wound) = newest.filter(|wound| wound.age >= reaction.reach_delay) else {
        return;
    };
    let place = pose[wound.part].transform_point(wound.point);
    let free = |arm: &Arm| {
        !struck.iter().any(|other| {
            is_arm(resolved.segment(other.part)) && resolved.side(other.part) == arm.side
        })
    };
    let arm = [
        Arm::of(&landmarks.left, Side::Left),
        Arm::of(&landmarks.right, Side::Right),
    ]
    .into_iter()
    .flatten()
    .filter(free)
    .min_by(|a, b| {
        let near = |arm: &Arm| pose[arm.hand].translation.distance(place);
        near(a).total_cmp(&near(b))
    });
    if let Some(arm) = arm
        && let Some(shoulder) = settings[arm.upper].parent
    {
        let outward = match arm.side {
            Side::Left => posed.across,
            _ => -posed.across,
        };
        let aimed = reach_hinged(
            &Chain {
                upper: arm.upper,
                lower: arm.lower,
                end: arm.hand,
            },
            settings,
            posed.rest,
            posed.carried,
            pose[arm.upper].translation,
            place,
            outward - Vec3::Y - posed.forward * 0.3,
        );
        let upper = aimed.upper;
        let lower = aimed.lower;
        muscles.targets[arm.upper] = Some((pose[shoulder].rotation.inverse() * upper).normalize());
        muscles.targets[arm.lower] = Some((upper.inverse() * lower).normalize());
        for part in [arm.upper, arm.lower, arm.hand] {
            muscles.touched[part] = true;
        }
    }
}

fn look_at_wound(reaction: &ShotSettings, struck: &[Wound], posed: &Posed, muscles: &mut Muscles) {
    let resolved = posed.resolved;
    let settings = &resolved.settings().parts;
    let pose = posed.pose;
    if let (Some(head), Some(wound)) = (resolved.landmarks().head, struck.last())
        && let Some(neck) = settings[head].parent
        && resolved.segment(wound.part) != Segment::Head
    {
        let eyes = pose[head].translation;
        let towards = if wound.age < reaction.look_seconds {
            pose[wound.part].transform_point(wound.point) - eyes
        } else {
            Vec3::new(-wound.impulse.x, 0.0, -wound.impulse.z)
        };
        if let Some(towards) = towards.try_normalize() {
            let facing = posed.rest[head].rotation.inverse() * Vec3::Z;
            let turn = Quat::from_rotation_arc(posed.carried[head] * facing, towards);
            let angle = turn.to_scaled_axis().length();
            let turn = if angle > reaction.max_look {
                Quat::IDENTITY.slerp(turn, reaction.max_look / angle)
            } else {
                turn
            };
            muscles.targets[head] =
                Some((pose[neck].rotation.inverse() * turn * posed.carried[head]).normalize());
            muscles.touched[head] = true;
        }
    }
}

fn claim_muscles(reaction: &ShotSettings, muscles: &Muscles, claims: &mut ClaimsOut) {
    for part in 0..muscles.scale.len() {
        if !muscles.touched[part] {
            continue;
        }
        let kept = muscles.scale[part].max(0.0);
        let mut command = PartCommand::held(PartStrength::Powered {
            stiffness: reaction.stiffness * kept,
            damping: reaction.stiffness * kept * 0.1,
            torque_limit: reaction.torque * kept,
        });
        if let Some(target) = muscles.targets[part] {
            command = command.with_target(target);
        }
        claims.claim(part, 1.0, command);
    }
}

/// Each fixed step, in `BehaviourSet`: ends every shot reaction whose balance has ended, with the same outcome.
pub fn end_shots(shots: Query<(Entity, &Shot)>, mut cmd: CommandQueue) {
    for (character, shot) in shots.iter() {
        let Some(outcome) = shot.ended else {
            continue;
        };
        cmd.remove::<Shot>(character);
        cmd.trigger(BehaviourEnded {
            character,
            behaviour: SHOT.into(),
            outcome,
        });
    }
}
