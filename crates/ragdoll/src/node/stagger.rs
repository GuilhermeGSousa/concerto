use concerto_animation::{
    attribute::PoseAttribute,
    evaluation::{AnimationGraphContext, DriveContext},
    node::{
        AnimationNode, BehaviourReport, InputSlots, NodeOutcome, NodeReport, ReflectAnimationNode,
    },
    pose::{AnimationFrame, EvaluatedFrame, FramePool},
};
use concerto_ecs::Res;
use concerto_name::{Name, name};
use concerto_physics::physics_state::PhysicsState;
use concerto_reflect::{Reflect, ReflectDefault};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    asset::StrengthProfile,
    attributes::{MotorCommand, PhysicsAmount},
    generate::BRACED,
    messages,
    node::balance::{
        BalanceController, BalanceRun, BalanceSettings, CommandsOut, motor_of, wanted,
    },
    stagger::{StaggerController, StaggerOut, StaggerRun, StaggerSettings},
    view::BodyView,
};

/// Makes a body stumble from the moment it receives a `Stagger` message: it balances as a `BalanceNode` does, but its steps fall short and late, its legs are weaker and its knees give on each landing, its trunk bends the way it was sent and its arms windmill. It succeeds when balance settles and fails when balance gives up.
#[derive(Clone, Debug, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(AnimationNode, Default)]
pub struct StaggerNode {
    /// The strength profile the body is powered with while it stumbles.
    pub profile: Name,
    /// How the stumble plays out.
    pub settings: StaggerSettings,
    /// How the body balances while it stumbles.
    pub balance: BalanceSettings,
}

impl Default for StaggerNode {
    fn default() -> Self {
        Self::new(BRACED)
    }
}

impl StaggerNode {
    /// A stagger powering `profile`, with the settings of the old `Stagger` and `Balance`.
    pub fn new(profile: impl Into<Name>) -> Self {
        Self {
            profile: profile.into(),
            settings: StaggerSettings::default(),
            balance: BalanceSettings::default(),
        }
    }

    pub fn with_settings(mut self, settings: StaggerSettings) -> Self {
        self.settings = settings;
        self
    }

    pub fn with_balance(mut self, balance: BalanceSettings) -> Self {
        self.balance = balance;
        self
    }

    fn motor(
        &self,
        state: &StaggerState,
        body: &BodyView,
        profile: Option<&StrengthProfile>,
        part: usize,
    ) -> Option<MotorCommand> {
        let base = wanted(&state.commands, body, profile, part);
        let Some((weight, claimed)) = state.claims.get(part) else {
            return base;
        };
        let claimed = motor_of(claimed, body.animated_target(part));
        match base {
            Some(mut base) => {
                base.blend(1.0 - weight, &claimed, weight);
                Some(base)
            }
            None => (weight >= 0.5).then_some(claimed),
        }
    }
}

/// A [`StaggerNode`]'s state: the stumble it is in, its balance's run and how it ended.
#[derive(Default)]
pub struct StaggerState {
    started: bool,
    run: StaggerRun,
    balance: BalanceRun,
    commands: CommandsOut,
    claims: StaggerOut,
    outcome: Option<NodeOutcome>,
}

impl StaggerState {
    /// The steps taken since the stumble started or the body last stood still.
    pub fn steps(&self) -> u32 {
        self.balance.steps()
    }

    /// Whether a foot is in the air on its way to a step.
    pub fn stepping(&self) -> bool {
        self.balance.stepping()
    }
}

impl AnimationNode for StaggerNode {
    type State = StaggerState;
    type CreateInput = ();
    type UpdateInput = ();
    type EvaluateInput = ();
    type DriveInput = Res<'static, PhysicsState>;

    fn input_slots(&self) -> InputSlots {
        InputSlots::Fixed(vec![name!("pose")])
    }

    fn create_state(&self, _: &(), _: &AnimationGraphContext<'_>) -> StaggerState {
        StaggerState::default()
    }

