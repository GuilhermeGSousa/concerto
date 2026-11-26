use concerto_physics::physics_state::PhysicsState;
use concerto_reflect::{Reflect, ReflectDefault};
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::{
    asset::PartStrength,
    ik::bent_spine,
    node::balance::{ClaimsOut, Hindrance, PartCommand},
    resolve::Limbs,
    view::BodyView,
};

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

pub(crate) struct StaggerController<'a> {
    pub(crate) settings: &'a StaggerSettings,
    pub(crate) run: &'a mut StaggerRun,
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
        view: BodyView<'_>,
        physics: &PhysicsState,
        dt: f32,
        out: &mut ClaimsOut,
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
