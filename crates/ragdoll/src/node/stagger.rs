use concerto_animation::{
    evaluation::{AnimationGraphContext, DriveContext},
    node::{AnimationNode, EditorAnimationNode, InputSlots, ReflectAnimationNode},
    pose::{AnimationFrame, EvaluatedFrame, FramePool},
};
use concerto_ecs::{Res, system::input::SystemInputData};
use concerto_name::{Name, name};
use concerto_physics::physics_state::PhysicsState;
use concerto_reflect::{Reflect, ReflectDefault};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    attributes::{MotorCommand, PhysicsAmount},
    generate::BRACED,
    messages,
    node::balance::{
        BalanceController, BalanceEnd, BalanceRun, BalanceSettings, ClaimsOut, CommandsOut,
    },
    stagger::{StaggerController, StaggerRun, StaggerSettings},
    view::{BodyInput, BodyView},
};

/// Makes a body stumble from the moment it receives a [`Stagger`](messages::Stagger) message: it balances as a [`BalanceNode`](crate::node::BalanceNode) does, but its steps fall short and late, its legs are weaker and its knees give on each landing, its trunk bends the way it was sent and its arms windmill.
///
/// Until the first `Stagger` it only passes its input through. From then on,
/// in its drive, it writes a [`PhysicsAmount`] of 1 and a [`MotorCommand`] on
/// every bone whose part it holds with `profile`, balance or the stumble; its
/// evaluate passes the pose through. A further `Stagger` starts the stumble
/// again.
///
/// It raises [`Steadied`](messages::Steadied) from its drive once balance
/// settles and [`Toppled`](messages::Toppled) when balance gives up; after
/// either it holds the body with `profile` alone. The character needs a
/// [`BodySense`](crate::sensing::BodySense); without one the node holds the
/// body with `profile` and never stumbles or ends.
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
    /// A stagger powering `profile`, with the default settings.
    pub fn new(profile: impl Into<Name>) -> Self {
        Self {
            profile: profile.into(),
            settings: StaggerSettings::default(),
            balance: BalanceSettings::default(),
        }
    }

    /// Sets `settings`.
    pub fn with_settings(mut self, settings: StaggerSettings) -> Self {
        self.settings = settings;
        self
    }

    /// Sets `balance`.
    pub fn with_balance(mut self, balance: BalanceSettings) -> Self {
        self.balance = balance;
        self
    }
}

/// A [`StaggerNode`]'s state: the stumble it is in, its balance's run and how it ended.
#[derive(Default)]
pub struct StaggerState {
    started: bool,
    run: StaggerRun,
    balance: BalanceRun,
    commands: CommandsOut,
    claims: ClaimsOut,
    ended: Option<BalanceEnd>,
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
    type DriveInput = (BodyInput, Res<'static, PhysicsState>);

    fn create_state(&self, _: &(), _: &AnimationGraphContext<'_>) -> StaggerState {
        StaggerState::default()
    }

    fn update(
        &self,
        state: &mut StaggerState,
        _: f32,
        _: &(),
        context: &mut AnimationGraphContext<'_>,
    ) {
        for stagger in context.message::<messages::Stagger>() {
            *state = StaggerState {
                started: true,
                run: StaggerRun::towards(stagger.direction, stagger.severity),
                ..StaggerState::default()
            };
        }
    }

    fn evaluate(
        &self,
        _: &mut StaggerState,
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
        state: &mut StaggerState,
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
        let (Some(body), Some(skeleton), true) = (
            BodyView::read(context.entity(), bodies),
            context.skeleton(),
            state.started,
        ) else {
            return;
        };
        if state.ended.is_none() {
            let dt = context.delta_time();
            let hindrance = StaggerController {
                settings: &self.settings,
                run: &mut state.run,
            }
            .step(body, physics, dt, &mut state.claims)
            .unwrap_or_default();
            let ended = BalanceController {
                settings: &self.balance,
                state: &mut state.balance,
            }
            .step(body, physics, dt, &hindrance, &mut state.commands);
            if let Some(end) = ended {
                state.ended = Some(end);
                end.raise(context);
            }
        } else {
            state.commands.clear();
            state.claims.clear();
        }
        let profile = body.profile(&self.profile);
        let bone_count = skeleton.bone_count();
        let attributes = output.attributes_mut();
        let commands = attributes.channel_mut::<MotorCommand>(bone_count);
        for (part, bone) in body.resolved().bones().iter().enumerate() {
            if let (Some(wanted), Some(command)) = (
                state
                    .commands
                    .layered(state.claims.get(part), body, profile, part),
                commands.get_mut(*bone),
            ) {
                *command = wanted;
            }
        }
        let amounts = attributes.channel_mut::<PhysicsAmount>(bone_count);
        for (part, bone) in body.resolved().bones().iter().enumerate() {
            let held = state
                .commands
                .layered(state.claims.get(part), body, profile, part)
                .is_some();
            if let (true, Some(amount)) = (held, amounts.get_mut(*bone)) {
                *amount = PhysicsAmount(1.0);
            }
        }
    }

    fn reset(&self, state: &mut StaggerState) {
        *state = StaggerState::default();
    }
}

impl EditorAnimationNode for StaggerNode {
    fn input_slots(&self) -> InputSlots {
        InputSlots::Fixed(vec![name!("pose")])
    }

    fn describe(&self, state: &StaggerState) -> Option<String> {
        let text = match state.ended {
            _ if !state.started => format!("waiting for a stagger at {}", self.profile),
            Some(BalanceEnd::Steadied) => format!("recovered from a stagger at {}", self.profile),
            Some(BalanceEnd::Toppled) => format!("fell in a stagger at {}", self.profile),
            None => format!(
                "staggering at {}, {} steps since it last stood still",
                self.profile,
                state.steps()
            ),
        };
        Some(text)
    }
}