    fn receive(
        &self,
        state: &mut StaggerState,
        message: &dyn Reflect,
        _: &AnimationGraphContext<'_>,
    ) {
        if let Some(stagger) = message.downcast_ref::<messages::Stagger>() {
            *state = StaggerState {
                started: true,
                run: StaggerRun::towards(stagger.direction, stagger.severity),
                ..StaggerState::default()
            };
        }
    }

    fn update(&self, _: &mut StaggerState, _: f32, _: &(), _: &AnimationGraphContext<'_>) {}

    fn evaluate(
        &self,
        state: &mut StaggerState,
        _: &(),
        context: &AnimationGraphContext<'_>,
        _: &[Uuid],
        inputs: &[EvaluatedFrame],
        _: &mut FramePool,
        output: &mut AnimationFrame,
    ) {
        if let Some(input) = inputs.first() {
            output.copy_from(&input.frame);
        }
        let (Some(body), Some(skeleton), true) = (
            context.extra::<BodyView>(),
            context.skeleton(),
            state.started,
        ) else {
            return;
        };
        let amounts = output
            .attributes_mut()
            .channel_mut::<PhysicsAmount>(skeleton.bone_count());
        let profile = body.profile(&self.profile);
        for (part, bone) in body.resolved().bones().iter().enumerate() {
            let acted = self.motor(state, body, profile, part).is_some();
            if let (true, Some(amount)) = (acted, amounts.get_mut(*bone)) {
                *amount = PhysicsAmount(1.0);
            }
        }
    }

    fn drive(
        &self,
        state: &mut StaggerState,
        physics: &Res<PhysicsState>,
        context: &DriveContext<'_>,
        _: &[Uuid],
        inputs: &[EvaluatedFrame],
        _: &mut FramePool,
        output: &mut AnimationFrame,
    ) {
        if let Some(input) = inputs.first() {
            output.attributes_mut().copy_from(input.frame.attributes());
        }
        let (Some(body), Some(skeleton), true) = (
            context.extra::<BodyView>(),
            context.graph().skeleton(),
            state.started,
        ) else {
            return;
        };
        if state.outcome.is_none() {
            let dt = context.delta_time();
            let hindrance = StaggerController {
                settings: &self.settings,
                run: &mut state.run,
            }
            .step(body, physics, dt, &mut state.claims)
            .unwrap_or_default();
            state.outcome = BalanceController {
                settings: &self.balance,
                state: &mut state.balance,
            }
            .step(body, physics, dt, &hindrance, &mut state.commands);
        } else {
            state.commands.clear();
            state.claims.clear();
        }
        let commands = output
            .attributes_mut()
            .channel_mut::<MotorCommand>(skeleton.bone_count());
        let profile = body.profile(&self.profile);
        for (part, bone) in body.resolved().bones().iter().enumerate() {
            if let (Some(wanted), Some(command)) = (
                self.motor(state, body, profile, part),
                commands.get_mut(*bone),
            ) {
                *command = wanted;
            }
        }
    }

    fn reset(&self, state: &mut StaggerState) {
        *state = StaggerState::default();
    }

    fn is_finished(&self, _: &StaggerState) -> bool {
        false
    }

    fn outcome(&self, state: &StaggerState) -> Option<NodeOutcome> {
        state.outcome
    }

    fn report(&self, state: &StaggerState) -> NodeReport {
        let text = match state.outcome {
            _ if !state.started => format!("waiting for a stagger at {}", self.profile),
            Some(NodeOutcome::Succeeded) => format!("recovered from a stagger at {}", self.profile),
            Some(NodeOutcome::Failed) => format!("fell in a stagger at {}", self.profile),
            None => format!(
                "staggering at {}, {} steps since it last stood still",
                self.profile,
                state.steps()
            ),
        };
        NodeReport {
            behaviour: Some(BehaviourReport {
                outcome: state.outcome,
                text,
            }),
            ..NodeReport::default()
        }
    }
}
