use concerto_animation::{
    attribute::PoseAttribute,
    evaluation::{AnimationGraphContext, DriveContext},
    node::{AnimationNode, EditorAnimationNode, InputSlots, ReflectAnimationNode},
    pose::{AnimationFrame, EvaluatedFrame, FramePool},
};
use concerto_ecs::{Res, system::input::SystemInputData};
use concerto_name::{Name, name};
use concerto_physics::{physics_state::PhysicsState, query::QueryFilter, ragdoll::BodyPose};
use concerto_reflect::{Reflect, ReflectDefault};
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    asset::{PartStrength, Side, StrengthProfile},
    attributes::{MotorCommand, PhysicsAmount},
    behaviour::PartCommand,
    generate::BRACED,
    ik::{Chain, reach},
    messages::{Steadied, Toppled},
    resolve::Limbs,
    sensing::BodySense,
    view::{BodyInput, BodyView},
};

/// How a body keeps on its own feet: the leg muscles, the assists on the pelvis, how it steps, and when it has recovered or given up.
#[derive(Clone, Debug, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Default)]
pub struct BalanceSettings {
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
    /// The assist: a horizontal force on the pelvis that pushes the capture point back over the feet.
    ///
    /// In N per metre the point is outside the feet; nothing while it is
    /// inside.
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
    /// The share of its standing height under which a sunken pelvis means the legs have folded, and the body gives up.
    pub min_height: f32,
    /// The seconds the body must stay balanced and slow before it has succeeded.
    pub settle_seconds: f32,
    /// The speed of the centre of mass under which the body counts as slow, in m/s.
    pub settle_speed: f32,
}

impl Default for BalanceSettings {
    fn default() -> Self {
        Self {
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
            min_height: 0.6,
            settle_seconds: 1.0,
            settle_speed: 0.2,
        }
    }
}

impl BalanceSettings {
    /// Sets `leg_stiffness`.
    pub fn with_leg_stiffness(mut self, leg_stiffness: f32) -> Self {
        self.leg_stiffness = leg_stiffness;
        self
    }

    /// Sets `leg_torque`.
    pub fn with_leg_torque(mut self, leg_torque: f32) -> Self {
        self.leg_torque = leg_torque;
        self
    }

    /// Sets `upright_stiffness`.
    pub fn with_upright_stiffness(mut self, upright_stiffness: f32) -> Self {
        self.upright_stiffness = upright_stiffness;
        self
    }

    /// Sets `upright_damping`.
    pub fn with_upright_damping(mut self, upright_damping: f32) -> Self {
        self.upright_damping = upright_damping;
        self
    }

    /// Sets `max_upright_torque`.
    pub fn with_max_upright_torque(mut self, max_upright_torque: f32) -> Self {
        self.max_upright_torque = max_upright_torque;
        self
    }

    /// Sets `height_stiffness`.
    pub fn with_height_stiffness(mut self, height_stiffness: f32) -> Self {
        self.height_stiffness = height_stiffness;
        self
    }

    /// Sets `height_damping`.
    pub fn with_height_damping(mut self, height_damping: f32) -> Self {
        self.height_damping = height_damping;
        self
    }

    /// Sets `max_lift`.
    pub fn with_max_lift(mut self, max_lift: f32) -> Self {
        self.max_lift = max_lift;
        self
    }

    /// Sets `centring`.
    pub fn with_centring(mut self, centring: f32) -> Self {
        self.centring = centring;
        self
    }

    /// Sets `sway_damping`.
    pub fn with_sway_damping(mut self, sway_damping: f32) -> Self {
        self.sway_damping = sway_damping;
        self
    }

    /// Sets `max_centring`.
    pub fn with_max_centring(mut self, max_centring: f32) -> Self {
        self.max_centring = max_centring;
        self
    }

    /// Sets `step_margin`.
    pub fn with_step_margin(mut self, step_margin: f32) -> Self {
        self.step_margin = step_margin;
        self
    }

    /// Sets `step_beyond`.
    pub fn with_step_beyond(mut self, step_beyond: f32) -> Self {
        self.step_beyond = step_beyond;
        self
    }

    /// Sets `max_step`.
    pub fn with_max_step(mut self, max_step: f32) -> Self {
        self.max_step = max_step;
        self
    }

