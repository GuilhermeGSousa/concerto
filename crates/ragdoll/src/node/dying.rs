use concerto_animation::{
    attribute::PoseAttribute,
    evaluation::{AnimationGraphContext, DriveContext},
    node::{
        AnimationNode, BehaviourReport, InputSlots, NodeOutcome, NodeReport, ReflectAnimationNode,
    },
    pose::{AnimationFrame, EvaluatedFrame, FramePool},
};
use concerto_name::{Name, name};
use concerto_reflect::{Reflect, ReflectDefault};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    attributes::{MotorCommand, PhysicsAmount},
    character::Held,
    dying::Dying,
    generate::BRACED,
    messages::{BodyDead, Death},
    node::balance::{CommandsOut, wanted},
    view::BodyView,
};

/// Drains a body's strength from the moment it receives a `Death` message, over its seconds, from the hands and feet inwards: every part is powered, a part that would be kinematic starting at a fixed strength, and each loses its muscles by how far its segment is from the trunk. It emits `BodyDead` once nothing is left and succeeds. Zero seconds drains it at once.
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

    fn motor(&self, state: &DyingState, body: &BodyView, part: usize) -> Option<MotorCommand> {
        let dying = state.dying.as_ref()?;
        let profile = body.profile(&self.profile);
        let base = wanted(&CommandsOut::default(), body, profile, part);
        let held = match base {
            Some(base) => Held {
                kinematic: false,
                stiffness: base.stiffness,
                damping: base.damping,
                torque_limit: base.torque_limit,
            },
            None => Held::KINEMATIC,
        };
        let (held, _) = dying.drain(body.resolved().segment(part), held);
        Some(MotorCommand {
            cover: 1.0,
            target: body.animated_target(part),
            stiffness: held.stiffness,
            damping: held.damping,
            torque_limit: held.torque_limit,
            ..MotorCommand::neutral()
        })
    }
}

/// A [`DyingNode`]'s state: the drain it has begun, if any, and whether it has finished.
#[derive(Default)]
pub struct DyingState {
    dying: Option<Dying>,
    outcome: Option<NodeOutcome>,
}

impl AnimationNode for DyingNode {
    type State = DyingState;
    type CreateInput = ();
    type UpdateInput = ();
    type EvaluateInput = ();
    type DriveInput = ();

    fn input_slots(&self) -> InputSlots {
        InputSlots::Fixed(vec![name!("pose")])
    }

    fn create_state(&self, _: &(), _: &AnimationGraphContext<'_>) -> DyingState {
        DyingState::default()
    }

    fn receive(
        &self,
        state: &mut DyingState,
        message: &dyn Reflect,
        _: &AnimationGraphContext<'_>,
    ) {
        if let Some(death) = message.downcast_ref::<Death>() {
            *state = DyingState {
                dying: Some(Dying::new(death.seconds)),
                outcome: None,
            };
        }
    }

    fn update(&self, _: &mut DyingState, _: f32, _: &(), _: &AnimationGraphContext<'_>) {}

    fn evaluate(
        &self,
        state: &mut DyingState,
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
        if state.dying.is_none() {
            return;
        }
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
        state: &mut DyingState,
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
        let graph = context.graph();
        let (Some(body), Some(skeleton)) = (context.extra::<BodyView>(), graph.skeleton()) else {
            return;
        };
        if state.outcome.is_none()
            && let Some(dying) = state.dying.as_mut()
            && dying.advance(context.delta_time())
        {
            state.outcome = Some(NodeOutcome::Succeeded);
            graph.emit(BodyDead);
        }
        let commands = output
            .attributes_mut()
            .channel_mut::<MotorCommand>(skeleton.bone_count());
        for (part, bone) in body.resolved().bones().iter().enumerate() {
            if let (Some(wanted), Some(command)) =
                (self.motor(state, body, part), commands.get_mut(*bone))
            {
                *command = wanted;
            }
        }
    }

    fn reset(&self, state: &mut DyingState) {
        *state = DyingState::default();
    }

    fn is_finished(&self, _: &DyingState) -> bool {
        false
    }

    fn outcome(&self, state: &DyingState) -> Option<NodeOutcome> {
        state.outcome
    }

    fn report(&self, state: &DyingState) -> NodeReport {
        let text = match (&state.dying, state.outcome) {
            (None, _) => "waiting for a death".to_string(),
            (Some(_), Some(_)) => "drained".to_string(),
            (Some(dying), None) => format!("draining over {} s", dying.seconds),
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
