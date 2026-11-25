use concerto_animation::{
    attribute::PoseAttribute,
    blackboard::AnimationBlackboard,
    evaluation::{AnimationGraphContext, DriveContext},
    mask::{BoneMask, MaskResolution, resolve_mask},
    node::{AnimationNode, BehaviourReport, InputSlots, NodeReport, ReflectAnimationNode},
    param::Param,
    pose::{AnimationFrame, EvaluatedFrame, FramePool},
};
use concerto_ecs::Res;
use concerto_foundation::assets::{asset_store::AssetStore, handle::AssetHandle};
use concerto_mesh::Skeleton;
use concerto_name::{Name, name};
use concerto_reflect::{Reflect, ReflectDefault};
use glam::Quat;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    asset::PartStrength,
    attributes::{MotorCommand, PhysicsAmount},
    generate::BRACED,
    node::animated_target,
    view::BodyView,
};

/// Holds the parts on a mask's bones with the muscles of a strength profile, drawn from physics by its weight: a hurt leg trailing the walk.
#[derive(Clone, Debug, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(AnimationNode, Default)]
pub struct HoldNode {
    /// The strength profile of the ragdoll asset whose muscles hold the parts; a part it leaves kinematic is not held.
    pub profile: Name,
    /// The bones whose parts are held, by their weights; every part when there is none.
    pub mask: Option<AssetHandle<BoneMask>>,
    /// How much of the held parts is drawn from physics and held, from 0 to 1; 1 while the blackboard has no such value.
    pub weight: Param<f32>,
    /// Multiplies the profile's stiffness, damping and torque limit; 1 while the blackboard has no such value.
    pub strength: Param<f32>,
    /// The seconds over which a change of weight or strength takes hold.
    pub fade: f32,
}

impl Default for HoldNode {
    fn default() -> Self {
        Self::new(BRACED)
    }
}

impl HoldNode {
    /// Holds every part `profile` powers, at full weight and strength, fading changes over 0.2 s.
    pub fn new(profile: impl Into<Name>) -> Self {
        Self {
            profile: profile.into(),
            mask: None,
            weight: Param::default(),
            strength: Param::default(),
            fade: 0.2,
        }
    }

    pub fn with_mask(mut self, mask: AssetHandle<BoneMask>) -> Self {
        self.mask = Some(mask);
        self
    }

    pub fn with_weight(mut self, weight: impl Into<Param<f32>>) -> Self {
        self.weight = weight.into();
        self
    }

    pub fn with_strength(mut self, strength: impl Into<Param<f32>>) -> Self {
        self.strength = strength.into();
        self
    }

    pub fn with_fade(mut self, fade: f32) -> Self {
        self.fade = fade;
        self
    }

    fn target_weight(&self, blackboard: &AnimationBlackboard) -> f32 {
        let weight = read(blackboard, &self.weight);
        if weight.is_nan() {
            0.0
        } else {
            weight.clamp(0.0, 1.0)
        }
    }

    fn target_strength(&self, blackboard: &AnimationBlackboard) -> f32 {
        let strength = read(blackboard, &self.strength);
        if strength.is_nan() {
            0.0
        } else {
            strength.max(0.0)
        }
    }

    fn approach(&self, current: Option<f32>, target: f32, delta_time: f32) -> f32 {
        match current {
            Some(current) if self.fade > 0.0 => {
                let step = delta_time.max(0.0) / self.fade;
                current + (target - current).clamp(-step, step)
            }
            _ => target,
        }
    }

    fn find_parts(
        &self,
        state: &mut HoldState,
        masks: &AssetStore<BoneMask>,
        skeleton: &Skeleton,
        root: Option<usize>,
        body: &BodyView,
    ) {
        state.parts.clear();
        let weights = match &self.mask {
            Some(handle) => {
                let Some(mask) = masks.get(handle) else {
                    return;
                };
                resolve_mask(mask, skeleton, root, &mut state.mask);
                Some(&state.mask.weights)
            }
            None => None,
        };
        let Some(profile) = body.profile(&self.profile) else {
            return;
        };
        for (part, bone) in body.resolved().bones().iter().enumerate() {
            let share = weights.map_or(1.0, |weights| weights.get(*bone).copied().unwrap_or(0.0));
            let Some(name) = body.resolved().bone_name(part) else {
                continue;
            };
            let strength = profile.strength_of(&name);
            if share > 0.0 && strength != PartStrength::Kinematic {
                state.parts.push(HeldPart {
                    part,
                    bone: *bone,
                    share,
                    strength,
                });
            }
        }
    }
}

