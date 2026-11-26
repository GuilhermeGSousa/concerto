use concerto_animation::{
    evaluation::{AnimationGraphContext, DriveContext},
    node::{AnimationNode, EditorAnimationNode, InputSlots, ReflectAnimationNode},
    pose::{AnimationFrame, EvaluatedFrame, FramePool},
};
use concerto_name::name;
use concerto_reflect::{Reflect, ReflectDefault};
use glam::Vec3;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    attributes::{MotorCommand, PhysicsAmount},
    view::BodyView,
};

/// Lets the whole body go: every part drawn from physics, its motors off, their stiffness kept so a motor taking hold again does not jump.
#[derive(Clone, Debug, Default, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(AnimationNode, Default)]
pub struct LimpNode {}

impl AnimationNode for LimpNode {
    type State = ();
    type CreateInput = ();
    type UpdateInput = ();
    type EvaluateInput = ();
    type DriveInput = ();

    fn create_state(&self, _: &(), _: &AnimationGraphContext<'_>) {}

    fn update(&self, _: &mut (), _: f32, _: &(), _: &AnimationGraphContext<'_>) {}

    fn evaluate(
        &self,
        _: &mut (),
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
        let (Some(body), Some(skeleton)) = (context.extra::<BodyView>(), context.skeleton()) else {
            return;
        };
        let amounts = output
            .attributes_mut()
            .channel_mut::<PhysicsAmount>(skeleton.bone_count());
        for bone in body.resolved().bones() {
            if let Some(amount) = amounts.get_mut(*bone) {
                *amount = PhysicsAmount(1.0);
            }
        }
    }

    fn drive(
        &self,
        _: &mut (),
        _: &(),
        context: &DriveContext<'_>,
        _: &[Uuid],
        inputs: &[EvaluatedFrame],
        _: &mut FramePool,
        output: &mut AnimationFrame,
    ) {
        if let Some(input) = inputs.first() {
            output.attributes_mut().copy_from(input.frame.attributes());
        }
        let (Some(body), Some(skeleton)) = (context.extra::<BodyView>(), context.skeleton()) else {
            return;
        };
        let commands = output
            .attributes_mut()
            .channel_mut::<MotorCommand>(skeleton.bone_count());
        for (part, bone) in body.resolved().bones().iter().enumerate() {
            let Some(command) = commands.get_mut(*bone) else {
                continue;
            };
            let target = if command.cover > 0.0 {
                command.target
            } else {
                body.animated_target(part)
            };
            *command = MotorCommand {
                cover: 1.0,
                target,
                stiffness: command.stiffness,
                damping: command.damping,
                torque_limit: 0.0,
                force: Vec3::ZERO,
                torque: Vec3::ZERO,
            };
        }
    }

    fn reset(&self, _: &mut ()) {}
}

impl EditorAnimationNode for LimpNode {
    fn input_slots(&self) -> InputSlots {
        InputSlots::Fixed(vec![name!("pose")])
    }
}
