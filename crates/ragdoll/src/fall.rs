use concerto_ecs::{
    CommandQueue, Component, Entity, Query, Res,
    reflect::{ReflectComponent, ReflectSceneComponent},
    signal::On,
};
use concerto_foundation::time::Time;
use concerto_physics::{
    physics_state::PhysicsState,
    ragdoll::{BodyPose, RagdollId},
};
use concerto_reflect::{Reflect, ReflectDefault};
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::{
    asset::{PartStrength, Side},
    behaviour::{
        BehaviourEnded, BodyDrive, BodyState, BodyStateChanged, Outcome, PartCommand, PhysicalState,
    },
    character::{CharacterRagdoll, RagdollBody, RagdollMode},
    generate::BRACED,
    ik::{Chain, bent_spine, reach_hinged},
    node::balance::ClaimsOut,
    resolve::Limbs,
    sensing::BodySense,
    view::BodyView,
};

/// The name the fall behaviour claims parts and reports under.
pub const FALL: &str = "fall";
/// The priority of the fall behaviour's claims.
pub const FALL_PRIORITY: i32 = 15;

pub(crate) const LYING: f32 = 1.0;

/// Starts a `Fall` of this strength on the character whenever its state becomes `Falling`.
#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq)]
#[reflect(Component, Default, SceneComponent)]
pub struct FallReaction {
    /// How conscious the fall is, from 1, fully braced, to 0, limp. The game sets it from the wound.
    pub strength: f32,
}

impl Default for FallReaction {
    fn default() -> Self {
        Self { strength: 1.0 }
    }
}

/// A fall the body takes part in.
///
/// The hands go to where it will land, with bent elbows that buckle on
/// contact, the head is held away from the ground, the spine curls and the
/// knees fold, as its [`FallSettings`] say. It ends when the body has landed
/// and relaxed, limp and `Down`. Add it to start a fall on a standing
/// character too.
#[derive(Component, Reflect, Clone, Debug, PartialEq)]
#[reflect(Component, Default, SceneComponent)]
#[require(BodyDrive, BodySense, PhysicalState)]
pub struct Fall {
    /// How conscious the fall is, from 1, fully braced, to 0, which is the limp fall.
    pub strength: f32,
    /// How the fall plays out.
    pub settings: FallSettings,
    #[reflect(skip)]
    started: bool,
    #[reflect(skip)]
    run: FallRun,
}

impl Default for Fall {
    fn default() -> Self {
        Self {
            strength: 1.0,
            settings: FallSettings::default(),
            started: false,
            run: FallRun::default(),
        }
    }
}

impl Fall {
    /// Sets how conscious the fall is, from 1, fully braced, to 0, limp.
    pub fn with_strength(mut self, strength: f32) -> Self {
        self.strength = strength;
        self
    }

    /// Sets `settings`.
    pub fn with_settings(mut self, settings: FallSettings) -> Self {
        self.settings = settings;
        self
    }

    /// Sets how an arm gives once its hand is down: over how many seconds, to what share of its strength.
    pub fn with_buckle(mut self, buckle_seconds: f32, buckled: f32) -> Self {
        self.settings.buckle_seconds = buckle_seconds;
        self.settings.buckled = buckled;
        self
    }

    /// Sets how the body relaxes after landing: over how many seconds, to what share of its strength.
    pub fn with_relax(mut self, relax_seconds: f32, resting: f32) -> Self {
        self.settings.relax_seconds = relax_seconds;
        self.settings.resting = resting;
        self
    }

    /// Whether the body has touched down.
    pub fn landed(&self) -> bool {
        self.run.landed()
    }
}

/// Starts a fall on a character with a `FallReaction` when its state becomes `Falling`.
pub fn fall_when_falling(
    on: On<BodyStateChanged>,
    characters: Query<(&FallReaction, Option<&Fall>)>,
    mut cmd: CommandQueue,
) {
    let changed = on.signal();
    if changed.to != BodyState::Falling {
        return;
    }
    if let Some((reaction, None)) = characters.get_entity(changed.character) {
        cmd.insert(
            Fall::default().with_strength(reaction.strength),
            changed.character,
        );
    }
}

