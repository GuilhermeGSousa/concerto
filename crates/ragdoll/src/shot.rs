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

use crate::{
    asset::{PartStrength, Segment, Side},
    balance::{BALANCE, Balance, Hindrance},
    behaviour::{BehaviourEnded, BodyDrive, BodyState, Outcome, PartCommand, PhysicalState},
    character::{CharacterRagdoll, RagdollBody, RagdollMode},
    hit::{BodyHit, HitKind},
    ik::{Chain, bent_spine, reach_hinged},
    resolve::{Limbs, ResolvedRagdoll},
    sensing::BodySense,
};

/// The name the shot behaviour claims parts and reports under.
pub const SHOT: &str = "shot";
/// The priority of the shot behaviour's claims.
pub const SHOT_PRIORITY: i32 = 30;

/// Makes a character answer a `BodyHit` of `HitKind::Shot` with its whole body, by where it struck, instead of flinching: the muscles at the wound snap tight and then give, a hand goes to the wound, the head turns to it and then to where the shot came from, and balance keeps the body up or loses it.
#[derive(Component, Reflect, Clone, Debug, PartialEq)]
#[reflect(Component, Default, SceneComponent)]
pub struct ShotReaction {
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

impl Default for ShotReaction {
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

impl ShotReaction {
    /// Sets the snap: how many times stronger the muscles at the wound get, for how long, and the share of strength left right after.
    pub fn with_snap(mut self, spike: f32, snap_seconds: f32, slack: f32) -> Self {
        self.spike = spike;
        self.snap_seconds = snap_seconds;
        self.slack = slack;
        self
    }

    /// Sets the shares of strength a wounded part, a shot arm and a shot leg keep.
    pub fn with_wounds(mut self, wounded: f32, hanging_arm: f32, lame_leg: f32) -> Self {
        self.wounded = wounded;
        self.hanging_arm = hanging_arm;
        self.lame_leg = lame_leg;
        self
    }

    pub fn with_fold(mut self, fold: f32, fold_seconds: f32) -> Self {
        self.fold = fold;
        self.fold_seconds = fold_seconds;
        self
    }
}

/// One wound of a shot in progress.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wound {
    pub part: usize,
    /// Where the bullet struck, in the part's own space.
    pub point: Vec3,
    /// The impulse of the bullet, in world space.
    pub impulse: Vec3,
    /// The seconds since it struck.
    pub age: f32,
    waiting: Option<u32>,
}

/// The share of its strength each wounded part of a character keeps. The shot behaviour leaves it on the character when it ends, for the game to read: a limp, a guarded arm.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct Wounds {
    strength: Vec<f32>,
}

impl Wounds {
    /// The share of its strength `part` keeps, from 0 to 1; 1 for a part that was never hit.
    pub fn strength_of(&self, part: usize) -> f32 {
        self.strength.get(part).copied().unwrap_or(1.0)
    }

    fn weaken(&mut self, part: usize, share: f32, parts: usize) {
        self.strength.resize(parts, 1.0);
        if let Some(strength) = self.strength.get_mut(part) {
            *strength = strength.min(share.clamp(0.0, 1.0));
        }
    }
}

/// A shot reaction in progress, beside the `ShotReaction` that started it. It runs on a `Balance` and ends when that ends, with the same outcome.
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

