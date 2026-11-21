use concerto_ecs::{
    CommandQueue, Component, Entity, Query, Res,
    reflect::{ReflectComponent, ReflectSceneComponent},
};
use concerto_foundation::time::Time;
use concerto_name::Name;
use concerto_physics::{
    physics_state::PhysicsState,
    query::QueryFilter,
    ragdoll::{BodyPose, RagdollId},
};
use concerto_reflect::{Reflect, ReflectDefault};
use glam::{Quat, Vec3};

use crate::{
    asset::{PartStrength, Side},
    behaviour::{BehaviourEnded, BodyDrive, BodyState, Outcome, PartCommand, PhysicalState},
    character::{CharacterRagdoll, RagdollBody, RagdollMode},
    generate::BRACED,
    resolve::Limbs,
    sensing::BodySense,
};

/// The name balance claims parts and reports under.
pub const BALANCE: &str = "balance";
/// The priority of balance's claims on the pelvis and the legs.
pub const BALANCE_PRIORITY: i32 = 10;

/// Keeps a body on its own feet: holds it upright while its capture point is inside its feet, steps to catch it when the point leaves, and gives up when it cannot. Add it to a character with a `BodyDrive`, a `BodySense` and a `PhysicalState` to start it; it removes itself when it succeeds or gives up, with a `BehaviourEnded`.
#[derive(Component, Reflect, Clone, Debug, PartialEq)]
#[reflect(Component, Default, SceneComponent)]
pub struct Balance {
    /// The strength profile the body is powered with while it balances; every part should be powered in it.
    pub profile: Name,
    /// The stiffness of the hip, knee and ankle muscles while balancing, in N·m/rad.
    pub leg_stiffness: f32,
    /// The most torque a leg muscle applies, in N·m.
    pub leg_torque: f32,
    /// The assist: a torque on the pelvis towards its animated orientation, in N·m per radian of tilt.
    pub upright_stiffness: f32,
    /// The assist's damping, in N·m per rad/s.
    pub upright_damping: f32,
    /// The most torque the assist applies, in N·m. At zero the body balances on its joints alone, and falls easily.
    pub max_upright_torque: f32,
    /// The assist: a force on the pelvis towards its animated height, in N per metre.
    pub height_stiffness: f32,
    /// The height assist's damping, in N per m/s.
    pub height_damping: f32,
    /// The most force the height assist applies, in N.
    pub max_lift: f32,
    /// The assist: a horizontal force on the pelvis that pushes the capture point back over the feet, in N per metre it is outside them; nothing while it is inside.
    pub centring: f32,
    /// The assist: a horizontal force on the pelvis against the centre of mass's horizontal velocity, in N per m/s, which stills the sway of a standing body.
    pub sway_damping: f32,
    /// The most force the centring and sway assists apply together, in N.
    pub max_centring: f32,
    /// How far outside the feet the capture point must be before the body steps, in metres.
    pub step_margin: f32,
    /// How far past the capture point a step lands, in metres.
    pub step_beyond: f32,
    /// The longest step, as a share of the leg's length.
    pub max_step: f32,
    /// How high the swing foot lifts, as a share of the leg's length.
    pub step_height: f32,
    /// The seconds a step takes.
    pub step_seconds: f32,
    /// The seconds both feet stay down after a step before the next may start.
    pub step_rest: f32,
    /// The steps the body may take without recovering before it gives up.
    pub max_steps: u32,
    /// The lean of the trunk past which the body gives up, in radians.
    pub max_lean: f32,
    /// The seconds the body must stay balanced and slow before it has succeeded.
    pub settle_seconds: f32,
    /// The speed of the centre of mass under which the body counts as slow, in m/s.
    pub settle_speed: f32,
    #[reflect(skip)]
    started: bool,
    #[reflect(skip)]
    step: Option<Step>,
    #[reflect(skip)]
    steps: u32,
    #[reflect(skip)]
    steady: f32,
    #[reflect(skip)]
    since_step: f32,
    #[reflect(skip)]
    hindrance: Hindrance,
}

/// What another behaviour takes from balance for one fixed step, so the body stumbles instead of recovering as well as it can.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hindrance {
    /// The share of the step balance wants that the foot covers, from 0 to 1.
    pub reach: f32,
    /// How many times longer a step takes.
    pub pace: f32,
    /// The share of their strength the legs and the assists keep, from 0 to 1.
    pub strength: f32,
    /// How much of their strength the knees lose for a moment after each landing, from 0 to 1.
    pub give: f32,
}

impl Default for Hindrance {
    fn default() -> Self {
        Self {
            reach: 1.0,
            pace: 1.0,
            strength: 1.0,
            give: 0.0,
        }
    }
}