    /// Sets `step_height`.
    pub fn with_step_height(mut self, step_height: f32) -> Self {
        self.step_height = step_height;
        self
    }

    /// Sets `step_seconds`.
    pub fn with_step_seconds(mut self, step_seconds: f32) -> Self {
        self.step_seconds = step_seconds;
        self
    }

    /// Sets `step_rest`.
    pub fn with_step_rest(mut self, step_rest: f32) -> Self {
        self.step_rest = step_rest;
        self
    }

    /// Sets `max_steps`.
    pub fn with_max_steps(mut self, max_steps: u32) -> Self {
        self.max_steps = max_steps;
        self
    }

    /// Sets `max_lean`.
    pub fn with_max_lean(mut self, max_lean: f32) -> Self {
        self.max_lean = max_lean;
        self
    }

    /// Sets `min_height`.
    pub fn with_min_height(mut self, min_height: f32) -> Self {
        self.min_height = min_height;
        self
    }

    /// Sets `settle_seconds`.
    pub fn with_settle_seconds(mut self, settle_seconds: f32) -> Self {
        self.settle_seconds = settle_seconds;
        self
    }

    /// Sets `settle_speed`.
    pub fn with_settle_speed(mut self, settle_speed: f32) -> Self {
        self.settle_speed = settle_speed;
        self
    }

    fn muscle(&self, kept: f32) -> PartStrength {
        PartStrength::Powered {
            stiffness: self.leg_stiffness * kept,
            damping: self.leg_stiffness * kept * 0.1,
            torque_limit: self.leg_torque * kept,
        }
    }
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
    /// The share of its strength the left leg keeps, from 0 to 1.
    ///
    /// A leg under a half is lame: its knee sags, and it is the one that
    /// steps, never the one stood on.
    pub left_leg: f32,
    /// The share of its strength the right leg keeps.
    pub right_leg: f32,
    /// Steps the body may take on top of `max_steps` before it gives up: a stumble is many short steps.
    pub more_steps: u32,
}

impl Default for Hindrance {
    fn default() -> Self {
        Self {
            reach: 1.0,
            pace: 1.0,
            strength: 1.0,
            give: 0.0,
            left_leg: 1.0,
            right_leg: 1.0,
            more_steps: 0,
        }
    }
}