/// Starts a character's shot reaction, or adds a wound to the one in progress, when it is hit by a shot while it is on its feet. A character with no `ShotReaction`, or one that is limp, falling, down or dead, is left to `flinch_when_hit`.
pub fn react_when_shot(
    on: On<BodyHit>,
    characters: Query<(
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
        characters.get_entity(hit.character)
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
        part: hit.part,
        point: struck.inverse().transform_point(hit.point),
        impulse: hit.impulse,
        age: 0.0,
        waiting: Some(0),
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
pub fn note_shot_balance_ended(on: On<BehaviourEnded>, shots: Query<&mut Shot>) {
    let ended = on.signal();
    if ended.behaviour == Name::from(BALANCE)
        && let Some(mut shot) = shots.get_entity(ended.character)
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

/// Each fixed step, in `BehaviourSet` before `balance_bodies`: for every shot body, gives the struck parts their impulses once they are simulated, snaps and slackens the muscles at each wound, folds the spine over a wound in the trunk, lets a shot arm hang, weakens a shot leg through balance, sends the nearer free hand to the newest wound and turns the head to it and then to where the shot came from.
pub fn shot_bodies(
    characters: Query<(
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
    let mut pose = Vec::new();
    for (reaction, mut shot, mut wounds, mut balance, id, body, mut drive) in characters.iter() {
        let shot = &mut *shot;
        let resolved = body.resolved();
        let landmarks = resolved.landmarks();
        let settings = &resolved.settings().parts;
        let count = settings.len();
        let animated = body.animated_pose();
        state.ragdoll_pose(*id, &mut pose);
        if pose.len() != count || animated.len() != count {
            continue;
        }
        let bodies = state.ragdoll_bodies(*id).to_vec();
        for wound in &mut shot.wounds {
            if let Some(waited) = wound.waiting {
                if !body.is_kinematic(wound.part) {
                    state.add_impulse_at(
                        bodies[wound.part],
                        wound.impulse,
                        pose[wound.part].transform_point(wound.point),
                    );
                    wound.waiting = None;
                } else if waited >= PATIENCE {
                    wound.waiting = None;
                } else {
                    wound.waiting = Some(waited + 1);
                }
            }
            wound.age += dt;
        }

        let mut scale = vec![1.0_f32; count];
        let mut touched = vec![false; count];
        let mut legs = Hindrance::default();
        for wound in &shot.wounds {
            let segment = resolved.segment(wound.part);
            if segment == Segment::Head {
                continue;
            }
            let healed = if reaction.recover > 0.0 {
                ((wound.age - reaction.snap_seconds) / reaction.recover).clamp(0.0, 1.0)
            } else {
                1.0
            };
            let lasting = if is_arm(segment) {
                reaction.hanging_arm
            } else if is_leg(segment) {
                reaction.lame_leg
            } else {
                reaction.wounded
            };
            wounds.weaken(wound.part, lasting, count);
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
            for part in 0..count {
                let near = part == wound.part || joined(resolved, part, wound.part);
                let limb = is_arm(segment)
                    && is_arm(resolved.segment(part))
                    && resolved.side(part) == resolved.side(wound.part);
                if !(near || limb)
                    || is_leg(resolved.segment(part))
                    || Some(part) == landmarks.pelvis
                {
                    continue;
                }
                let share = if part == wound.part || limb {
                    now
                } else {
                    now.max(reaction.wounded).min(now.max(1.0))
                };
                scale[part] = if touched[part] {
                    if share > 1.0 || scale[part] > 1.0 {
                        scale[part].max(share)
                    } else {
                        scale[part].min(share)
                    }
                } else {
                    share
                };
                touched[part] = true;
            }
        }
        balance.hinder(legs);

        let mut targets: Vec<Option<Quat>> = vec![None; count];
        let pelvis = landmarks.pelvis.unwrap_or(0);
        let turned = pose[pelvis].rotation * animated[pelvis].rotation.inverse();
        let carried: Vec<Quat> = animated
            .iter()
            .map(|part| (turned * part.rotation).normalize())
            .collect();
        let rest: Vec<BodyPose> = settings.iter().map(|part| part.pose).collect();

        let mut fold = Vec3::ZERO;
        for wound in &shot.wounds {
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
        let fold = fold.clamp_length_max(reaction.fold);
        for aimed in bent_spine(
            &landmarks.spine,
            settings,
            animated,
            turned.inverse() * fold,
        ) {
            targets[aimed.part] = Some(aimed.target);
            touched[aimed.part] = true;
        }

        let across = (pose[pelvis].rotation * rest[pelvis].rotation.inverse() * Vec3::X)
            .try_normalize()
            .unwrap_or(Vec3::X);
        let forward = across.cross(Vec3::Y).try_normalize().unwrap_or(Vec3::Z);
        let newest =
            shot.wounds.iter().rev().find(|wound| {
                !matches!(resolved.segment(wound.part), Segment::Head | Segment::Hand)
            });
        if let Some(wound) = newest.filter(|wound| wound.age >= reaction.reach_delay) {
            let place = pose[wound.part].transform_point(wound.point);
            let free = |arm: &Arm| {
                !shot.wounds.iter().any(|other| {
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
                    Side::Left => across,
                    _ => -across,
                };
                let aimed = reach_hinged(
                    &Chain {
                        upper: arm.upper,
                        lower: arm.lower,
                        end: arm.hand,
                    },
                    settings,
                    &rest,
                    &carried,
                    pose[arm.upper].translation,
                    place,
                    outward - Vec3::Y - forward * 0.3,
                );
                let upper = aimed.upper;
                let lower = aimed.lower;
                targets[arm.upper] = Some((pose[shoulder].rotation.inverse() * upper).normalize());
                targets[arm.lower] = Some((upper.inverse() * lower).normalize());
                for part in [arm.upper, arm.lower, arm.hand] {
                    touched[part] = true;
                }
            }
        }

        if let (Some(head), Some(wound)) = (landmarks.head, shot.wounds.last())
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
                let facing = rest[head].rotation.inverse() * Vec3::Z;
                let turn = Quat::from_rotation_arc(carried[head] * facing, towards);
                let angle = turn.to_scaled_axis().length();
                let turn = if angle > reaction.max_look {
                    Quat::IDENTITY.slerp(turn, reaction.max_look / angle)
                } else {
                    turn
                };
                targets[head] =
                    Some((pose[neck].rotation.inverse() * turn * carried[head]).normalize());
                touched[head] = true;
            }
        }

        for part in 0..count {
            if !touched[part] {
                continue;
            }
            let kept = scale[part].max(0.0);
            let mut command = PartCommand::held(PartStrength::Powered {
                stiffness: reaction.stiffness * kept,
                damping: reaction.stiffness * kept * 0.1,
                torque_limit: reaction.torque * kept,
            });
            if let Some(target) = targets[part] {
                command = command.with_target(target);
            }
            drive.claim(SHOT, part, SHOT_PRIORITY, 1.0, command);
        }
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
