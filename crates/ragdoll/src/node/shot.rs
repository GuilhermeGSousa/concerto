use concerto_animation::{
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
    node::balance::{BalanceController, BalanceRun, BalanceSettings, CommandsOut, layered},
    shot::{ShotController, ShotOut, ShotSettings, Wound},
    view::BodyView,
};

/// Makes a body answer a gunshot from the moment it receives a `Shot` message, by where it struck: the muscles at the wound snap tight and then give, a hand goes to the wound, the head turns to it and then to where the shot came from, and it balances as a `BalanceNode` does, a shot leg hindering it. Each further `Shot` adds a wound. It emits `Wounded` for each part it weakens, succeeds when balance settles and fails when balance gives up; the impulses are applied by the character's hit routing, not by the node.
#[derive(Clone, Debug, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(AnimationNode, Default)]
pub struct ShotNode {
    /// The strength profile the body is powered with while it answers the shot.
    pub profile: Name,
    /// How the reaction plays out.
    pub settings: ShotSettings,
    /// How the body balances while it answers.
    pub balance: BalanceSettings,
}

impl Default for ShotNode {
    fn default() -> Self {
        Self::new(BRACED)
    }
}

impl ShotNode {
    /// A shot reaction powering `profile`, with the default settings.
    pub fn new(profile: impl Into<Name>) -> Self {
        Self {
            profile: profile.into(),
            settings: ShotSettings::default(),
            balance: BalanceSettings::default(),
        }
    }

    pub fn with_settings(mut self, settings: ShotSettings) -> Self {
        self.settings = settings;
        self
    }

    pub fn with_balance(mut self, balance: BalanceSettings) -> Self {
        self.balance = balance;
        self
    }

    fn motor(
        &self,
        state: &ShotState,
        body: &BodyView,
        profile: Option<&StrengthProfile>,
        part: usize,
    ) -> Option<MotorCommand> {
        layered(
            &state.commands,
            state.out.claims.get(part),
            body,
            profile,
            part,
        )
    }
}

/// A [`ShotNode`]'s state: the wounds so far, its balance's run and how it ended.
#[derive(Default)]
pub struct ShotState {
    started: bool,
    wounds: Vec<Wound>,
    balance: BalanceRun,
    commands: CommandsOut,
    out: ShotOut,
    outcome: Option<NodeOutcome>,
}

impl ShotState {
    /// The wounds so far, oldest first.
    pub fn wounds(&self) -> &[Wound] {
        &self.wounds
    }
}

impl AnimationNode for ShotNode {
    type State = ShotState;
    type CreateInput = ();
    type UpdateInput = ();
    type EvaluateInput = ();
    type DriveInput = Res<'static, PhysicsState>;

    fn input_slots(&self) -> InputSlots {
        InputSlots::Fixed(vec![name!("pose")])
    }

    fn create_state(&self, _: &(), _: &AnimationGraphContext<'_>) -> ShotState {
        ShotState::default()
    }

    fn update(&self, state: &mut ShotState, _: f32, _: &(), context: &AnimationGraphContext<'_>) {
        let Some(body) = context.extra::<BodyView>() else {
            return;
        };
        for shot in context.message::<messages::Shot>() {
            let Some(struck) = body.pose().get(shot.part) else {
                continue;
            };
            state.started = true;
            state.wounds.push(Wound::struck(
                shot.part,
                struck.inverse().transform_point(shot.point),
                shot.impulse,
            ));
        }
    }

    fn evaluate(
        &self,
        state: &mut ShotState,
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
        state: &mut ShotState,
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
            let hindrance = ShotController {
                settings: &self.settings,
                wounds: &mut state.wounds,
            }
            .step(body, physics, dt, &mut state.out)
            .unwrap_or_default();
            for wounded in state.out.wounded.drain(..) {
                context.graph().emit(wounded);
            }
            state.outcome = BalanceController {
                settings: &self.balance,
                state: &mut state.balance,
            }
            .step(body, physics, dt, &hindrance, &mut state.commands);
        } else {
            state.commands.clear();
            state.out.claims.clear();
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

    fn reset(&self, state: &mut ShotState) {
        *state = ShotState::default();
    }

    fn is_finished(&self, _: &ShotState) -> bool {
        false
    }

    fn outcome(&self, state: &ShotState) -> Option<NodeOutcome> {
        state.outcome
    }

    fn report(&self, state: &ShotState) -> NodeReport {
        let text = match state.outcome {
            _ if !state.started => format!("waiting for a shot at {}", self.profile),
            Some(NodeOutcome::Succeeded) => {
                format!("kept its feet after {} shots", state.wounds.len())
            }
            Some(NodeOutcome::Failed) => format!("fell after {} shots", state.wounds.len()),
            None => format!(
                "reacting to {} shots at {}",
                state.wounds.len(),
                self.profile
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
