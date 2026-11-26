use concerto_animation::{
    attribute::PoseAttribute,
    evaluation::{AnimationGraphContext, DriveContext},
    node::{AnimationNode, EditorAnimationNode, InputSlots, ReflectAnimationNode},
    pose::{AnimationFrame, EvaluatedFrame, FramePool},
};
use concerto_name::{Name, name};
use concerto_reflect::{Reflect, ReflectDefault};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    asset::{PartStrength, Segment},
    attributes::{MotorCommand, PhysicsAmount},
    character::Held,
    generate::BRACED,
    messages::{BodyDead, Death, Drained},
    node::Decided,
    view::BodyView,
};

const DYING_STIFFNESS: f32 = 400.0;
const DYING_TORQUE: f32 = 120.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Dying {
    pub(crate) seconds: f32,
    elapsed: f32,
}

impl Dying {
    pub(crate) fn new(seconds: f32) -> Self {
        Self {
            seconds,
            elapsed: 0.0,
        }
    }

    pub(crate) fn advance(&mut self, dt: f32) -> bool {
        self.elapsed += dt;
        self.elapsed >= self.seconds
    }

    pub(crate) fn drain(&self, segment: Segment, mut held: Held) -> Held {
        let left = self.left(segment);
        if held.kinematic {
            held = Held::of(PartStrength::powered(DYING_STIFFNESS, DYING_TORQUE));
        }
        held.stiffness *= left;
        held.damping *= left;
        held.torque_limit *= left;
        held
    }

    pub(crate) fn left(&self, segment: Segment) -> f32 {
        if self.seconds <= 0.0 {
            return 0.0;
        }
        let spent_at = match segment {
            Segment::Hand | Segment::Foot => 0.4,
            Segment::LowerArm | Segment::Shin => 0.6,
            Segment::UpperArm | Segment::Thigh | Segment::Head => 0.8,
            Segment::Pelvis | Segment::Spine | Segment::Other => 1.0,
        };
        (1.0 - self.elapsed / self.seconds / spent_at).clamp(0.0, 1.0)
    }
}

/// Drains a body's strength from the moment it receives a `Death` message, over its seconds, from the hands and feet inwards: every part is powered, a part that would be kinematic starting at a fixed strength, and each loses its muscles by how far its segment is from the trunk. It emits `BodyDead` and raises `Drained` once nothing is left. Zero seconds drains it at once.
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
        let strength = profile
            .zip(body.resolved().bone_name(part))
            .map_or(PartStrength::Kinematic, |(profile, bone)| {
                profile.strength_of(&bone)
            });
        let held = dying.drain(body.resolved().segment(part), Held::of(strength));
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
    drained: Decided<Drained>,
}

impl AnimationNode for DyingNode {
    type State = DyingState;
    type CreateInput = ();
    type UpdateInput = ();
    type EvaluateInput = ();
    type DriveInput = ();

    fn create_state(&self, _: &(), _: &AnimationGraphContext<'_>) -> DyingState {
        DyingState::default()
    }

    fn update(&self, state: &mut DyingState, _: f32, _: &(), context: &AnimationGraphContext<'_>) {
        for death in context.message::<Death>() {
            *state = DyingState {
                dying: Some(Dying::new(death.seconds)),
                drained: Decided::default(),
            };
        }
        if let Some(drained) = state.drained.unraised() {
            context.raise(drained);
        }
    }

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
        let (Some(body), Some(skeleton)) = (context.extra::<BodyView>(), context.skeleton()) else {
            return;
        };
        if !state.drained.is_decided()
            && let Some(dying) = state.dying.as_mut()
            && dying.advance(context.delta_time())
        {
            state.drained.decide(Drained);
            context.emit(BodyDead);
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
}

impl EditorAnimationNode for DyingNode {
    fn input_slots(&self) -> InputSlots {
        InputSlots::Fixed(vec![name!("pose")])
    }

    fn describe(&self, state: &DyingState) -> Option<String> {
        let text = match (&state.dying, state.drained.get()) {
            (None, _) => "waiting for a death".to_string(),
            (Some(_), Some(_)) => "drained".to_string(),
            (Some(dying), None) => format!("draining over {} s", dying.seconds),
        };
        Some(text)
    }
}