const GIVE_SECONDS: f32 = 0.25;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Step {
    swing: Side,
    from: Vec3,
    to: Vec3,
    elapsed: f32,
}

impl Default for Balance {
    fn default() -> Self {
        Self {
            profile: BRACED.into(),
            leg_stiffness: 3000.0,
            leg_torque: 600.0,
            upright_stiffness: 1200.0,
            upright_damping: 120.0,
            max_upright_torque: 250.0,
            height_stiffness: 6000.0,
            height_damping: 600.0,
            max_lift: 500.0,
            centring: 2000.0,
            sway_damping: 400.0,
            max_centring: 150.0,
            step_margin: 0.06,
            step_beyond: 0.08,
            max_step: 0.7,
            step_height: 0.12,
            step_seconds: 0.3,
            step_rest: 0.15,
            max_steps: 4,
            max_lean: 0.8,
            settle_seconds: 1.0,
            settle_speed: 0.2,
            started: false,
            step: None,
            steps: 0,
            steady: 0.0,
            since_step: f32::MAX,
            hindrance: Hindrance::default(),
        }
    }
}

impl Balance {
    pub fn with_profile(mut self, profile: impl Into<Name>) -> Self {
        self.profile = profile.into();
        self
    }

    /// Sets the most the assists apply: the upright torque in N·m, the lift in N, and the centring and sway force in N.
    pub fn with_assist(
        mut self,
        max_upright_torque: f32,
        max_lift: f32,
        max_centring: f32,
    ) -> Self {
        self.max_upright_torque = max_upright_torque;
        self.max_lift = max_lift;
        self.max_centring = max_centring;
        self
    }

    pub fn with_max_steps(mut self, max_steps: u32) -> Self {
        self.max_steps = max_steps;
        self
    }

    pub fn with_settle_seconds(mut self, settle_seconds: f32) -> Self {
        self.settle_seconds = settle_seconds;
        self
    }

    /// Hinders balance for the next fixed step; a behaviour that makes the body stumble calls it every step it runs, before `balance_bodies`.
    pub fn hinder(&mut self, hindrance: Hindrance) {
        self.hindrance = hindrance;
    }

    /// The steps taken since balance started or last stood still.
    pub fn steps(&self) -> u32 {
        self.steps
    }

