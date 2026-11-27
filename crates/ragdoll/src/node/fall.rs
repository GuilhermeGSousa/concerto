use concerto_animation::param::Param;
use concerto_animation::{
    attribute::PoseAttribute,
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
    asset::StrengthProfile,
    attributes::{MotorCommand, PhysicsAmount},
    fall::{FallController, FallRun, FallSettings, LYING},
    generate::BRACED,
    messages::BodyDown,
    node::{
        Decided,
        balance::{ClaimsOut, CommandsOut, layered, wanted},
    },
    view::{BodyInput, BodyView},
};

/// Makes a body fall as a person does, from the moment it is active: braced, the hands go to where it will land, with bent elbows that buckle on contact, the head is held away from the ground, the spine curls and the knees fold. It drains its strength and goes limp once it has landed, and raises `BodyDown` once it has landed and relaxed. A strength of 0 is the limp fall: the body is limp at once.
#[derive(Clone, Debug, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(AnimationNode, Default)]
pub struct FallNode {
    /// The strength profile the body is powered with while it falls.
    pub profile: Name,
    /// How conscious the fall is, from 1, fully braced, to 0, limp; 1 while the blackboard has no such value.
    pub strength: Param<f32>,
    /// How the fall plays out.
    pub settings: FallSettings,
}

impl Default for FallNode {
    fn default() -> Self {
        Self::new(BRACED)
    }
}

impl FallNode {
    /// A fall powering `profile`, with the default settings.
    pub fn new(profile: impl Into<Name>) -> Self {
        Self {
            profile: profile.into(),
            strength: Param::default(),
            settings: FallSettings::default(),
        }
    }

    pub fn with_strength(mut self, strength: impl Into<Param<f32>>) -> Self {
        self.strength = strength.into();
        self
    }

    pub fn with_settings(mut self, settings: FallSettings) -> Self {
        self.settings = settings;
        self
    }

    fn motor(
        &self,
        state: &FallState,
        body: BodyView<'_>,
        profile: Option<&StrengthProfile>,
        part: usize,
    ) -> Option<MotorCommand> {
        if state.limp {
            return Some(limp(
                wanted(&state.commands, body, profile, part),
                body,
                part,
            ));
        }
        layered(&state.commands, state.claims.get(part), body, profile, part)
    }
}

fn limp(base: Option<MotorCommand>, body: BodyView<'_>, part: usize) -> MotorCommand {
    let neutral = MotorCommand::neutral();
    let held = base.unwrap_or(neutral);
    MotorCommand {
        cover: 1.0,
        target: body.animated_target(part),
        stiffness: held.stiffness,
        damping: held.damping,
        torque_limit: 0.0,
        ..neutral
    }
}

/// A [`FallNode`]'s state: the fall's run, its claims on the parts and whether it has landed and relaxed.
#[derive(Default)]
pub struct FallState {
    run: FallRun,
    commands: CommandsOut,
    claims: ClaimsOut,
    limp: bool,
    landed: Decided<BodyDown>,
}

impl AnimationNode for FallNode {
    type State = FallState;
    type CreateInput = ();
    type UpdateInput = ();
    type EvaluateInput = BodyInput;
    type DriveInput = (BodyInput, Res<'static, PhysicsState>);

    fn create_state(&self, _: &(), _: &AnimationGraphContext<'_>) -> FallState {
        FallState::default()
    }

    fn update(&self, state: &mut FallState, _: f32, _: &(), context: &AnimationGraphContext<'_>) {
        if let Some(landed) = state.landed.unraised() {
            context.raise(landed);
        }
    }

    fn evaluate(
        &self,
        state: &mut FallState,
        bodies: &SystemInputData<'_, '_, BodyInput>,
        context: &AnimationGraphContext<'_>,
        _: &[Uuid],
        inputs: &[EvaluatedFrame],
        _: &mut FramePool,
        output: &mut AnimationFrame,
    ) {
        if let Some(input) = inputs.first() {
            output.copy_from(&input.frame);
        }
        let (Some(body), Some(skeleton)) =
            (BodyView::read(context.entity(), bodies), context.skeleton())
        else {
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
        state: &mut FallState,
        input: &SystemInputData<'_, '_, Self::DriveInput>,
        context: &DriveContext<'_>,
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
        let strength = context
            .blackboard()
            .read(&self.strength)
            .copied()
            .unwrap_or(1.0);
        if !state.landed.is_decided() {
            let lying = body
                .sense()
                .is_some_and(|sense| sense.at_rest && sense.lean > LYING);
            let step = FallController {
                settings: &self.settings,
                strength,
                run: &mut state.run,
            }
            .step(
                body,
                physics,
                context.delta_time(),
                lying,
                &mut state.claims,
            );
            state.limp = strength <= 0.0;
            if step.is_some_and(|step| step.relaxed) {
                state.limp = true;
                state.landed.decide(BodyDown);
            }
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

    fn reset(&self, state: &mut FallState) {
        *state = FallState::default();
    }
}

impl EditorAnimationNode for FallNode {
    fn input_slots(&self) -> InputSlots {
        InputSlots::Fixed(vec![name!("pose")])
    }

    fn describe(&self, state: &FallState) -> Option<String> {
        let text = match state.landed.get() {
            Some(_) => format!("landed and relaxed at {}", self.profile),
            None if state.run.landed.is_some() => {
                format!("landed, relaxing at {}", self.profile)
            }
            None => format!("falling at {}", self.profile),
        };
        Some(text)
    }
}