const GIVE_SECONDS: f32 = 0.25;
const LAME_CROUCH: f32 = 0.4;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Step {
    swing: Side,
    from: Vec3,
    to: Vec3,
    elapsed: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BalanceRun {
    step: Option<Step>,
    steps: u32,
    steady: f32,
    since_step: f32,
}

impl Default for BalanceRun {
    fn default() -> Self {
        Self {
            step: None,
            steps: 0,
            steady: 0.0,
            since_step: f32::MAX,
        }
    }
}

impl BalanceRun {
    pub(crate) fn steps(&self) -> u32 {
        self.steps
    }

    pub(crate) fn stepping(&self) -> bool {
        self.step.is_some()
    }
}

impl PartCommand {
    pub(crate) fn motor(self, animated: Quat) -> MotorCommand {
        MotorCommand {
            target: self.target.unwrap_or(animated),
            force: self.force,
            torque: self.torque,
            ..MotorCommand::holding(self.strength, 1.0, animated)
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct CommandsOut {
    parts: Vec<Option<PartCommand>>,
}

impl CommandsOut {
    fn reset(&mut self, count: usize) {
        self.parts.clear();
        self.parts.resize(count, None);
    }

    fn set(&mut self, part: usize, command: PartCommand) {
        if let Some(slot) = self.parts.get_mut(part) {
            *slot = Some(command);
        }
    }

    pub(crate) fn clear(&mut self) {
        self.parts.clear();
    }

    pub(crate) fn get(&self, part: usize) -> Option<PartCommand> {
        self.parts.get(part).copied().flatten()
    }

    pub(crate) fn wanted(
        &self,
        body: BodyView<'_>,
        profile: Option<&StrengthProfile>,
        part: usize,
    ) -> Option<MotorCommand> {
        if let Some(ordered) = self.get(part) {
            return Some(ordered.motor(body.animated_target(part)));
        }
        profile?.command_for(body, part)
    }

    pub(crate) fn layered(
        &self,
        claim: Option<OrderedClaim>,
        body: BodyView<'_>,
        profile: Option<&StrengthProfile>,
        part: usize,
    ) -> Option<MotorCommand> {
        let base = self.wanted(body, profile, part);
        match claim {
            Some(claim) => claim.layer_onto(base, body.animated_target(part)),
            None => base,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct OrderedClaim {
    pub(crate) weight: f32,
    pub(crate) command: PartCommand,
}

impl OrderedClaim {
    pub(crate) fn layer_onto(
        self,
        base: Option<MotorCommand>,
        animated: Quat,
    ) -> Option<MotorCommand> {
        let weight = self.weight;
        let claimed = self.command.motor(animated);
        match base {
            Some(mut base) => {
                let force = base.force;
                let torque = base.torque;
                base.blend(1.0 - weight, &claimed, weight);
                base.force = force + claimed.force * weight;
                base.torque = torque + claimed.torque * weight;
                Some(base)
            }
            None => (weight >= 0.5).then_some(claimed),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ClaimsOut {
    claims: Vec<Option<OrderedClaim>>,
}

impl ClaimsOut {
    pub(crate) fn reset(&mut self, count: usize) {
        self.claims.clear();
        self.claims.resize(count, None);
    }

    pub(crate) fn claim(&mut self, part: usize, weight: f32, command: PartCommand) {
        if let Some(slot) = self.claims.get_mut(part) {
            *slot = Some(OrderedClaim {
                weight: weight.clamp(0.0, 1.0),
                command,
            });
        }
    }

    pub(crate) fn clear(&mut self) {
        self.claims.clear();
    }

    pub(crate) fn get(&self, part: usize) -> Option<OrderedClaim> {
        self.claims.get(part).copied().flatten()
    }
}

pub(crate) struct BalanceController<'a> {
    pub(crate) settings: &'a BalanceSettings,
    pub(crate) state: &'a mut BalanceRun,
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

struct Sided<'a> {
    leg: &'a Leg,
    side: Side,
    kept: f32,
    share: f32,
}

impl Sided<'_> {
    fn hold(
        &self,
        stance: &Stance,
        muscle: PartStrength,
        knee: PartStrength,
        out: &mut CommandsOut,
    ) {
        let leg = self.leg;
        let carried = stance.carried;
        let crouch = LAME_CROUCH * (1.0 - self.share / 0.5).clamp(0.0, 1.0);
        if crouch > 0.0 {
            let thigh =
                (Quat::from_axis_angle(stance.sideways, -crouch) * carried[leg.thigh]).normalize();
            let shin =
                (Quat::from_axis_angle(stance.sideways, crouch) * carried[leg.shin]).normalize();
            out.set(
                leg.thigh,
                PartCommand::held(muscle).with_target(
                    (stance.pose[stance.pelvis].rotation.inverse() * thigh).normalize(),
                ),
            );
            out.set(
                leg.shin,
                PartCommand::held(knee).with_target((thigh.inverse() * shin).normalize()),
            );
            out.set(
                leg.foot,
                PartCommand::held(muscle)
                    .with_target((shin.inverse() * carried[leg.foot]).normalize()),
            );
            return;
        }
        for part in [leg.thigh, leg.shin, leg.foot] {
            out.set(
                part,
                PartCommand::held(if part == leg.shin { knee } else { muscle }),
            );
        }
    }
}

struct Stance<'a> {
    pose: &'a [BodyPose],
    rest: &'a [BodyPose],
    carried: &'a [Quat],
    pelvis: usize,
    across: Vec3,
    forward: Vec3,
    sideways: Vec3,
    leg_length: f32,
    given: f32,
    pace: f32,
    dt: f32,
}

impl Stance<'_> {
    fn swing_side(&self, hindrance: &Hindrance, sense: &BodySense, footing: &Footing) -> Side {
        let left = &footing.left;
        let right = &footing.right;
        let distance = |leg: &Leg| {
            let foot = self.pose[leg.foot].translation;
            Vec3::new(
                foot.x - sense.capture_point.x,
                0.0,
                foot.z - sense.capture_point.z,
            )
            .length()
        };
        let grounded = |leg: &Leg| sense.supporting.contains(&leg.foot);
        let sideways = sense.off_balance.dot(self.across);
        let lengthways = sense.off_balance.dot(self.forward);
        let lame = |share: f32| share < 0.5;
        match (grounded(left), grounded(right)) {
            _ if lame(hindrance.left_leg) && !lame(hindrance.right_leg) => Side::Left,
            _ if lame(hindrance.right_leg) && !lame(hindrance.left_leg) => Side::Right,
            (true, false) => Side::Right,
            (false, true) => Side::Left,
            _ if sideways.abs() > lengthways.abs() * 0.5 => {
                if sideways > 0.0 {
                    Side::Left
                } else {
                    Side::Right
                }
            }
            _ if distance(left) >= distance(right) => Side::Left,
            _ => Side::Right,
        }
    }
}

struct Footing<'a> {
    sense: &'a BodySense,
    pose: &'a [BodyPose],
    animated: &'a [BodyPose],
    pelvis: usize,
    left: Leg,
    right: Leg,
}

impl<'a> Footing<'a> {
    fn of(view: BodyView<'a>) -> Option<Self> {
        let landmarks = view.resolved().landmarks();
        let count = view.resolved().settings().parts.len();
        let footing = Self {
            sense: view.sense()?,
            pose: view.pose(),
            animated: view.animated(),
            pelvis: landmarks.pelvis?,
            left: Leg::of(&landmarks.left)?,
            right: Leg::of(&landmarks.right)?,
        };
        (footing.pose.len() == count && footing.animated.len() == count).then_some(footing)
    }
}

fn smooth(phase: f32) -> f32 {
    let phase = phase.clamp(0.0, 1.0);
    phase * phase * (3.0 - 2.0 * phase)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BalanceEnd {
    Steadied,
    Toppled,
}

impl BalanceEnd {
    pub(crate) fn raise(self, context: &mut DriveContext<'_>) {
        match self {
            Self::Steadied => context.raise(Steadied),
            Self::Toppled => context.raise(Toppled),
        }
    }
}

impl BalanceController<'_> {
    pub(crate) fn step(
        &mut self,
        view: BodyView<'_>,
        physics: &PhysicsState,
        dt: f32,
        hindrance: &Hindrance,
        out: &mut CommandsOut,
    ) -> Option<BalanceEnd> {
        let resolved = view.resolved();
        out.reset(resolved.part_count());
        let footing = Footing::of(view)?;
        let bodies = physics.ragdoll_bodies(view.ragdoll_id());
        let pelvis_body = *bodies.get(footing.pelvis)?;
        let sense = footing.sense;
        let settings = self.settings;
        let wants_step =
            !sense.supporting.is_empty() && sense.off_balance.length() > settings.step_margin;
        let standing_height = footing.animated[footing.pelvis].translation.y - sense.ground;
        if self.gives_up(&footing, hindrance, wants_step, standing_height) {
            return Some(BalanceEnd::Toppled);
        }
        if self.settles(sense, dt) {
            return Some(BalanceEnd::Steadied);
        }

        let kept = hindrance.strength.clamp(0.0, 1.0)
            * hindrance.left_leg.max(hindrance.right_leg).clamp(0.0, 1.0);
        let pose = footing.pose;
        let animated = footing.animated;
        let pelvis = footing.pelvis;
        let tilt = (animated[pelvis].rotation * pose[pelvis].rotation.inverse()).normalize();
        let tilt = if tilt.w < 0.0 { -tilt } else { tilt };
        let torque = (tilt.to_scaled_axis() * settings.upright_stiffness
            - physics.angular_velocity(pelvis_body) * settings.upright_damping)
            .clamp_length_max(settings.max_upright_torque * kept);
        let pelvis_velocity = physics.linear_velocity(pelvis_body);
        let lift = ((sense.ground + standing_height - pose[pelvis].translation.y)
            * settings.height_stiffness
            - pelvis_velocity.y * settings.height_damping)
            .clamp(-settings.max_lift * kept, settings.max_lift * kept);
        let sway = Vec3::new(sense.centre.velocity.x, 0.0, sense.centre.velocity.z);
        let centring = (sense.off_balance * -settings.centring - sway * settings.sway_damping)
            .clamp_length_max(settings.max_centring * kept);
        out.set(
            pelvis,
            PartCommand::held(settings.muscle(kept))
                .with_force(centring + Vec3::Y * lift)
                .with_torque(torque),
        );

        let rest: Vec<BodyPose> = resolved
            .settings()
            .parts
            .iter()
            .map(|part| part.pose)
            .collect();
        let turned = pose[pelvis].rotation * animated[pelvis].rotation.inverse();
        let carried: Vec<Quat> = animated
            .iter()
            .map(|part| (turned * part.rotation).normalize())
            .collect();
        let stance = self.stance(&footing, &rest, &carried, hindrance, dt);
        self.state.since_step += dt;
        if self.state.step.is_none() && wants_step && self.state.since_step >= settings.step_rest {
            match self.plan_step(hindrance, sense, &stance, &footing, physics) {
                Some(step) => self.state.step = Some(step),
                None => return Some(BalanceEnd::Toppled),
            }
        }
        self.drive_legs(&footing, &stance, hindrance, out);
        None
    }

    fn gives_up(
        &self,
        footing: &Footing,
        hindrance: &Hindrance,
        wants_step: bool,
        standing_height: f32,
    ) -> bool {
        let settings = self.settings;
        let sense = footing.sense;
        let sunk = footing.pose[footing.pelvis].translation.y - sense.ground
            < standing_height * settings.min_height;
        sense.lean > settings.max_lean
            || sunk
            || (sense.supporting.is_empty() && sense.airborne > 0.3)
            || (wants_step
                && self.state.step.is_none()
                && self.state.since_step >= settings.step_rest
                && self.state.steps >= settings.max_steps + hindrance.more_steps)
    }

    fn settles(&mut self, sense: &BodySense, dt: f32) -> bool {
        if self.state.step.is_none()
            && sense.balanced()
            && sense.centre.velocity.length() < self.settings.settle_speed
        {
            self.state.steady += dt;
            self.state.steady >= self.settings.settle_seconds
        } else {
            self.state.steady = 0.0;
            false
        }
    }

    fn stance<'a>(
        &self,
        footing: &'a Footing,
        rest: &'a [BodyPose],
        carried: &'a [Quat],
        hindrance: &Hindrance,
        dt: f32,
    ) -> Stance<'a> {
        let pose = footing.pose;
        let pelvis = footing.pelvis;
        let left = &footing.left;
        let hips = rest[left.thigh].translation - rest[footing.right.thigh].translation;
        let across = (hips - Vec3::Y * hips.y).try_normalize().unwrap_or(Vec3::X);
        let facing = pose[pelvis].rotation * rest[pelvis].rotation.inverse();
        let given = 1.0
            - hindrance.give.clamp(0.0, 1.0)
                * (1.0 - self.state.since_step / GIVE_SECONDS).clamp(0.0, 1.0);
        Stance {
            pose,
            rest,
            carried,
            pelvis,
            across,
            forward: facing * across.cross(Vec3::Y).normalize(),
            sideways: facing * across,
            leg_length: (rest[left.shin].translation - rest[left.thigh].translation).length()
                + (rest[left.foot].translation - rest[left.shin].translation).length(),
            given,
            pace: hindrance.pace,
            dt,
        }
    }

    fn drive_legs(
        &mut self,
        footing: &Footing,
        stance: &Stance,
        hindrance: &Hindrance,
        out: &mut CommandsOut,
    ) {
        let whole = hindrance.strength.clamp(0.0, 1.0);
        let mut landed = false;
        for sided in [
            Sided {
                leg: &footing.left,
                side: Side::Left,
                kept: whole * hindrance.left_leg.clamp(0.0, 1.0),
                share: hindrance.left_leg.clamp(0.0, 1.0),
            },
            Sided {
                leg: &footing.right,
                side: Side::Right,
                kept: whole * hindrance.right_leg.clamp(0.0, 1.0),
                share: hindrance.right_leg.clamp(0.0, 1.0),
            },
        ] {
            if self.drive_leg(&sided, stance, footing.sense, out) {
                landed = true;
            }
        }
        if landed {
            self.state.step = None;
            self.state.steps += 1;
            self.state.since_step = 0.0;
        }
        if self.state.step.is_none() && self.state.steady > 0.3 {
            self.state.steps = 0;
        }
    }

    fn plan_step(
        &self,
        hindrance: &Hindrance,
        sense: &BodySense,
        stance: &Stance,
        footing: &Footing,
        physics: &PhysicsState,
    ) -> Option<Step> {
        let settings = self.settings;
        let pose = stance.pose;
        let swing = stance.swing_side(hindrance, sense, footing);
        let (swinging, standing) = match swing {
            Side::Left => (&footing.left, &footing.right),
            _ => (&footing.right, &footing.left),
        };
        let outward = sense.off_balance.try_normalize().unwrap_or(stance.forward);
        let stand = pose[standing.foot].translation;
        let aim = sense.capture_point + outward * settings.step_beyond;
        let from_stance = Vec3::new(aim.x - stand.x, 0.0, aim.z - stand.z)
            .clamp_length_max(stance.leg_length * settings.max_step)
            * hindrance.reach.clamp(0.0, 1.0);
        let above = Vec3::new(stand.x, sense.ground + 0.5, stand.z) + from_stance;
        let hit = physics.cast_ray(above, Vec3::NEG_Y, QueryFilter::default())?;
        let from = pose[swinging.foot].translation;
        let clearance = (from.y - sense.ground).max(0.0);
        Some(Step {
            swing,
            from,
            to: hit.point + Vec3::Y * clearance,
            elapsed: 0.0,
        })
    }

    fn drive_leg(
        &mut self,
        sided: &Sided,
        stance: &Stance,
        sense: &BodySense,
        out: &mut CommandsOut,
    ) -> bool {
        let settings = self.settings;
        let leg = sided.leg;
        let pose = stance.pose;
        let carried = stance.carried;
        let muscle = settings.muscle(sided.kept);
        let knee = settings.muscle(sided.kept * stance.given);
        let stepping = self.state.step.filter(|step| step.swing == sided.side);
        let Some(mut step) = stepping else {
            sided.hold(stance, muscle, knee, out);
            return false;
        };
        step.elapsed += stance.dt;
        let phase = if settings.step_seconds > 0.0 {
            step.elapsed / (settings.step_seconds * stance.pace.max(1.0))
        } else {
            1.0
        };
        let ankle = step.from.lerp(step.to, smooth(phase))
            + Vec3::Y
                * (stance.leg_length
                    * settings.step_height
                    * (phase.clamp(0.0, 1.0) * std::f32::consts::PI).sin());
        let aimed = reach(
            &Chain {
                upper: leg.thigh,
                lower: leg.shin,
                end: leg.foot,
            },
            stance.rest,
            carried,
            pose[leg.thigh].translation,
            ankle,
            stance.forward,
        );
        out.set(
            leg.thigh,
            PartCommand::held(muscle)
                .with_target((pose[stance.pelvis].rotation.inverse() * aimed.upper).normalize()),
        );
        out.set(
            leg.shin,
            PartCommand::held(muscle)
                .with_target((aimed.upper.inverse() * aimed.lower).normalize()),
        );
        out.set(
            leg.foot,
            PartCommand::held(muscle)
                .with_target((aimed.lower.inverse() * carried[leg.foot]).normalize()),
        );
        let touched = phase > 0.6 && sense.supporting.contains(&leg.foot);
        if phase >= 1.0 || touched {
            true
        } else {
            self.state.step = Some(step);
            false
        }
    }
}

/// Keeps a body on its own feet from its animation graph, and says when it has steadied or toppled.
///
/// In its drive, each physics step, it holds every part with `profile`, the
/// legs with balance's muscles, pushes the pelvis upright, to its animated
/// height and back over the feet, and takes a step to catch the body when its
/// capture point leaves its feet. It writes a [`PhysicsAmount`] of 1 and a
/// [`MotorCommand`] on every bone whose part it holds; its evaluate passes the
/// pose through.
///
/// It raises [`Steadied`] from its drive once the body has stood balanced and
/// slow for `settle_seconds`, and [`Toppled`] when it gives up; a state machine
/// moves on with `OnMessage` of either. After either it holds the body with
/// `profile` alone. The character needs a [`BodySense`]; without one the node
/// holds the body with `profile` and never steps or ends.
#[derive(Clone, Debug, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(AnimationNode, Default)]
pub struct BalanceNode {
    /// The strength profile every part is held with; the leg muscles and the assists on the pelvis override it.
    pub profile: Name,
    /// How the body keeps its feet.
    pub settings: BalanceSettings,
}

impl Default for BalanceNode {
    fn default() -> Self {
        Self::new(BRACED)
    }
}

impl BalanceNode {
    /// Balances a body held with `profile`, with the default settings.
    pub fn new(profile: impl Into<Name>) -> Self {
        Self {
            profile: profile.into(),
            settings: BalanceSettings::default(),
        }
    }

    /// Sets `settings`.
    pub fn with_settings(mut self, settings: BalanceSettings) -> Self {
        self.settings = settings;
        self
    }
}

/// A [`BalanceNode`]'s state: the step it is taking, the steps since it last stood still, and how it ended.
#[derive(Default)]
pub struct BalanceState {
    run: BalanceRun,
    commands: CommandsOut,
    ended: Option<BalanceEnd>,
}

impl BalanceState {
    /// The steps taken since balance started or last stood still.
    pub fn steps(&self) -> u32 {
        self.run.steps()
    }

    /// Whether a foot is in the air on its way to a step.
    pub fn stepping(&self) -> bool {
        self.run.stepping()
    }
}

impl AnimationNode for BalanceNode {
    type State = BalanceState;
    type CreateInput = ();
    type UpdateInput = ();
    type EvaluateInput = ();
    type DriveInput = (BodyInput, Res<'static, PhysicsState>);

    fn create_state(&self, _: &(), _: &AnimationGraphContext<'_>) -> BalanceState {
        BalanceState::default()
    }

    fn update(&self, _: &mut BalanceState, _: f32, _: &(), _: &mut AnimationGraphContext<'_>) {}

    fn evaluate(
        &self,
        _: &mut BalanceState,
        _: &SystemInputData<'_, '_, ()>,
        _: &AnimationGraphContext<'_>,
        _: &[Uuid],
        inputs: &[EvaluatedFrame],
        _: &mut FramePool,
        output: &mut AnimationFrame,
    ) {
        if let Some(input) = inputs.first() {
            output.copy_from(&input.frame);
        }
    }

    fn drive(
        &self,
        state: &mut BalanceState,
        input: &SystemInputData<'_, '_, Self::DriveInput>,
        context: &mut DriveContext<'_>,
        _: &[Uuid],
        inputs: &[EvaluatedFrame],
        _: &mut FramePool,
        output: &mut AnimationFrame,
    ) {
        let (bodies, physics) = input;
        if let Some(input) = inputs.first() {
            output.attributes_mut().copy_from(input.frame.attributes());
        }
        let (Some(body), Some(skeleton)) =
            (BodyView::read(context.entity(), bodies), context.skeleton())
        else {
            return;
        };
        if state.ended.is_none() {
            let ended = BalanceController {
                settings: &self.settings,
                state: &mut state.run,
            }
            .step(
                body,
                physics,
                context.delta_time(),
                &Hindrance::default(),
                &mut state.commands,
            );
            if let Some(end) = ended {
                state.ended = Some(end);
                end.raise(context);
            }
        } else {
            state.commands.clear();
        }
        let profile = body.profile(&self.profile);
        let bone_count = skeleton.bone_count();
        let attributes = output.attributes_mut();
        let commands = attributes.channel_mut::<MotorCommand>(bone_count);
        for (part, bone) in body.resolved().bones().iter().enumerate() {
            if let (Some(wanted), Some(command)) = (
                state.commands.wanted(body, profile, part),
                commands.get_mut(*bone),
            ) {
                *command = wanted;
            }
        }
        let amounts = attributes.channel_mut::<PhysicsAmount>(bone_count);
        for (part, bone) in body.resolved().bones().iter().enumerate() {
            let held = state.commands.wanted(body, profile, part).is_some();
            if let (true, Some(amount)) = (held, amounts.get_mut(*bone)) {
                *amount = PhysicsAmount(1.0);
            }
        }
    }

    fn reset(&self, state: &mut BalanceState) {
        *state = BalanceState::default();
    }
}

impl EditorAnimationNode for BalanceNode {
    fn input_slots(&self) -> InputSlots {
        InputSlots::Fixed(vec![name!("pose")])
    }

    fn describe(&self, state: &BalanceState) -> Option<String> {
        let text = match state.ended {
            Some(BalanceEnd::Steadied) => format!("stood still at {}", self.profile),
            Some(BalanceEnd::Toppled) => format!("gave up at {}", self.profile),
            None if state.stepping() => format!(
                "stepping at {}, step {} since it last stood still",
                self.profile,
                state.steps() + 1
            ),
            None => format!(
                "balancing at {}, {} steps since it last stood still",
                self.profile,
                state.steps()
            ),
        };
        Some(text)
    }
}
