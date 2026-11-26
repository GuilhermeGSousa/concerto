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
    asset::PartStrength,
    attributes::{MotorCommand, PhysicsAmount},
    generate::FLINCH,
    messages::{Hit, Recovered},
    node::hold::command,
    view::BodyView,
};

/// Makes a character flinch when it is hit while its ragdoll follows its animation: the parts of `profile` are powered and take the impulse, the muscles at the hit give and recover, and the body settles back on the animation, when it raises `Recovered`. Play it from a slot entry that starts on `Hit` and ends on `Recovered`.
#[derive(Clone, Debug, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(AnimationNode, Default)]
pub struct FlinchNode {
    /// The strength profile the body is powered with while it flinches.
    pub profile: Name,
    /// What is left of the hit part's strength at the moment of the hit, from 0 to 1.
    pub dip: f32,
    /// What is left of the strength of the parts joined to the hit part.
    pub neighbour_dip: f32,
    /// The seconds over which the dipped strength comes back.
    pub recover: f32,
    /// The seconds after the last hit during which the simulated body is drawn in full.
    pub hold: f32,
    /// The seconds over which the drawn body then returns to the animation; when they are over the node has succeeded.
    pub settle: f32,
}

impl Default for FlinchNode {
    fn default() -> Self {
        Self::new(FLINCH)
    }
}

impl FlinchNode {
    /// A flinch powering `profile`, with the default settings.
    pub fn new(profile: impl Into<Name>) -> Self {
        Self {
            profile: profile.into(),
            dip: 0.25,
            neighbour_dip: 0.6,
            recover: 0.5,
            hold: 0.5,
            settle: 0.4,
        }
    }

    pub fn with_dip(mut self, dip: f32) -> Self {
        self.dip = dip;
        self
    }

    pub fn with_neighbour_dip(mut self, neighbour_dip: f32) -> Self {
        self.neighbour_dip = neighbour_dip;
        self
    }

    pub fn with_recover(mut self, recover: f32) -> Self {
        self.recover = recover;
        self
    }

    pub fn with_hold(mut self, hold: f32) -> Self {
        self.hold = hold;
        self
    }

    pub fn with_settle(mut self, settle: f32) -> Self {
        self.settle = settle;
        self
    }

    fn weight(&self, since_hit: f32) -> f32 {
        let over = since_hit - self.hold;
        if over <= 0.0 {
            1.0
        } else if self.settle > 0.0 {
            (1.0 - over / self.settle).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    fn dip_around(&self, scales: &mut [f32], body: &BodyView, part: usize) {
        let parts = &body.resolved().settings().parts;
        for (index, scale) in scales.iter_mut().enumerate() {
            let joined = parts[index].parent == Some(part) || parts[part].parent == Some(index);
            let left = if index == part {
                self.dip
            } else if joined {
                self.neighbour_dip
            } else {
                continue;
            };
            *scale = scale.min(left.clamp(0.0, 1.0));
        }
    }

    fn take(&self, state: &mut FlinchState, body: &BodyView, hit: &Hit) {
        let count = body.resolved().part_count();
        if hit.part >= count {
            return;
        }
        state.scales.resize(count, 1.0);
        self.dip_around(&mut state.scales, body, hit.part);
        state.struck = Some(Struck {
            part: hit.part,
            since_hit: 0.0,
            recovered: false,
        });
    }

    fn acting_weight(&self, state: &FlinchState) -> Option<f32> {
        state
            .struck
            .as_ref()
            .map(|struck| self.weight(struck.since_hit))
            .filter(|weight| *weight > 0.0)
    }
}

struct Struck {
    part: usize,
    since_hit: f32,
    recovered: bool,
}

/// A [`FlinchNode`]'s state: the hit it is answering, and what is left of each part's strength.
#[derive(Default)]
pub struct FlinchState {
    struck: Option<Struck>,
    scales: Vec<f32>,
}

impl AnimationNode for FlinchNode {
    type State = FlinchState;
    type CreateInput = ();
    type UpdateInput = ();
    type EvaluateInput = ();
    type DriveInput = ();

    fn create_state(&self, _: &(), _: &AnimationGraphContext<'_>) -> FlinchState {
        FlinchState::default()
    }

    fn update(
        &self,
        state: &mut FlinchState,
        delta_time: f32,
        _: &(),
        context: &AnimationGraphContext<'_>,
    ) {
        if let Some(body) = context.extra::<BodyView>() {
            for hit in context.message::<Hit>() {
                self.take(state, body, hit);
            }
        }
        let Some(struck) = state.struck.as_mut() else {
            return;
        };
        struck.since_hit += delta_time;
        let regained = if self.recover > 0.0 {
            delta_time / self.recover
        } else {
            1.0
        };
        for scale in &mut state.scales {
            *scale = (*scale + regained).min(1.0);
        }
        if !struck.recovered && self.weight(struck.since_hit) <= 0.0 {
            struck.recovered = true;
            context.raise(Recovered);
        }
    }

    fn evaluate(
        &self,
        state: &mut FlinchState,
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
        let (Some(body), Some(skeleton), Some(weight)) = (
            context.extra::<BodyView>(),
            context.skeleton(),
            self.acting_weight(state),
        ) else {
            return;
        };
        let Some(profile) = body.profile(&self.profile) else {
            return;
        };
        let resolved = body.resolved();
        let amounts = output
            .attributes_mut()
            .channel_mut::<PhysicsAmount>(skeleton.bone_count());
        for (part, bone) in resolved.bones().iter().enumerate() {
            let powered = resolved
                .bone_name(part)
                .is_some_and(|name| profile.strength_of(&name) != PartStrength::Kinematic);
            if let (true, Some(amount)) = (powered, amounts.get_mut(*bone)) {
                amount.blend(1.0 - weight, &PhysicsAmount(1.0), weight);
            }
        }
    }

    fn drive(
        &self,
        state: &mut FlinchState,
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
        let (Some(body), Some(skeleton), Some(weight)) = (
            context.extra::<BodyView>(),
            context.skeleton(),
            self.acting_weight(state),
        ) else {
            return;
        };
        let Some(profile) = body.profile(&self.profile) else {
            return;
        };
        let resolved = body.resolved();
        let commands = output
            .attributes_mut()
            .channel_mut::<MotorCommand>(skeleton.bone_count());
        for (part, bone) in resolved.bones().iter().enumerate() {
            let Some(strength) = resolved
                .bone_name(part)
                .map(|name| profile.strength_of(&name))
            else {
                continue;
            };
            if strength == PartStrength::Kinematic {
                continue;
            }
            let scale = state.scales.get(part).copied().unwrap_or(1.0);
            let wanted = command(strength, scale, body.animated_target(part));
            if let Some(command) = commands.get_mut(*bone) {
                command.blend(1.0 - weight, &wanted, weight);
            }
        }
    }

    fn reset(&self, state: &mut FlinchState) {
        state.struck = None;
        state.scales.clear();
    }
}

impl EditorAnimationNode for FlinchNode {
    fn input_slots(&self) -> InputSlots {
        InputSlots::Fixed(vec![name!("pose")])
    }

    fn describe(&self, state: &FlinchState) -> Option<String> {
        let text = match &state.struck {
            Some(struck) if struck.recovered => format!(
                "recovered from a hit on part {} at {}",
                struck.part, self.profile
            ),
            Some(struck) => format!(
                "flinching from a hit on part {} at {}, {:.2} s ago",
                struck.part, self.profile, struck.since_hit
            ),
            None => format!("waiting for a hit at {}", self.profile),
        };
        Some(text)
    }
}