/// How a fall plays out: the muscles, how the arms reach and buckle, how the head, spine and legs are held, and how the body relaxes once it has landed.
#[derive(Clone, Debug, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Default)]
pub struct FallSettings {
    /// The stiffness of the muscles at strength 1, in N·m/rad.
    pub stiffness: f32,
    /// The most torque one of them applies at strength 1, in N·m.
    pub torque: f32,
    /// The share of that strength the legs have: they fold.
    pub legs: f32,
    /// The bend of each knee and hip, in radians.
    pub knee_bend: f32,
    /// The share of the arm's length a hand reaches, under 1 so the elbow stays bent.
    pub reach: f32,
    /// How far to its own side of the landing point each hand goes, in metres.
    pub spread: f32,
    /// The seconds over which an arm gives after its hand touched down.
    pub buckle_seconds: f32,
    /// The share of its strength a buckled arm keeps.
    pub buckled: f32,
    /// How far the head is held away from the way the body travels, in radians.
    pub tuck: f32,
    /// How far the spine curls forward, in radians.
    pub curl: f32,
    /// How far the spine twists when the fall is backwards, so one shoulder leads, in radians.
    pub twist: f32,
    /// The seconds before a predicted impact on something upright within which the hands go to it at chest height.
    pub brace_seconds: f32,
    /// The seconds after landing over which the strength falls to `resting`; the body is limp after them.
    pub relax_seconds: f32,
    /// The share of its strength the body has when it has relaxed.
    pub resting: f32,
}

impl Default for FallSettings {
    fn default() -> Self {
        Self {
            stiffness: 300.0,
            torque: 100.0,
            legs: 0.3,
            knee_bend: 0.6,
            reach: 0.85,
            spread: 0.2,
            buckle_seconds: 0.3,
            buckled: 0.15,
            tuck: 0.5,
            curl: 0.3,
            twist: 0.4,
            brace_seconds: 0.6,
            relax_seconds: 1.0,
            resting: 0.1,
        }
    }
}

impl FallSettings {
    /// Sets `stiffness`.
    pub fn with_stiffness(mut self, stiffness: f32) -> Self {
        self.stiffness = stiffness;
        self
    }

    /// Sets `torque`.
    pub fn with_torque(mut self, torque: f32) -> Self {
        self.torque = torque;
        self
    }

    /// Sets `legs`.
    pub fn with_legs(mut self, legs: f32) -> Self {
        self.legs = legs;
        self
    }

    /// Sets `knee_bend`.
    pub fn with_knee_bend(mut self, knee_bend: f32) -> Self {
        self.knee_bend = knee_bend;
        self
    }

    /// Sets `reach`.
    pub fn with_reach(mut self, reach: f32) -> Self {
        self.reach = reach;
        self
    }

    /// Sets `spread`.
    pub fn with_spread(mut self, spread: f32) -> Self {
        self.spread = spread;
        self
    }

    /// Sets `buckle_seconds`.
    pub fn with_buckle_seconds(mut self, buckle_seconds: f32) -> Self {
        self.buckle_seconds = buckle_seconds;
        self
    }

    /// Sets `buckled`.
    pub fn with_buckled(mut self, buckled: f32) -> Self {
        self.buckled = buckled;
        self
    }

    /// Sets `tuck`.
    pub fn with_tuck(mut self, tuck: f32) -> Self {
        self.tuck = tuck;
        self
    }

    /// Sets `curl`.
    pub fn with_curl(mut self, curl: f32) -> Self {
        self.curl = curl;
        self
    }

    /// Sets `twist`.
    pub fn with_twist(mut self, twist: f32) -> Self {
        self.twist = twist;
        self
    }

    /// Sets `brace_seconds`.
    pub fn with_brace_seconds(mut self, brace_seconds: f32) -> Self {
        self.brace_seconds = brace_seconds;
        self
    }

    /// Sets `relax_seconds`.
    pub fn with_relax_seconds(mut self, relax_seconds: f32) -> Self {
        self.relax_seconds = relax_seconds;
        self
    }

