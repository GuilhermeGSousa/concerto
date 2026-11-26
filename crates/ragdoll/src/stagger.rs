use concerto_ecs::{
    CommandQueue, Component, Entity, Query, Res,
    reflect::{ReflectComponent, ReflectSceneComponent},
    signal::On,
};
use concerto_foundation::time::Time;
use concerto_name::Name;
use concerto_physics::{physics_state::PhysicsState, ragdoll::RagdollId};
use concerto_reflect::{Reflect, ReflectDefault};
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::{
    asset::PartStrength,
    balance::{BALANCE, Balance},
    behaviour::{BehaviourEnded, BodyDrive, Outcome, PartCommand, PhysicalState},
    character::RagdollBody,
    ik::bent_spine,
    node::balance::Hindrance,
    resolve::Limbs,
    sensing::BodySense,
    view::BodyView,
};

/// The name stagger claims parts and reports under.
pub const STAGGER: &str = "stagger";
/// The priority of stagger's claims on the spine and the arms.
pub const STAGGER_PRIORITY: i32 = 20;

/// How a stumble plays out: how fast it wears off, what it takes from balance's legs, and how the trunk and arms move.
#[derive(Clone, Debug, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Default)]
pub struct StaggerSettings {
    /// The seconds over which the stumble wears off.
    pub seconds: f32,
    /// The share of a step the foot falls short by at severity 1.
    pub short_steps: f32,
    /// How many times longer a step takes at severity 1.
    pub slow_steps: f32,
    /// The share of their strength the legs lose at severity 1.
    pub weak_legs: f32,
    /// The steps the body may take on top of balance's `max_steps` while it stumbles.
    pub more_steps: u32,
    /// The bend of the whole spine at severity 1, in radians.
    pub bend: f32,
    /// The stiffness of the spine and arm muscles while staggering, in N·m/rad.
    pub stiffness: f32,
    /// The most torque a spine or arm muscle applies, in N·m.
    pub torque: f32,
    /// The trunk's angular speed at which the arms swing their widest, in rad/s.
    pub full_spin: f32,
    /// How fast the arms circle, in rad/s.
    pub arm_speed: f32,
}

impl Default for StaggerSettings {
    fn default() -> Self {
        Self {
            seconds: 1.5,
            short_steps: 0.6,
            slow_steps: 1.8,
            weak_legs: 0.7,
            more_steps: 4,
            bend: 0.7,
            stiffness: 400.0,
            torque: 120.0,
            full_spin: 2.5,
            arm_speed: 9.0,
        }
    }
}

impl StaggerSettings {
    pub fn with_seconds(mut self, seconds: f32) -> Self {
        self.seconds = seconds;
        self
    }

    pub fn with_short_steps(mut self, short_steps: f32) -> Self {
        self.short_steps = short_steps;
        self
    }

    pub fn with_slow_steps(mut self, slow_steps: f32) -> Self {
        self.slow_steps = slow_steps;
        self
    }

    pub fn with_weak_legs(mut self, weak_legs: f32) -> Self {
        self.weak_legs = weak_legs;
        self
    }

    pub fn with_more_steps(mut self, more_steps: u32) -> Self {
        self.more_steps = more_steps;
        self
    }

    pub fn with_bend(mut self, bend: f32) -> Self {
        self.bend = bend;
        self
    }

    pub fn with_stiffness(mut self, stiffness: f32) -> Self {
        self.stiffness = stiffness;
        self
    }

    pub fn with_torque(mut self, torque: f32) -> Self {
        self.torque = torque;
        self
    }

    pub fn with_full_spin(mut self, full_spin: f32) -> Self {
        self.full_spin = full_spin;
        self
    }

