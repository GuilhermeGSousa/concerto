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
    messages::{self, Wounded},
    node::balance::{
        BalanceController, BalanceEnd, BalanceRun, BalanceSettings, ClaimsOut, CommandsOut,
    },
    shot::{ShotController, ShotSettings, Wound},
    view::{BodyInput, BodyView},
};

/// Makes a body answer a gunshot from the moment it receives a [`Shot`](messages::Shot) message, by where it struck: the muscles at the wound snap tight and then give, a hand goes to the wound, the head turns to it and then to where the shot came from, and it balances as a [`BalanceNode`](crate::node::BalanceNode) does, a shot leg hindering it.
///
/// Each further `Shot` adds a wound. Until the first it only passes its input
/// through. From then on, in its drive, it writes a [`PhysicsAmount`] of 1 and
/// a [`MotorCommand`] on every bone whose part it holds with `profile`,
/// balance or the reaction; its evaluate passes the pose through. The
/// character's hit routing applies the bullets' impulses, not the node.
///
/// It raises [`Wounded`] from its update for each part a shot weakens for
/// good, [`Steadied`](messages::Steadied) from its drive once balance settles
/// and [`Toppled`](messages::Toppled) when balance gives up; after either of
/// those it holds the body with `profile` alone. The character needs a
/// [`BodySense`](crate::sensing::BodySense); without one the node holds the
/// body with `profile` and never balances or ends.
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

    /// Sets `settings`.
    pub fn with_settings(mut self, settings: ShotSettings) -> Self {
        self.settings = settings;
        self
    }

    /// Sets `balance`.
    pub fn with_balance(mut self, balance: BalanceSettings) -> Self {
        self.balance = balance;
        self
    }
}

/// A [`ShotNode`]'s state: the wounds so far, its balance's run and how it ended.
#[derive(Default)]
pub struct ShotState {
    started: bool,
    wounds: Vec<Wound>,
    balance: BalanceRun,
    commands: CommandsOut,
    claims: ClaimsOut,
    ended: Option<BalanceEnd>,
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
    type UpdateInput = BodyInput;
    type EvaluateInput = ();
    type DriveInput = (BodyInput, Res<'static, PhysicsState>);

    fn create_state(&self, _: &(), _: &AnimationGraphContext<'_>) -> ShotState {
        ShotState::default()
    }

    fn update(
        &self,
        state: &mut ShotState,
        _: f32,
        bodies: &SystemInputData<'_, '_, BodyInput>,
        context: &mut AnimationGraphContext<'_>,
    ) {
        let Some(body) = BodyView::read(context.entity(), bodies) else {
            return;
        };
        for shot in context.message::<messages::Shot>() {
            let Some(struck) = body.pose().get(shot.part) else {
                continue;
            };
            state.started = true;
            if let Some(share) = self
                .settings
                .lasting_share(body.resolved().segment(shot.part))
            {
                context.raise(Wounded {
                    part: shot.part,
                    share,
                });
            }
            state.wounds.push(Wound::struck(
                shot.part,
                struck.inverse().transform_point(shot.point),
                shot.impulse,
            ));
        }
    }

    fn evaluate(
        &self,
        _: &mut ShotState,
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
        state: &mut ShotState,
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
            let hindrance = ShotController {
                settings: &self.settings,
                wounds: &mut state.wounds,
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

    fn reset(&self, state: &mut ShotState) {
        *state = ShotState::default();
    }
}

impl EditorAnimationNode for ShotNode {
    fn input_slots(&self) -> InputSlots {
        InputSlots::Fixed(vec![name!("pose")])
    }

    fn describe(&self, state: &ShotState) -> Option<String> {
        let text = match state.ended {
            _ if !state.started => format!("waiting for a shot at {}", self.profile),
            Some(BalanceEnd::Steadied) => {
                format!("kept its feet after {} shots", state.wounds.len())
            }
            Some(BalanceEnd::Toppled) => format!("fell after {} shots", state.wounds.len()),
            None => format!(
                "reacting to {} shots at {}",
                state.wounds.len(),
                self.profile
            ),
        };
        Some(text)
    }
}
