use concerto_animation::node::NodeOutcome;
use concerto_ecs::{
    CommandQueue, Component, Entity, Query, Res,
    reflect::{ReflectComponent, ReflectSceneComponent},
};
use concerto_foundation::time::Time;
use concerto_name::Name;
use concerto_physics::{physics_state::PhysicsState, ragdoll::RagdollId};
use concerto_reflect::{Reflect, ReflectDefault};

pub use crate::node::balance::Hindrance;
use crate::{
    behaviour::{BehaviourEnded, BodyDrive, BodyState, Outcome, PhysicalState},
    character::{CharacterRagdoll, RagdollBody, RagdollMode},
    generate::BRACED,
    node::balance::{BalanceController, BalanceRun, BalanceSettings, CommandsOut},
    sensing::BodySense,
    view::BodyView,
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
    /// The share of its standing height under which a sunken pelvis means the legs have folded, and the body gives up.
    pub min_height: f32,
    /// The seconds the body must stay balanced and slow before it has succeeded.
    pub settle_seconds: f32,
    /// The speed of the centre of mass under which the body counts as slow, in m/s.
    pub settle_speed: f32,
    #[reflect(skip)]
    started: bool,
    #[reflect(skip)]
    run: BalanceRun,
    #[reflect(skip)]
    hindrance: Hindrance,
}

impl Default for Balance {
    fn default() -> Self {
        let settings = BalanceSettings::default();
        Self {
            profile: BRACED.into(),
            leg_stiffness: settings.leg_stiffness,
            leg_torque: settings.leg_torque,
            upright_stiffness: settings.upright_stiffness,
            upright_damping: settings.upright_damping,
            max_upright_torque: settings.max_upright_torque,
            height_stiffness: settings.height_stiffness,
            height_damping: settings.height_damping,
            max_lift: settings.max_lift,
            centring: settings.centring,
            sway_damping: settings.sway_damping,
            max_centring: settings.max_centring,
            step_margin: settings.step_margin,
            step_beyond: settings.step_beyond,
            max_step: settings.max_step,
            step_height: settings.step_height,
            step_seconds: settings.step_seconds,
            step_rest: settings.step_rest,
            max_steps: settings.max_steps,
            max_lean: settings.max_lean,
            min_height: settings.min_height,
            settle_seconds: settings.settle_seconds,
            settle_speed: settings.settle_speed,
            started: false,
            run: BalanceRun::default(),
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

    /// Hinders balance for the next fixed step, on top of what other behaviours took this step; a behaviour that makes the body stumble calls it every step it runs, before `balance_bodies`.
    pub fn hinder(&mut self, hindrance: Hindrance) {
        let held = &mut self.hindrance;
        held.reach *= hindrance.reach;
        held.pace *= hindrance.pace;
        held.strength *= hindrance.strength;
        held.give = held.give.max(hindrance.give);
        held.left_leg *= hindrance.left_leg;
        held.right_leg *= hindrance.right_leg;
        held.more_steps = held.more_steps.max(hindrance.more_steps);
    }

    fn settings(&self) -> BalanceSettings {
        BalanceSettings {
            leg_stiffness: self.leg_stiffness,
            leg_torque: self.leg_torque,
            upright_stiffness: self.upright_stiffness,
            upright_damping: self.upright_damping,
            max_upright_torque: self.max_upright_torque,
            height_stiffness: self.height_stiffness,
            height_damping: self.height_damping,
            max_lift: self.max_lift,
            centring: self.centring,
            sway_damping: self.sway_damping,
            max_centring: self.max_centring,
            step_margin: self.step_margin,
            step_beyond: self.step_beyond,
            max_step: self.max_step,
            step_height: self.step_height,
            step_seconds: self.step_seconds,
            step_rest: self.step_rest,
            max_steps: self.max_steps,
            max_lean: self.max_lean,
            min_height: self.min_height,
            settle_seconds: self.settle_seconds,
            settle_speed: self.settle_speed,
        }
    }

    /// The steps taken since balance started or last stood still.
    pub fn steps(&self) -> u32 {
        self.run.steps()
    }

    /// Whether a foot is in the air on its way to a step.
    pub fn stepping(&self) -> bool {
        self.run.stepping()
    }
}

/// Each fixed step, in `BehaviourSet`: for every balancing body, pushes the pelvis towards upright and its animated height, holds the legs strong, steps the foot further from the capture point to just past it when the point has left the feet, and ends with success once the body has stood balanced and slow, or by giving up, which makes the character `Falling`.
pub fn balance_bodies(
    mut characters: Query<(
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
    let mut commands = CommandsOut::default();
    for (entity, mut balance, mut character, id, body, sense, mut drive, mut physical) in
        characters.iter_mut()
    {
        let balance = &mut *balance;
        let hindrance = std::mem::take(&mut balance.hindrance);
        if !balance.started {
            character.power(balance.profile);
            balance.started = true;
        }
        let view = BodyView::of(entity, *id, body, Some(sense), None);
        let settings = balance.settings();
        let outcome = BalanceController {
            settings: &settings,
            state: &mut balance.run,
        }
        .step(&view, &state, dt, &hindrance, &mut commands);
        for part in 0..commands.len() {
            if let Some(command) = commands.get(part) {
                drive.claim(BALANCE, part, BALANCE_PRIORITY, 1.0, command);
            }
        }
        let outcome = match outcome {
            Some(NodeOutcome::Failed) => {
                physical.state = BodyState::Falling;
                Outcome::GaveUp
            }
            Some(NodeOutcome::Succeeded) => {
                character.mode = RagdollMode::FollowsAnimation;
                Outcome::Succeeded
            }
            None => continue,
        };
        cmd.trigger(BehaviourEnded {
            character: entity,
            behaviour: BALANCE.into(),
            outcome,
        });
        cmd.remove::<Balance>(entity);
    }
}