    /// Sets `resting`.
    pub fn with_resting(mut self, resting: f32) -> Self {
        self.resting = resting;
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct FallRun {
    left_down: Option<f32>,
    right_down: Option<f32>,
    landed: Option<f32>,
}

impl FallRun {
    pub(crate) fn landed(&self) -> bool {
        self.landed.is_some()
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

struct Leg {
    thigh: usize,
    shin: usize,
    foot: usize,
}

impl Leg {
    fn of(limbs: &Limbs) -> Option<Self> {
        Some(Self {
            thigh: limbs.thigh?,
            shin: limbs.shin?,
            foot: limbs.foot?,
        })
    }
}

fn horizontal(vector: Vec3) -> Vec3 {
    Vec3::new(vector.x, 0.0, vector.z)
}

pub(crate) struct FallController<'a> {
    pub(crate) settings: &'a FallSettings,
    pub(crate) strength: f32,
    pub(crate) run: &'a mut FallRun,
}

impl FallController<'_> {
    pub(crate) fn step(
        &mut self,
        view: BodyView<'_>,
        physics: &PhysicsState,
        dt: f32,
        lying: bool,
        claims: &mut ClaimsOut,
    ) -> bool {
        let fall = self.settings;
        let run = &mut *self.run;
        let resolved = view.resolved();
        let landmarks = resolved.landmarks();
        let settings = &resolved.settings().parts;
        let count = settings.len();
        claims.reset(count);
        let animated = view.animated();
        let mut pose = Vec::new();
        physics.ragdoll_pose(view.ragdoll_id(), &mut pose);
        let (Some(pelvis), Some(chest), Some(sense)) =
            (landmarks.pelvis, landmarks.chest, view.sense())
        else {
            return false;
        };
        if pose.len() != count || animated.len() != count {
            return false;
        }
        let entity = view.entity();
        let bodies = physics.ragdoll_bodies(view.ragdoll_id());
        let touching = |part: usize| {
            physics
                .contacts_of(bodies[part])
                .iter()
                .any(|contact| contact.other != Some(entity))
        };

        let on_limbs = sense.lean > LYING
            && (0..count).any(|part| !landmarks.feet().any(|foot| foot == part) && touching(part));
        let down = lying
            || ((touching(chest) || touching(pelvis) || on_limbs)
                && sense.centre.velocity.length() < 0.5);
        if down || run.landed.is_some() {
            let landed = run.landed.map_or(0.0, |landed| landed + dt);
            run.landed = Some(landed);
            if landed >= fall.relax_seconds {
                return true;
            }
        }
        let relaxed = run.landed.map_or(1.0, |landed| {
            let through = if fall.relax_seconds > 0.0 {
                (landed / fall.relax_seconds).clamp(0.0, 1.0)
            } else {
                1.0
            };
            1.0 + (fall.resting - 1.0) * through
        });
        let strength = self.strength.clamp(0.0, 1.0) * relaxed;

        let muscle = |share: f32| PartStrength::Powered {
            stiffness: fall.stiffness * strength * share,
            damping: fall.stiffness * strength * share * 0.1,
            torque_limit: fall.torque * strength * share,
        };

        let rest: Vec<BodyPose> = settings.iter().map(|part| part.pose).collect();
        let turned = pose[pelvis].rotation * animated[pelvis].rotation.inverse();
        let carried: Vec<Quat> = animated
            .iter()
            .map(|part| (turned * part.rotation).normalize())
            .collect();
        let frame = pose[chest].rotation * rest[chest].rotation.inverse();
        let across = (frame * Vec3::X).try_normalize().unwrap_or(Vec3::X);
        let forward = (frame * Vec3::Z).try_normalize().unwrap_or(Vec3::Z);
        let leaning = horizontal(pose[chest].translation - pose[pelvis].translation);
        let moving = horizontal(sense.centre.velocity);
        let travel = if moving.length() > 0.3 {
            moving
        } else {
            leaning
        }
        .try_normalize()
        .unwrap_or(horizontal(forward).try_normalize().unwrap_or(Vec3::Z));
        let backwards = travel.dot(forward) < -0.3;

        let landing = match sense.chest_impact {
            Some(impact) if impact.normal.y.abs() < 0.5 && impact.seconds < fall.brace_seconds => {
                Vec3::new(impact.point.x, pose[chest].translation.y, impact.point.z)
            }
            Some(impact) => impact.point,
            None => Vec3::new(
                sense.centre.position.x + travel.x * 0.5,
                sense.ground,
                sense.centre.position.z + travel.z * 0.5,
            ),
        };
        for arm in [
            Arm::of(&landmarks.left, Side::Left),
            Arm::of(&landmarks.right, Side::Right),
        ]
        .into_iter()
        .flatten()
        {
            let Some(shoulder) = settings[arm.upper].parent else {
                continue;
            };
            let outward = match arm.side {
                Side::Left => across,
                _ => -across,
            };
            let since = match arm.side {
                Side::Left => &mut run.left_down,
                _ => &mut run.right_down,
            };
            if since.is_some() || touching(arm.hand) {
                *since = Some(since.map_or(0.0, |since| since + dt));
            }
            let given = since.map_or(1.0, |since| {
                let through = if fall.buckle_seconds > 0.0 {
                    (since / fall.buckle_seconds).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                1.0 + (fall.buckled - 1.0) * through
            });
            let length = (rest[arm.lower].translation - rest[arm.upper].translation).length()
                + (rest[arm.hand].translation - rest[arm.lower].translation).length();
            let root = pose[arm.upper].translation;
            let goal = root
                + (landing + horizontal(outward) * fall.spread - root)
                    .clamp_length_max(length * fall.reach.clamp(0.1, 1.0));
            let aimed = reach_hinged(
                &Chain {
                    upper: arm.upper,
                    lower: arm.lower,
                    end: arm.hand,
                },
                settings,
                &rest,
                &carried,
                root,
                goal,
                outward - travel * 0.3,
            );
            claims.claim(
                arm.upper,
                1.0,
                PartCommand::held(muscle(given))
                    .with_target((pose[shoulder].rotation.inverse() * aimed.upper).normalize()),
            );
            claims.claim(
                arm.lower,
                1.0,
                PartCommand::held(muscle(given))
                    .with_target((aimed.upper.inverse() * aimed.lower).normalize()),
            );
            claims.claim(arm.hand, 1.0, PartCommand::held(muscle(given)));
        }

        if let Some(head) = landmarks.head
            && let Some(neck) = settings[head].parent
        {
            let away = Quat::from_axis_angle(
                Vec3::Y.cross(-travel).try_normalize().unwrap_or(across),
                fall.tuck,
            );
            claims.claim(
                head,
                1.0,
                PartCommand::held(muscle(1.5)).with_target(
                    (pose[neck].rotation.inverse() * away * carried[head]).normalize(),
                ),
            );
        }

        let twist = if backwards { fall.twist } else { 0.0 };
        let bend = Vec3::X * fall.curl + Vec3::Y * twist;
        for aimed in bent_spine(&landmarks.spine, settings, animated, bend) {
            claims.claim(
                aimed.part,
                1.0,
                PartCommand::held(muscle(1.0)).with_target(aimed.target),
            );
        }

        let sideways = pose[pelvis].rotation * rest[pelvis].rotation.inverse() * Vec3::X;
        for leg in [Leg::of(&landmarks.left), Leg::of(&landmarks.right)]
            .into_iter()
            .flatten()
        {
            let thigh =
                (Quat::from_axis_angle(sideways, -fall.knee_bend) * carried[leg.thigh]).normalize();
            let shin =
                (Quat::from_axis_angle(sideways, fall.knee_bend) * carried[leg.shin]).normalize();
            claims.claim(
                leg.thigh,
                1.0,
                PartCommand::held(muscle(fall.legs))
                    .with_target((pose[pelvis].rotation.inverse() * thigh).normalize()),
            );
            claims.claim(
                leg.shin,
                1.0,
                PartCommand::held(muscle(fall.legs))
                    .with_target((thigh.inverse() * shin).normalize()),
            );
            claims.claim(leg.foot, 1.0, PartCommand::held(muscle(fall.legs)));
        }

        false
    }
}

/// Drives every falling body with a [`Fall`], each fixed step in `BehaviourSet`.
///
/// It sends each hand towards where the chest will land, or to a wall about
/// to be hit, lets an arm buckle once its hand is down, holds the head away
/// from the way the body travels, curls the spine and folds the legs.
///
/// Once the trunk is down, or the body lies on any part but its feet, and is
/// slow, the character is `Down`, the strength drains, and the body ends
/// limp.
pub fn fall_bodies(
    mut characters: Query<(
        Entity,
        &mut Fall,
        &mut CharacterRagdoll,
        &RagdollId,
        &RagdollBody,
        &BodySense,
        &mut BodyDrive,
        &mut PhysicalState,
    )>,
    state: Res<PhysicsState>,
    mut cmd: CommandQueue,
) {
    let dt = Time::fixed_delta_time().as_secs_f32();
    let mut claims = ClaimsOut::default();
    for (entity, mut fall, mut character, id, body, sense, mut drive, mut physical) in
        characters.iter_mut()
    {
        let fall = &mut *fall;
        let ended = |cmd: &mut CommandQueue| {
            cmd.remove::<Fall>(entity);
            cmd.trigger(BehaviourEnded {
                character: entity,
                behaviour: FALL.into(),
                outcome: Outcome::Succeeded,
            });
        };
        if physical.state == BodyState::Dead {
            cmd.remove::<Fall>(entity);
            continue;
        }
        if !fall.started {
            fall.started = true;
            if physical.state != BodyState::Down {
                physical.state = BodyState::Falling;
            }
            if fall.strength <= 0.0 {
                character.go_limp();
                ended(&mut cmd);
                continue;
            }
            if character.mode != RagdollMode::Powered {
                character.power(BRACED);
            }
        }
        let view = BodyView::new(entity, *id, body, Some(sense), None);
        let relaxed = FallController {
            settings: &fall.settings,
            strength: fall.strength,
            run: &mut fall.run,
        }
        .step(
            view,
            &state,
            dt,
            physical.state == BodyState::Down,
            &mut claims,
        );
        if fall.run.landed() && physical.state != BodyState::Down {
            physical.state = BodyState::Down;
        }
        if relaxed {
            character.go_limp();
            ended(&mut cmd);
            continue;
        }
        for part in 0..body.resolved().part_count() {
            if let Some(claim) = claims.get(part) {
                drive.claim(FALL, part, FALL_PRIORITY, claim.weight, claim.command);
            }
        }
    }
}
