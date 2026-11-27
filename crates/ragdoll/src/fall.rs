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

use crate::{
    asset::{PartStrength, Side},
    behaviour::{
        BehaviourEnded, BodyDrive, BodyState, BodyStateChanged, LYING, Outcome, PartCommand,
        PhysicalState,
    },
    character::{CharacterRagdoll, RagdollBody, RagdollMode},
    generate::BRACED,
    ik::{Chain, bent_spine},
    resolve::Limbs,
    sensing::BodySense,
};

/// The name the fall behaviour claims parts and reports under.
pub const FALL: &str = "fall";
/// The priority of the fall behaviour's claims.
pub const FALL_PRIORITY: i32 = 15;

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
/// knees fold. It ends when the body has landed and relaxed, limp and
/// `Down`. Add it to start a fall on a standing character too.
#[derive(Component, Reflect, Clone, Debug, PartialEq)]
#[reflect(Component, Default, SceneComponent)]
#[require(BodyDrive, BodySense, PhysicalState)]
pub struct Fall {
    /// How conscious the fall is, from 1, fully braced, to 0, which is the limp fall.
    pub strength: f32,
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
    #[reflect(skip)]
    started: bool,
    #[reflect(skip)]
    left_down: Option<f32>,
    #[reflect(skip)]
    right_down: Option<f32>,
    #[reflect(skip)]
    landed: Option<f32>,
}

impl Default for Fall {
    fn default() -> Self {
        Self {
            strength: 1.0,
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
            started: false,
            left_down: None,
            right_down: None,
            landed: None,
        }
    }
}

impl Fall {
    /// Sets how conscious the fall is, from 1, fully braced, to 0, limp.
    pub fn with_strength(mut self, strength: f32) -> Self {
        self.strength = strength;
        self
    }

    /// Sets how an arm gives once its hand is down: over how many seconds, to what share of its strength.
    pub fn with_buckle(mut self, buckle_seconds: f32, buckled: f32) -> Self {
        self.buckle_seconds = buckle_seconds;
        self.buckled = buckled;
        self
    }

    /// Sets how the body relaxes after landing: over how many seconds, to what share of its strength.
    pub fn with_relax(mut self, relax_seconds: f32, resting: f32) -> Self {
        self.relax_seconds = relax_seconds;
        self.resting = resting;
        self
    }

    /// Whether the body has touched down.
    pub fn landed(&self) -> bool {
        self.landed.is_some()
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
    let mut pose = Vec::new();
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
        let resolved = body.resolved();
        let landmarks = resolved.landmarks();
        let settings = &resolved.settings().parts;
        let count = settings.len();
        let animated = body.animated_pose();
        state.ragdoll_pose(*id, &mut pose);
        let (Some(pelvis), Some(chest)) = (landmarks.pelvis, landmarks.chest) else {
            continue;
        };
        if pose.len() != count || animated.len() != count {
            continue;
        }
        let bodies = state.ragdoll_bodies(*id);
        let touching = |part: usize| {
            state
                .contacts_of(bodies[part])
                .iter()
                .any(|contact| contact.other != Some(entity))
        };

        let on_limbs = sense.lean > LYING
            && (0..count).any(|part| !landmarks.feet().any(|foot| foot == part) && touching(part));
        let down = physical.state == BodyState::Down
            || ((touching(chest) || touching(pelvis) || on_limbs)
                && sense.centre.velocity.length() < 0.5);
        if down || fall.landed.is_some() {
            let landed = fall.landed.map_or(0.0, |landed| landed + dt);
            fall.landed = Some(landed);
            if physical.state != BodyState::Down {
                physical.state = BodyState::Down;
            }
            if landed >= fall.relax_seconds {
                character.go_limp();
                ended(&mut cmd);
                continue;
            }
        }
        let relaxed = fall.landed.map_or(1.0, |landed| {
            let through = if fall.relax_seconds > 0.0 {
                (landed / fall.relax_seconds).clamp(0.0, 1.0)
            } else {
                1.0
            };
            1.0 + (fall.resting - 1.0) * through
        });
        let strength = fall.strength.clamp(0.0, 1.0) * relaxed;
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
                Side::Left => &mut fall.left_down,
                _ => &mut fall.right_down,
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
            let aimed = Chain {
                upper: arm.upper,
                lower: arm.lower,
                end: arm.hand,
            }
            .reach_hinged(
                settings,
                &rest,
                &carried,
                root,
                goal,
                outward - travel * 0.3,
            );
            drive.claim(
                FALL,
                arm.upper,
                FALL_PRIORITY,
                1.0,
                PartCommand::held(muscle(given))
                    .with_target((pose[shoulder].rotation.inverse() * aimed.upper).normalize()),
            );
            drive.claim(
                FALL,
                arm.lower,
                FALL_PRIORITY,
                1.0,
                PartCommand::held(muscle(given))
                    .with_target((aimed.upper.inverse() * aimed.lower).normalize()),
            );
            drive.claim(
                FALL,
                arm.hand,
                FALL_PRIORITY,
                1.0,
                PartCommand::held(muscle(given)),
            );
        }

        if let Some(head) = landmarks.head
            && let Some(neck) = settings[head].parent
        {
            let away = Quat::from_axis_angle(
                Vec3::Y.cross(-travel).try_normalize().unwrap_or(across),
                fall.tuck,
            );
            drive.claim(
                FALL,
                head,
                FALL_PRIORITY,
                1.0,
                PartCommand::held(muscle(1.5)).with_target(
                    (pose[neck].rotation.inverse() * away * carried[head]).normalize(),
                ),
            );
        }

        let twist = if backwards { fall.twist } else { 0.0 };
        let bend = Vec3::X * fall.curl + Vec3::Y * twist;
        for aimed in bent_spine(&landmarks.spine, settings, animated, bend) {
            drive.claim(
                FALL,
                aimed.part,
                FALL_PRIORITY,
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
            drive.claim(
                FALL,
                leg.thigh,
                FALL_PRIORITY,
                1.0,
                PartCommand::held(muscle(fall.legs))
                    .with_target((pose[pelvis].rotation.inverse() * thigh).normalize()),
            );
            drive.claim(
                FALL,
                leg.shin,
                FALL_PRIORITY,
                1.0,
                PartCommand::held(muscle(fall.legs))
                    .with_target((thigh.inverse() * shin).normalize()),
            );
            drive.claim(
                FALL,
                leg.foot,
                FALL_PRIORITY,
                1.0,
                PartCommand::held(muscle(fall.legs)),
            );
        }
    }
}
