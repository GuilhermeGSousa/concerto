use concerto_animation::{
    evaluation::{AnimationGraphContext, DriveContext},
    node::{AnimationNode, EditorAnimationNode, InputSlots, ReflectAnimationNode},
    param::Param,
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
    node::balance::ClaimsOut,
    view::{BodyInput, BodyView},
};

/// Makes a body fall as a person does, from the moment its state is entered: the hands go to where it will land, with bent elbows that buckle on contact, the head is held away from the ground, the spine curls and the knees fold.
///
/// In its drive it writes a [`PhysicsAmount`] of 1 and a [`MotorCommand`] on
/// every bone whose part it holds, braced with `profile` and the fall's own
/// muscles, as its [`FallSettings`] say; its evaluate passes the pose
/// through. Once the body has landed its strength drains over
/// `relax_seconds`; then the node lets every part go limp and raises
/// [`BodyDown`] from its drive, once. A `strength` of 0 is the limp fall: the
/// body is limp at once, and the node still raises `BodyDown` when it has
/// landed and relaxed. The character needs a
/// [`BodySense`](crate::sensing::BodySense); without one the node holds the
/// body with `profile` and never lands.
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

    /// Sets `strength`.
    pub fn with_strength(mut self, strength: impl Into<Param<f32>>) -> Self {
        self.strength = strength.into();
        self
    }

    /// Sets `settings`.
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
        let base = profile.and_then(|profile| profile.command_for(body, part));
        let animated = body.animated_target(part);
        if state.limp {
            return Some(MotorCommand::limp(base, animated));
        }
        match state.claims.get(part) {
            Some(claim) => claim.layer_onto(base, animated),
            None => base,
        }
    }
}

/// A [`FallNode`]'s state: the fall's run, its claims on the parts and whether it has landed and relaxed.
#[derive(Default)]
pub struct FallState {
    run: FallRun,
    claims: ClaimsOut,
    limp: bool,
    down: bool,
}

impl AnimationNode for FallNode {
    type State = FallState;
    type CreateInput = ();
    type UpdateInput = ();
    type EvaluateInput = ();
    type DriveInput = (BodyInput, Res<'static, PhysicsState>);

    fn create_state(&self, _: &(), _: &AnimationGraphContext<'_>) -> FallState {
        FallState::default()
    }

    fn update(&self, _: &mut FallState, _: f32, _: &(), _: &mut AnimationGraphContext<'_>) {}

    fn evaluate(
        &self,
        _: &mut FallState,
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
        state: &mut FallState,
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
        if !state.down {
            let strength = context
                .blackboard()
                .read(&self.strength)
                .copied()
                .unwrap_or(1.0);
            let lying = body
                .sense()
                .is_some_and(|sense| sense.at_rest && sense.lean > LYING);
            let relaxed = FallController {
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
            state.limp = strength <= 0.0 || relaxed;
            if relaxed {
                state.down = true;
                context.raise(BodyDown);
            }
        }
        let profile = body.profile(&self.profile);
        let bone_count = skeleton.bone_count();
        let attributes = output.attributes_mut();
        let commands = attributes.channel_mut::<MotorCommand>(bone_count);
        for (part, bone) in body.resolved().bones().iter().enumerate() {
            if let (Some(wanted), Some(command)) = (
                self.motor(state, body, profile, part),
                commands.get_mut(*bone),
            ) {
                *command = wanted;
            }
        }
        let amounts = attributes.channel_mut::<PhysicsAmount>(bone_count);
        for (part, bone) in body.resolved().bones().iter().enumerate() {
            let held = self.motor(state, body, profile, part).is_some();
            if let (true, Some(amount)) = (held, amounts.get_mut(*bone)) {
                *amount = PhysicsAmount(1.0);
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
        let text = if state.down {
            format!("landed and relaxed at {}", self.profile)
        } else if state.run.landed() {
            format!("landed, relaxing at {}", self.profile)
        } else {
            format!("falling at {}", self.profile)
        };
        Some(text)
    }
}
