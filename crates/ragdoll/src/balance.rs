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
    node::balance::{BalanceController, BalanceEnd, BalanceRun, BalanceSettings, CommandsOut},
    sensing::BodySense,
    view::BodyView,
};

/// The name balance claims parts and reports under.
pub const BALANCE: &str = "balance";
/// The priority of balance's claims on the pelvis and the legs.
pub const BALANCE_PRIORITY: i32 = 10;

/// Keeps a body on its own feet.
///
/// It holds the body upright while its capture point is inside its feet,
/// steps to catch it when the point leaves, and gives up when it cannot.
///
/// Add it to a character with a `BodyDrive`, a `BodySense` and a
/// `PhysicalState` to start it. It removes itself when it succeeds or gives
/// up, with a `BehaviourEnded`.
#[derive(Component, Reflect, Clone, Debug, PartialEq)]
#[reflect(Component, Default, SceneComponent)]
pub struct Balance {
    /// The strength profile the body is powered with while it balances; every part should be powered in it.
    pub profile: Name,
    /// How the body keeps its feet.
    pub settings: BalanceSettings,
    #[reflect(skip)]
    started: bool,
    #[reflect(skip)]
    run: BalanceRun,
    #[reflect(skip)]
    hindrance: Hindrance,
}

impl Default for Balance {
    fn default() -> Self {
        Self {
            profile: BRACED.into(),
            settings: BalanceSettings::default(),
            started: false,
            run: BalanceRun::default(),
            hindrance: Hindrance::default(),
        }
    }
}

impl Balance {
    /// Sets the strength profile the body is powered with while it balances.
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
        self.settings.max_upright_torque = max_upright_torque;
        self.settings.max_lift = max_lift;
        self.settings.max_centring = max_centring;
        self
    }

    /// Sets how many steps the body may take without recovering before it gives up.
    pub fn with_max_steps(mut self, max_steps: u32) -> Self {
        self.settings.max_steps = max_steps;
        self
    }

    /// Sets the seconds the body must stay balanced and slow before it has succeeded.
    pub fn with_settle_seconds(mut self, settle_seconds: f32) -> Self {
        self.settings.settle_seconds = settle_seconds;
        self
    }

    /// Hinders balance for the next fixed step, on top of what other behaviours took this step.
    ///
    /// A behaviour that makes the body stumble calls it every step it runs,
    /// before [`balance_bodies`].
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

    /// The steps taken since balance started or last stood still.
    pub fn steps(&self) -> u32 {
        self.run.steps()
    }

    /// Whether a foot is in the air on its way to a step.
    pub fn stepping(&self) -> bool {
        self.run.stepping()
    }
}

/// Balances every balancing body, each fixed step in `BehaviourSet`.
///
/// It pushes the pelvis towards upright and its animated height and holds
/// the legs strong. When the capture point has left the feet, it steps the
/// foot further from the point to just past it. Balance ends with success
/// once the body has stood balanced and slow, or by giving up, which makes
/// the character `Falling`.
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
        let view = BodyView::new(entity, *id, body, Some(sense), None);
        let ended = BalanceController {
            settings: &balance.settings,
            state: &mut balance.run,
        }
        .step(view, &state, dt, &hindrance, &mut commands);
        let outcome = match ended {
            None => {
                for part in 0..body.resolved().part_count() {
                    if let Some(command) = commands.get(part) {
                        drive.claim(BALANCE, part, BALANCE_PRIORITY, 1.0, command);
                    }
                }
                continue;
            }
            Some(BalanceEnd::Steadied) => {
                character.mode = RagdollMode::FollowsAnimation;
                Outcome::Succeeded
            }
            Some(BalanceEnd::Toppled) => {
                physical.state = BodyState::Falling;
                Outcome::GaveUp
            }
        };
        cmd.trigger(BehaviourEnded {
            character: entity,
            behaviour: BALANCE.into(),
            outcome,
        });
        cmd.remove::<Balance>(entity);
    }
}
