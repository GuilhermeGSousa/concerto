use concerto_animation::{
    attribute::PoseAttribute,
    evaluation::{AnimationGraphContext, DriveContext},
    node::{AnimationNode, EditorAnimationNode, InputSlots, ReflectAnimationNode},
    pose::{AnimationFrame, EvaluatedFrame, FramePool},
};
use concerto_ecs::system::input::SystemInputData;
use concerto_name::{Name, name};
use concerto_reflect::{Reflect, ReflectDefault};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    asset::PartStrength,
    attributes::{MotorCommand, PhysicsAmount},
    character::Held,
    dying::Dying,
    generate::BRACED,
    messages::{BodyDead, Death},
    view::{BodyInput, BodyView},
};

/// Drains a body's strength from the moment it receives a [`Death`] message, over its seconds, from the hands and feet inwards.
///
/// Every part is powered, a part its profile leaves kinematic starting at a
/// fixed strength, and each loses its muscles by how far its segment is from
/// the trunk. In its drive it gives every bone with a part a [`PhysicsAmount`]
/// of 1 and a [`MotorCommand`] aimed at the animation with what strength is
/// left, and raises [`BodyDead`] once nothing is left; its evaluate passes the
/// pose through. Zero seconds drains it at once. A later `Death` starts the
/// drain again.
#[derive(Clone, Debug, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(AnimationNode, Default)]
pub struct DyingNode {
    /// The strength profile the body is powered with before it drains.
    pub profile: Name,
}

impl Default for DyingNode {
    fn default() -> Self {
        Self::new(BRACED)
    }
}

impl DyingNode {
    /// A dying body powered by `profile` until its strength drains.
    pub fn new(profile: impl Into<Name>) -> Self {
        Self {
            profile: profile.into(),
        }
    }

    fn motor(&self, dying: &Dying, body: BodyView<'_>, part: usize) -> MotorCommand {
        let strength = body
            .profile(&self.profile)
            .zip(body.resolved().bone_name(part))
            .map_or(PartStrength::Kinematic, |(profile, bone)| {
                profile.strength_of(&bone)
            });
        let held = dying.drain(body.resolved().segment(part), Held::of(strength));
        MotorCommand {
            cover: 1.0,
            target: body.animated_target(part),
            stiffness: held.stiffness,
            damping: held.damping,
            torque_limit: held.torque_limit,
            ..MotorCommand::neutral()
        }
    }
}

/// A [`DyingNode`]'s state: the drain it has begun, if any, and whether it has finished.
#[derive(Default)]
pub struct DyingState {
    dying: Option<Dying>,
    drained: bool,
}

impl AnimationNode for DyingNode {
    type State = DyingState;
    type CreateInput = ();
    type UpdateInput = ();
    type EvaluateInput = ();
    type DriveInput = BodyInput;

    fn create_state(&self, _: &(), _: &AnimationGraphContext<'_>) -> DyingState {
        DyingState::default()
    }

    fn update(
        &self,
        state: &mut DyingState,
        _: f32,
        _: &(),
        context: &mut AnimationGraphContext<'_>,
    ) {
        for death in context.message::<Death>() {
            *state = DyingState {
                dying: Some(Dying::new(death.seconds)),
                drained: false,
            };
        }
    }

    fn evaluate(
        &self,
        _: &mut DyingState,
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
        state: &mut DyingState,
        bodies: &SystemInputData<'_, '_, BodyInput>,
        context: &mut DriveContext<'_>,
        _: &[Uuid],
        inputs: &[EvaluatedFrame],
        _: &mut FramePool,
        output: &mut AnimationFrame,
    ) {
        if let Some(input) = inputs.first() {
            output.attributes_mut().copy_from(input.frame.attributes());
        }
        let (Some(body), Some(skeleton)) =
            (BodyView::read(context.entity(), bodies), context.skeleton())
        else {
            return;
        };
        let Some(dying) = state.dying.as_mut() else {
            return;
        };
        if !state.drained && dying.advance(context.delta_time()) {
            state.drained = true;
            context.raise(BodyDead);
        }
        let bone_count = skeleton.bone_count();
        let attributes = output.attributes_mut();
        let amounts = attributes.channel_mut::<PhysicsAmount>(bone_count);
        for bone in body.resolved().bones() {
            if let Some(amount) = amounts.get_mut(*bone) {
                *amount = PhysicsAmount(1.0);
            }
        }
        let commands = attributes.channel_mut::<MotorCommand>(bone_count);
        for (part, bone) in body.resolved().bones().iter().enumerate() {
            if let Some(command) = commands.get_mut(*bone) {
                *command = self.motor(dying, body, part);
            }
        }
    }

    fn reset(&self, state: &mut DyingState) {
        *state = DyingState::default();
    }
}

impl EditorAnimationNode for DyingNode {
    fn input_slots(&self) -> InputSlots {
        InputSlots::Fixed(vec![name!("pose")])
    }

    fn describe(&self, state: &DyingState) -> Option<String> {
        let text = match &state.dying {
            None => "waiting for a death".to_string(),
            Some(_) if state.drained => "drained".to_string(),
            Some(dying) => format!("draining over {} s", dying.seconds),
        };
        Some(text)
    }
}