    /// Whether a foot is in the air on its way to a step.
    pub fn stepping(&self) -> bool {
        self.step.is_some()
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

fn smooth(phase: f32) -> f32 {
    let phase = phase.clamp(0.0, 1.0);
    phase * phase * (3.0 - 2.0 * phase)
}

struct Reach {
    thigh: Quat,
    shin: Quat,
}

fn reach(
    leg: &Leg,
    rest: &[BodyPose],
    carried: &[Quat],
    hip: Vec3,
    ankle: Vec3,
    forward: Vec3,
) -> Reach {
    let upper = rest[leg.shin].translation - rest[leg.thigh].translation;
    let lower = rest[leg.foot].translation - rest[leg.shin].translation;
    let upper_length = upper.length().max(1e-3);
    let lower_length = lower.length().max(1e-3);
    let to_ankle = ankle - hip;
    let distance = to_ankle.length().clamp(
        (upper_length - lower_length).abs() + 1e-3,
        (upper_length + lower_length) * 0.999,
    );
    let direction = to_ankle.try_normalize().unwrap_or(Vec3::NEG_Y);
    let cosine = ((upper_length * upper_length + distance * distance
        - lower_length * lower_length)
        / (2.0 * upper_length * distance))
        .clamp(-1.0, 1.0);
    let bend = (forward - direction * forward.dot(direction))
        .try_normalize()
        .unwrap_or_else(|| direction.any_orthonormal_vector());
    let knee = hip
        + direction * (upper_length * cosine)
        + bend * (upper_length * (1.0 - cosine * cosine).max(0.0).sqrt());
    let foot = hip + direction * distance;
    let upper_local = rest[leg.thigh].rotation.inverse() * upper / upper_length;
    let lower_local = rest[leg.shin].rotation.inverse() * lower / lower_length;
    let thigh = (Quat::from_rotation_arc(
        carried[leg.thigh] * upper_local,
        (knee - hip).try_normalize().unwrap_or(direction),
    ) * carried[leg.thigh])
        .normalize();
    let shin_carried = thigh * (carried[leg.thigh].inverse() * carried[leg.shin]);
    let shin = (Quat::from_rotation_arc(
        shin_carried * lower_local,
        (foot - knee).try_normalize().unwrap_or(direction),
    ) * shin_carried)
        .normalize();
    Reach { thigh, shin }
}

/// Each fixed step, in `BehaviourSet`: for every balancing body, pushes the pelvis towards upright and its animated height, holds the legs strong, steps the foot further from the capture point to just past it when the point has left the feet, and ends with success once the body has stood balanced and slow, or by giving up, which makes the character `Falling`.
pub fn balance_bodies(
    characters: Query<(
        Entity,
        &mut Balance,
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
    for (entity, mut balance, mut character, id, body, sense, mut drive, mut physical) in
        characters.iter()
    {
        let balance = &mut *balance;
        let hindrance = std::mem::take(&mut balance.hindrance);
        let kept = hindrance.strength.clamp(0.0, 1.0);
        if !balance.started {
            character.power(balance.profile);
            balance.started = true;
        }
        let resolved = body.resolved();
        let landmarks = resolved.landmarks();
        let (Some(pelvis), Some(left), Some(right)) = (
            landmarks.pelvis,
            Leg::of(&landmarks.left),
            Leg::of(&landmarks.right),
        ) else {
            continue;
        };
        state.ragdoll_pose(*id, &mut pose);
        let animated = body.animated_pose();
        let settings = &resolved.settings().parts;
        if pose.len() != settings.len() || animated.len() != settings.len() {
            continue;
        }
        let rest: Vec<BodyPose> = settings.iter().map(|part| part.pose).collect();
        let bodies = state.ragdoll_bodies(*id);

        let ended = |outcome: Outcome, cmd: &mut CommandQueue| {
            cmd.trigger(BehaviourEnded {
                character: entity,
                behaviour: BALANCE.into(),
                outcome,
            });
            cmd.remove::<Balance>(entity);
        };
        let off = sense.off_balance.length();
        let wants_step = !sense.supporting.is_empty() && off > balance.step_margin;
        let fallen = sense.lean > balance.max_lean
            || (sense.supporting.is_empty() && sense.airborne > 0.3)
            || (wants_step
                && balance.step.is_none()
                && balance.since_step >= balance.step_rest
                && balance.steps >= balance.max_steps);
        if fallen {
            physical.state = BodyState::Falling;
            ended(Outcome::GaveUp, &mut cmd);
            continue;
        }
        if balance.step.is_none()
            && sense.balanced()
            && sense.centre.velocity.length() < balance.settle_speed
        {
            balance.steady += dt;
            if balance.steady >= balance.settle_seconds {
                character.mode = RagdollMode::FollowsAnimation;
                ended(Outcome::Succeeded, &mut cmd);
                continue;
            }
        } else {
            balance.steady = 0.0;
        }

        let tilt = (animated[pelvis].rotation * pose[pelvis].rotation.inverse()).normalize();
        let tilt = if tilt.w < 0.0 { -tilt } else { tilt };
        let torque = (tilt.to_scaled_axis() * balance.upright_stiffness
            - state.angular_velocity(bodies[pelvis]) * balance.upright_damping)
            .clamp_length_max(balance.max_upright_torque * kept);
        let pelvis_velocity = state.linear_velocity(bodies[pelvis]);
        let standing_height = animated[pelvis].translation.y - sense.ground;
        let lift = ((sense.ground + standing_height - pose[pelvis].translation.y)
            * balance.height_stiffness
            - pelvis_velocity.y * balance.height_damping)
            .clamp(-balance.max_lift * kept, balance.max_lift * kept);
        let sway = Vec3::new(sense.centre.velocity.x, 0.0, sense.centre.velocity.z);
        let centring = (sense.off_balance * -balance.centring - sway * balance.sway_damping)
            .clamp_length_max(balance.max_centring * kept);
        let muscle = PartStrength::Powered {
            stiffness: balance.leg_stiffness * kept,
            damping: balance.leg_stiffness * kept * 0.1,
            torque_limit: balance.leg_torque * kept,
        };
        let given = kept
            * (1.0
                - hindrance.give.clamp(0.0, 1.0)
                    * (1.0 - balance.since_step / GIVE_SECONDS).clamp(0.0, 1.0));
        let knee = PartStrength::Powered {
            stiffness: balance.leg_stiffness * given,
            damping: balance.leg_stiffness * given * 0.1,
            torque_limit: balance.leg_torque * given,
        };
        drive.claim(
            BALANCE,
            pelvis,
            BALANCE_PRIORITY,
            1.0,
            PartCommand::held(muscle)
                .with_force(centring + Vec3::Y * lift)
                .with_torque(torque),
        );

        let turned = pose[pelvis].rotation * animated[pelvis].rotation.inverse();
        let carried: Vec<Quat> = animated
            .iter()
            .map(|part| (turned * part.rotation).normalize())
            .collect();
        let hips = rest[left.thigh].translation - rest[right.thigh].translation;
        let across = (hips - Vec3::Y * hips.y).try_normalize().unwrap_or(Vec3::X);
        let forward = pose[pelvis].rotation
            * rest[pelvis].rotation.inverse()
            * across.cross(Vec3::Y).normalize();
        let leg_length = (rest[left.shin].translation - rest[left.thigh].translation).length()
            + (rest[left.foot].translation - rest[left.shin].translation).length();

        balance.since_step += dt;
        if balance.step.is_none() && wants_step && balance.since_step >= balance.step_rest {
            let distance = |leg: &Leg| {
                let foot = pose[leg.foot].translation;
                Vec3::new(
                    foot.x - sense.capture_point.x,
                    0.0,
                    foot.z - sense.capture_point.z,
                )
                .length()
            };
            let grounded = |leg: &Leg| sense.supporting.contains(&leg.foot);
            let sideways = sense.off_balance.dot(across);
            let lengthways = sense.off_balance.dot(forward);
            let swing = match (grounded(&left), grounded(&right)) {
                (true, false) => Side::Right,
                (false, true) => Side::Left,
                _ if sideways.abs() > lengthways.abs() * 0.5 => {
                    if sideways > 0.0 {
                        Side::Left
                    } else {
                        Side::Right
                    }
                }
                _ if distance(&left) >= distance(&right) => Side::Left,
                _ => Side::Right,
            };
            let (swinging, standing) = match swing {
                Side::Left => (&left, &right),
                _ => (&right, &left),
            };
            let outward = sense.off_balance.try_normalize().unwrap_or(forward);
            let stance = pose[standing.foot].translation;
            let aim = sense.capture_point + outward * balance.step_beyond;
            let from_stance = Vec3::new(aim.x - stance.x, 0.0, aim.z - stance.z)
                .clamp_length_max(leg_length * balance.max_step)
                * hindrance.reach.clamp(0.0, 1.0);
            let above = Vec3::new(stance.x, sense.ground + 0.5, stance.z) + from_stance;
            let landing = state.cast_ray(above, Vec3::NEG_Y, QueryFilter::default());
            match landing {
                Some(hit) => {
                    let from = pose[swinging.foot].translation;
                    let clearance = (from.y - sense.ground).max(0.0);
                    balance.step = Some(Step {
                        swing,
                        from,
                        to: hit.point + Vec3::Y * clearance,
                        elapsed: 0.0,
                    });
                }
                None => {
                    physical.state = BodyState::Falling;
                    ended(Outcome::GaveUp, &mut cmd);
                    continue;
                }
            }
        }

        let mut landed = false;
        for sided in [
            Sided {
                leg: &left,
                side: Side::Left,
            },
            Sided {
                leg: &right,
                side: Side::Right,
            },
        ] {
            let leg = sided.leg;
            let side = sided.side;
            let stepping = balance.step.filter(|step| step.swing == side);
            let Some(mut step) = stepping else {
                for part in [leg.thigh, leg.shin, leg.foot] {
                    drive.claim(
                        BALANCE,
                        part,
                        BALANCE_PRIORITY,
                        1.0,
                        PartCommand::held(if part == leg.shin { knee } else { muscle }),
                    );
                }
                continue;
            };
            step.elapsed += dt;
            let phase = if balance.step_seconds > 0.0 {
                step.elapsed / (balance.step_seconds * hindrance.pace.max(1.0))
            } else {
                1.0
            };
            let ankle = step.from.lerp(step.to, smooth(phase))
                + Vec3::Y
                    * (leg_length
                        * balance.step_height
                        * (phase.clamp(0.0, 1.0) * std::f32::consts::PI).sin());
            let aimed = reach(
                leg,
                &rest,
                &carried,
                pose[leg.thigh].translation,
                ankle,
                forward,
            );
            drive.claim(
                BALANCE,
                leg.thigh,
                BALANCE_PRIORITY,
                1.0,
                PartCommand::held(muscle)
                    .with_target((pose[pelvis].rotation.inverse() * aimed.thigh).normalize()),
            );
            drive.claim(
                BALANCE,
                leg.shin,
                BALANCE_PRIORITY,
                1.0,
                PartCommand::held(muscle)
                    .with_target((aimed.thigh.inverse() * aimed.shin).normalize()),
            );
            drive.claim(
                BALANCE,
                leg.foot,
                BALANCE_PRIORITY,
                1.0,
                PartCommand::held(muscle)
                    .with_target((aimed.shin.inverse() * carried[leg.foot]).normalize()),
            );
            let touched = phase > 0.6 && sense.supporting.contains(&leg.foot);
            if phase >= 1.0 || touched {
                landed = true;
            } else {
                balance.step = Some(step);
            }
        }
        if landed {
            balance.step = None;
            balance.steps += 1;
            balance.since_step = 0.0;
        }
        if balance.step.is_none() && balance.steady > 0.3 {
            balance.steps = 0;
        }
    }
}

struct Sided<'a> {
    leg: &'a Leg,
    side: Side,
}