fn read(blackboard: &AnimationBlackboard, param: &Param<f32>) -> f32 {
    blackboard.read(param).copied().unwrap_or(1.0)
}

fn command(strength: PartStrength, scale: f32, target: Quat) -> MotorCommand {
    let mut command = MotorCommand {
        cover: 1.0,
        target,
        ..MotorCommand::neutral()
    };
    if let PartStrength::Powered {
        stiffness,
        damping,
        torque_limit,
    } = strength
    {
        command.stiffness = stiffness * scale;
        command.damping = damping * scale;
        command.torque_limit = torque_limit * scale;
    }
    command
}

struct HeldPart {
    part: usize,
    bone: usize,
    share: f32,
    strength: PartStrength,
}

/// A [`HoldNode`]'s smoothed weight and strength, its resolved mask, and the parts it held last.
#[derive(Default)]
pub struct HoldState {
    weight: Option<f32>,
    strength: Option<f32>,
    mask: MaskResolution,
    parts: Vec<HeldPart>,
}

impl AnimationNode for HoldNode {
    type State = HoldState;
    type CreateInput = ();
    type UpdateInput = ();
    type EvaluateInput = Res<'static, AssetStore<BoneMask>>;
    type DriveInput = Res<'static, AssetStore<BoneMask>>;

    fn input_slots(&self) -> InputSlots {
        InputSlots::Fixed(vec![name!("pose")])
    }

    fn create_state(&self, _: &(), _: &AnimationGraphContext<'_>) -> HoldState {
        HoldState::default()
    }

    fn update(
        &self,
        state: &mut HoldState,
        delta_time: f32,
        _: &(),
        context: &AnimationGraphContext<'_>,
    ) {
        let blackboard = context.blackboard();
        state.weight =
            Some(self.approach(state.weight, self.target_weight(blackboard), delta_time));
        state.strength =
            Some(self.approach(state.strength, self.target_strength(blackboard), delta_time));
    }

    fn evaluate(
        &self,
        state: &mut HoldState,
        masks: &Res<AssetStore<BoneMask>>,
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
        let root = context.root().map(|root| root.index);
        self.find_parts(state, masks, skeleton, root, body);
        let weight = state
            .weight
            .unwrap_or_else(|| self.target_weight(context.blackboard()));
        let amounts = output
            .attributes_mut()
            .channel_mut::<PhysicsAmount>(skeleton.bone_count());
        for held in &state.parts {
            let share = weight * held.share;
            if let Some(amount) = amounts.get_mut(held.bone) {
                amount.blend(1.0 - share, &PhysicsAmount(1.0), share);
            }
        }
    }

    fn drive(
        &self,
        state: &mut HoldState,
        masks: &Res<AssetStore<BoneMask>>,
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
        let root = graph.root().map(|root| root.index);
        self.find_parts(state, masks, skeleton, root, body);
        let weight = state
            .weight
            .unwrap_or_else(|| self.target_weight(graph.blackboard()));
        let strength = state
            .strength
            .unwrap_or_else(|| self.target_strength(graph.blackboard()));
        let commands = output
            .attributes_mut()
            .channel_mut::<MotorCommand>(skeleton.bone_count());
        for held in &state.parts {
            let share = weight * held.share;
            let muscle = strength * body.wounds().strength_of(held.part);
            let wanted = command(held.strength, muscle, animated_target(body, held.part));
            if let Some(command) = commands.get_mut(held.bone) {
                command.blend(1.0 - share, &wanted, share);
            }
        }
    }

    fn reset(&self, state: &mut HoldState) {
        state.weight = None;
        state.strength = None;
    }

    fn is_finished(&self, _: &HoldState) -> bool {
        false
    }

    fn report(&self, state: &HoldState) -> NodeReport {
        NodeReport {
            behaviour: Some(BehaviourReport::running(format!(
                "holding {} parts at {} ×{:.2}",
                state.parts.len(),
                self.profile,
                state.strength.unwrap_or(1.0)
            ))),
            ..NodeReport::default()
        }
    }
}