    pub fn with_arm_speed(mut self, arm_speed: f32) -> Self {
        self.arm_speed = arm_speed;
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct StaggerRun {
    pub(crate) direction: Vec3,
    pub(crate) severity: f32,
    elapsed: f32,
    swing: f32,
    phase: f32,
}

impl StaggerRun {
    pub(crate) fn towards(direction: Vec3, severity: f32) -> Self {
        Self {
            direction,
            severity,
            ..Self::default()
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct StaggerClaim {
    weight: f32,
    command: PartCommand,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct StaggerOut {
    claims: Vec<Option<StaggerClaim>>,
}

impl StaggerOut {
    fn reset(&mut self, count: usize) {
        self.claims.clear();
        self.claims.resize(count, None);
    }

    fn claim(&mut self, part: usize, weight: f32, command: PartCommand) {
        if let Some(slot) = self.claims.get_mut(part) {
            *slot = Some(StaggerClaim {
                weight: weight.clamp(0.0, 1.0),
                command,
            });
        }
    }

    pub(crate) fn clear(&mut self) {
        self.claims.clear();
    }

    pub(crate) fn len(&self) -> usize {
        self.claims.len()
    }

    pub(crate) fn get(&self, part: usize) -> Option<(f32, PartCommand)> {
        self.claims
            .get(part)
            .copied()
            .flatten()
            .map(|claim| (claim.weight, claim.command))
    }
}

pub(crate) struct StaggerController<'a> {
    pub(crate) settings: &'a StaggerSettings,
    pub(crate) run: &'a mut StaggerRun,
}

/// Makes a balancing body stumble: its steps fall short and late, its legs are weaker and its knees give on each landing, its trunk bends the way it was sent and its arms windmill while the trunk turns. Add it to a character when it is hit or shoved; it brings a `Balance` when the character has none, and ends when balance does, with the same outcome.
#[derive(Component, Reflect, Clone, Debug, PartialEq)]
#[reflect(Component, Default, SceneComponent)]
#[require(Balance, BodyDrive, BodySense, PhysicalState)]
pub struct Stagger {
    /// The way the body was sent, in world space; its horizontal part is used.
    pub direction: Vec3,
    /// How bad the stumble is, from 0, which is balance alone, to 1, which cannot be recovered.
    pub severity: f32,
    /// The seconds over which the stumble wears off.
    pub seconds: f32,
    /// The share of a step the foot falls short by at severity 1.
    pub short_steps: f32,
    /// How many times longer a step takes at severity 1.
    pub slow_steps: f32,
    /// The share of their strength the legs lose at severity 1.
    pub weak_legs: f32,
    /// The steps the body may take on top of balance's `max_steps` while it stumbles.
    pub more_steps: u32,
    /// The bend of the whole spine at severity 1, in radians.
    pub bend: f32,
    /// The stiffness of the spine and arm muscles while staggering, in N·m/rad.
    pub stiffness: f32,
    /// The most torque a spine or arm muscle applies, in N·m.
    pub torque: f32,
    /// The trunk's angular speed at which the arms swing their widest, in rad/s.
    pub full_spin: f32,
    /// How fast the arms circle, in rad/s.
    pub arm_speed: f32,
    #[reflect(skip)]
    run: StaggerRun,
    #[reflect(skip)]
    ended: Option<Outcome>,
}

impl Default for Stagger {
    fn default() -> Self {
        let settings = StaggerSettings::default();
        Self {
            direction: Vec3::Z,
            severity: 0.5,
            seconds: settings.seconds,
            short_steps: settings.short_steps,
            slow_steps: settings.slow_steps,
            weak_legs: settings.weak_legs,
            more_steps: settings.more_steps,
            bend: settings.bend,
            stiffness: settings.stiffness,
            torque: settings.torque,
            full_spin: settings.full_spin,
            arm_speed: settings.arm_speed,
            run: StaggerRun::default(),
            ended: None,
        }
    }
}

impl Stagger {
    /// A stumble towards `direction` of `severity`, from 0 to 1.
    pub fn new(direction: Vec3, severity: f32) -> Self {
        Self {
            direction,
            severity,
            ..Self::default()
        }
    }

    pub fn with_seconds(mut self, seconds: f32) -> Self {
        self.seconds = seconds;
        self
    }

    /// Sets what severity 1 takes from the legs: the share a step falls short by, how many times longer it takes, and the share of strength lost.
    pub fn with_legs(mut self, short_steps: f32, slow_steps: f32, weak_legs: f32) -> Self {
        self.short_steps = short_steps;
        self.slow_steps = slow_steps;
        self.weak_legs = weak_legs;
        self
    }

    pub fn with_bend(mut self, bend: f32) -> Self {
        self.bend = bend;
        self
    }

    /// How far the arms are swung out, from 0 to 1.
    pub fn swing(&self) -> f32 {
        self.run.swing
    }

    fn settings(&self) -> StaggerSettings {
        StaggerSettings {
            seconds: self.seconds,
            short_steps: self.short_steps,
            slow_steps: self.slow_steps,
            weak_legs: self.weak_legs,
            more_steps: self.more_steps,
            bend: self.bend,
            stiffness: self.stiffness,
            torque: self.torque,
            full_spin: self.full_spin,
            arm_speed: self.arm_speed,
        }
    }
}

struct Arm {
    upper: usize,
    lower: usize,
    hand: usize,
}

impl Arm {
    fn of(limbs: &Limbs) -> Option<Self> {
        Some(Self {
            upper: limbs.upper_arm?,
            lower: limbs.lower_arm?,
            hand: limbs.hand?,
        })
    }
}

struct SidedArm {
    arm: Option<Arm>,
    outward: Vec3,
    lag: f32,
}

impl StaggerController<'_> {
    pub(crate) fn step(
        &mut self,
        view: &BodyView,
        physics: &PhysicsState,
        dt: f32,
        out: &mut StaggerOut,
    ) -> Option<Hindrance> {
        let resolved = view.resolved();
        out.reset(resolved.part_count());
        let landmarks = resolved.landmarks();
        let settings = &resolved.settings().parts;
        let animated = view.animated();
        let sense = view.sense()?;
        let mut pose = Vec::new();
        physics.ragdoll_pose(view.ragdoll_id(), &mut pose);
        let (pelvis, chest) = (landmarks.pelvis?, landmarks.chest?);
        if pose.len() != settings.len() || animated.len() != settings.len() {
            return None;
        }
        let tuning = self.settings;
        let run = &mut *self.run;
        run.elapsed += dt;
        let left = if tuning.seconds > 0.0 {
            (1.0 - run.elapsed / tuning.seconds).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let severity = run.severity.clamp(0.0, 1.0);
        let stumble = severity * left;
        let hindrance = Hindrance {
            reach: 1.0 - tuning.short_steps * stumble,
            pace: 1.0 + (tuning.slow_steps - 1.0).max(0.0) * stumble,
            strength: 1.0 - tuning.weak_legs * stumble,
            give: stumble,
            more_steps: if stumble > 0.0 { tuning.more_steps } else { 0 },
            ..Hindrance::default()
        };

        let muscle = PartStrength::Powered {
            stiffness: tuning.stiffness,
            damping: tuning.stiffness * 0.1,
            torque_limit: tuning.torque,
        };
        let turned = pose[pelvis].rotation * animated[pelvis].rotation.inverse();
        let direction = Vec3::new(run.direction.x, 0.0, run.direction.z)
            .try_normalize()
            .unwrap_or(Vec3::Z);
        let tip = Vec3::Y.cross(direction);
        let bend = turned.inverse() * tip * tuning.bend * stumble;
        for aimed in bent_spine(&landmarks.spine, settings, animated, bend) {
            out.claim(
                aimed.part,
                left,
                PartCommand::held(muscle).with_target(aimed.target),
            );
        }

        let spin = Vec3::new(sense.trunk_spin.x, 0.0, sense.trunk_spin.z);
        let wanted =
            (spin.length() / tuning.full_spin.max(1e-3)).clamp(0.4 * stumble, 1.0) * severity;
        run.swing += (wanted - run.swing) * (dt * 8.0).min(1.0);
        let across = (pose[chest].rotation * settings[chest].pose.rotation.inverse() * Vec3::X)
            .try_normalize()
            .unwrap_or(Vec3::X);
        let forward = across.cross(Vec3::Y).try_normalize().unwrap_or(Vec3::Z);
        let sense_of = if spin.dot(across).abs() > 0.3 {
            spin.dot(across).signum()
        } else {
            tip.dot(across).signum()
        };
        run.phase += sense_of * tuning.arm_speed * dt;
        if run.swing < 0.02 {
            return Some(hindrance);
        }
        for sided in [
            SidedArm {
                arm: Arm::of(&landmarks.left),
                outward: across,
                lag: 0.0,
            },
            SidedArm {
                arm: Arm::of(&landmarks.right),
                outward: -across,
                lag: 0.6,
            },
        ] {
            let Some(arm) = sided.arm else {
                continue;
            };
            let Some(shoulder) = settings[arm.upper].parent else {
                continue;
            };
            let angle = run.phase - sense_of * sided.lag;
            let circle = Quat::from_axis_angle(across, angle) * forward;
            let aim = (Vec3::NEG_Y * (1.0 - run.swing)
                + (sided.outward * 0.6 + circle * 0.8) * run.swing)
                .try_normalize()
                .unwrap_or(Vec3::NEG_Y);
            let along = settings[arm.lower].pose.translation - settings[arm.upper].pose.translation;
            let local = settings[arm.upper].pose.rotation.inverse()
                * along.try_normalize().unwrap_or(sided.outward);
            let carried = (turned * animated[arm.upper].rotation).normalize();
            let aimed = (Quat::from_rotation_arc(carried * local, aim) * carried).normalize();
            out.claim(
                arm.upper,
                run.swing,
                PartCommand::held(muscle)
                    .with_target((pose[shoulder].rotation.inverse() * aimed).normalize()),
            );
            for part in [arm.lower, arm.hand] {
                out.claim(part, run.swing, PartCommand::held(muscle));
            }
        }
        Some(hindrance)
    }
}

/// Each fixed step, in `BehaviourSet` before `balance_bodies`: for every staggering body, hinders its balance by what is left of the stumble, bends the spine towards the stagger's direction, and circles both arms in the sense the trunk is turning, as wide as it turns fast.
pub fn stagger_bodies(
    mut characters: Query<(
        Entity,
        &mut Stagger,
        &mut Balance,
        &RagdollId,
        &RagdollBody,
        &BodySense,
        &mut BodyDrive,
    )>,
    state: Res<PhysicsState>,
) {
    let dt = Time::fixed_delta_time().as_secs_f32();
    let mut claims = StaggerOut::default();
    for (entity, mut stagger, mut balance, id, body, sense, mut drive) in characters.iter_mut() {
        let stagger = &mut *stagger;
        let view = BodyView::of(entity, *id, body, Some(sense), None);
        let settings = stagger.settings();
        stagger.run.direction = stagger.direction;
        stagger.run.severity = stagger.severity;
        let hindrance = StaggerController {
            settings: &settings,
            run: &mut stagger.run,
        }
        .step(&view, &state, dt, &mut claims);
        if let Some(hindrance) = hindrance {
            balance.hinder(hindrance);
        }
        for part in 0..claims.len() {
            if let Some((weight, command)) = claims.get(part) {
                drive.claim(STAGGER, part, STAGGER_PRIORITY, weight, command);
            }
        }
    }
}

/// When a staggering character's balance ends, notes its outcome on the stagger.
pub fn note_balance_ended(on: On<BehaviourEnded>, mut staggers: Query<&mut Stagger>) {
    let ended = on.signal();
    if ended.behaviour == Name::from(BALANCE)
        && let Some(mut stagger) = staggers.get_entity_mut(ended.character)
    {
        stagger.ended = Some(ended.outcome);
    }
}

/// Each fixed step, in `BehaviourSet`: ends every stagger whose balance has ended, with the same outcome.
pub fn end_staggers(staggers: Query<(Entity, &Stagger)>, mut cmd: CommandQueue) {
    for (character, stagger) in staggers.iter() {
        let Some(outcome) = stagger.ended else {
            continue;
        };
        cmd.remove::<Stagger>(character);
        cmd.trigger(BehaviourEnded {
            character,
            behaviour: STAGGER.into(),
            outcome,
        });
    }
}
