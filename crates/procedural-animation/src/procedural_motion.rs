use std::f64::consts::TAU;

use concerto_animation::{
    blackboard::AnimationBlackboard,
    evaluation::AnimationGraphContext,
    mask::{BoneMask, MaskResolution, resolve_mask},
    node::{AnimationNode, InputSlots, ReflectAnimationNode, model_space::parent},
    param::Param,
    pose::{AnimationFrame, EvaluatedFrame, FramePool},
};
use concerto_ecs::Res;
use concerto_foundation::{
    assets::{asset_store::AssetStore, handle::AssetHandle},
    math::Spring,
};
use concerto_mesh::Skeleton;
use concerto_name::name;
use concerto_noise::{fractal_noise, mix_seed, unit_from_seed};
use concerto_reflect::{Reflect, ReflectDefault};
use glam::{Quat, Vec3};
use log::warn;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Adds rotations that wave or wander over time to the bones of each channel's mask.
///
/// It passes its first input through first.
#[derive(Clone, Debug, Default, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(AnimationNode, Default)]
pub struct ProceduralMotionNode {
    pub channels: Vec<ProceduralMotionChannel>,
    pub seed: u64,
    pub seed_param: Option<Param<u64>>,
}

impl ProceduralMotionNode {
    /// A node with no channels, seed 0 and no seed param.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one channel after the others.
    pub fn with_channel(mut self, channel: ProceduralMotionChannel) -> Self {
        self.channels.push(channel);
        self
    }

    /// Sets the seed mixed with the param's to pick each channel's phase and noise.
    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    /// Mixes in the `u64` blackboard param named `param`, so each character can move out of step.
    pub fn with_seed_param(mut self, param: impl Into<Param<u64>>) -> Self {
        self.seed_param = Some(param.into());
        self
    }

    fn param_seed(&self, blackboard: &AnimationBlackboard) -> u64 {
        let Some(param) = self.seed_param else {
            return 0;
        };
        blackboard.read(&param).copied().unwrap_or(0)
    }
}

/// One motion: the bones it moves, its waveform and frequency in hertz, its peak rotation in radians about each model axis, and the param that scales it.
#[derive(Clone, Debug, PartialEq, Reflect, Serialize, Deserialize)]
pub struct ProceduralMotionChannel {
    pub mask: AssetHandle<BoneMask>,
    pub waveform: Waveform,
    pub frequency: f32,
    pub amplitude: Vec3,
    pub magnitude_param: Option<Param<f32>>,
    pub magnitude_halflife: f32,
}

impl Default for ProceduralMotionChannel {
    fn default() -> Self {
        Self {
            mask: AssetHandle::default(),
            waveform: Waveform::default(),
            frequency: 1.0,
            amplitude: Vec3::splat(1.0_f32.to_radians()),
            magnitude_param: None,
            magnitude_halflife: 0.0,
        }
    }
}

impl ProceduralMotionChannel {
    /// A channel over the bones of `mask`, with the default waveform, frequency and amplitude.
    pub fn new(mask: AssetHandle<BoneMask>) -> Self {
        Self {
            mask,
            ..Self::default()
        }
    }

    /// A slow sine pitch, for the chest and spine.
    pub fn breathing(mask: AssetHandle<BoneMask>) -> Self {
        Self::new(mask)
            .with_waveform(Waveform::Sine)
            .with_frequency(0.25)
            .with_amplitude(Vec3::new(2.0_f32.to_radians(), 0.0, 0.0))
    }

    /// A slow wander on every axis, for aim sway.
    pub fn sway(mask: AssetHandle<BoneMask>) -> Self {
        Self::new(mask)
            .with_waveform(Waveform::Noise { octaves: 2 })
            .with_frequency(0.4)
            .with_amplitude(Vec3::new(
                1.5_f32.to_radians(),
                1.5_f32.to_radians(),
                0.5_f32.to_radians(),
            ))
    }

    /// A fast, small shake on every axis, for the hands.
    pub fn tremor(mask: AssetHandle<BoneMask>) -> Self {
        Self::new(mask)
            .with_waveform(Waveform::Noise { octaves: 2 })
            .with_frequency(9.0)
            .with_amplitude(Vec3::splat(0.3_f32.to_radians()))
    }

    /// Sets how the motion varies over time.
    pub fn with_waveform(mut self, waveform: Waveform) -> Self {
        self.waveform = waveform;
        self
    }

    /// Sets how fast the motion goes, in hertz.
    pub fn with_frequency(mut self, frequency: f32) -> Self {
        self.frequency = frequency;
        self
    }

    /// Sets the peak rotation in radians about each model axis.
    pub fn with_amplitude(mut self, amplitude: Vec3) -> Self {
        self.amplitude = amplitude;
        self
    }

    /// Scales the motion by the `f32` blackboard param named `param`.
    pub fn with_magnitude_param(mut self, param: impl Into<Param<f32>>) -> Self {
        self.magnitude_param = Some(param.into());
        self
    }

    /// Sets the halflife in seconds over which the magnitude eases to the param's value.
    pub fn with_magnitude_halflife(mut self, halflife: f32) -> Self {
        self.magnitude_halflife = halflife;
        self
    }

    fn goal_magnitude(&self, blackboard: &AnimationBlackboard) -> f32 {
        let magnitude = self
            .magnitude_param
            .and_then(|param| blackboard.read(&param).copied())
            .unwrap_or(1.0);
        if magnitude.is_finite() && magnitude > 0.0 {
            magnitude
        } else {
            0.0
        }
    }

    fn halflife(&self) -> f32 {
        if self.magnitude_halflife.is_finite() && self.magnitude_halflife > 0.0 {
            self.magnitude_halflife
        } else {
            0.0
        }
    }

    fn moves(&self) -> bool {
        self.frequency.is_finite() && self.frequency > 0.0
    }

    fn amplitude(&self) -> Vec3 {
        let finite = |value: f32| if value.is_finite() { value } else { 0.0 };
        Vec3::new(
            finite(self.amplitude.x),
            finite(self.amplitude.y),
            finite(self.amplitude.z),
        )
    }
}

/// How a channel's motion varies over time.
#[derive(Clone, Copy, Debug, PartialEq, Reflect, Serialize, Deserialize)]
pub enum Waveform {
    /// A sine, the same on every axis: breathing.
    Sine,
    /// Fractal gradient noise, its own on each axis: sway and tremor.
    Noise { octaves: u32 },
}

impl Default for Waveform {
    fn default() -> Self {
        Waveform::Noise { octaves: 2 }
    }
}

impl Waveform {
    fn sample(self, stream: u64, phase: f64) -> Vec3 {
        match self {
            Waveform::Sine => {
                let offset = unit_from_seed(stream) as f64;
                Vec3::splat((TAU * (phase + offset)).sin() as f32)
            }
            Waveform::Noise { octaves } => {
                let axis = |axis: u64| fractal_noise(mix_seed(stream, axis + 1), phase, octaves);
                Vec3::new(axis(0), axis(1), axis(2))
            }
        }
    }
}

/// The time since the last evaluate, each channel's phase and resolved mask, and the per-bone rotations a [`ProceduralMotionNode`] reuses.
#[derive(Default)]
pub struct ProceduralMotionState {
    elapsed: f32,
    started: bool,
    channels: Vec<ChannelState>,
    offsets: Vec<Option<Quat>>,
    input_model: Vec<Quat>,
    output_model: Vec<Quat>,
}

struct ChannelState {
    phase: f64,
    fresh: bool,
    magnitude: Spring<f32>,
    mask: MaskResolution,
    warned_missing: bool,
}

impl ChannelState {
    fn new() -> Self {
        Self {
            phase: 0.0,
            fresh: true,
            magnitude: Spring::critically_damped_with_halflife(0.0, 0.0),
            mask: MaskResolution::default(),
            warned_missing: false,
        }
    }

    fn ease(&mut self, channel: &ProceduralMotionChannel, goal: f32, elapsed: f32) -> f32 {
        if self.fresh {
            self.fresh = false;
            self.magnitude.reset_to(goal);
            return goal;
        }
        self.magnitude.set_halflife(channel.halflife());
        let eased = self.magnitude.update(goal, elapsed);
        if goal == 0.0 && eased < 1e-4 {
            self.magnitude.reset_to(0.0);
            return 0.0;
        }
        eased
    }
}

impl ProceduralMotionState {
    fn keep_up(&mut self, node: &ProceduralMotionNode) {
        self.channels
            .resize_with(node.channels.len(), ChannelState::new);
    }

    fn take_elapsed(&mut self) -> f32 {
        let elapsed = std::mem::take(&mut self.elapsed);
        if self.started {
            elapsed
        } else {
            self.started = true;
            0.0
        }
    }

    /// Adds `delta_time` to the time to ease over and advances each channel's phase by it times the channel's frequency.
    fn advance(&mut self, node: &ProceduralMotionNode, delta_time: f32) {
        self.keep_up(node);
        if !(delta_time.is_finite() && delta_time > 0.0) {
            return;
        }
        self.elapsed += delta_time;
        for (channel, state) in node.channels.iter().zip(&mut self.channels) {
            if channel.moves() {
                state.phase += delta_time as f64 * channel.frequency as f64;
            }
        }
    }
}

fn parent_below(skeleton: &Skeleton, bone: usize) -> Option<usize> {
    parent(skeleton, bone).filter(|parent| *parent < bone)
}

impl ProceduralMotionNode {
    fn gather_offsets(
        &self,
        state: &mut ProceduralMotionState,
        masks: &AssetStore<BoneMask>,
        context: &AnimationGraphContext<'_>,
        skeleton: &Skeleton,
        elapsed: f32,
    ) -> bool {
        state.offsets.clear();
        state.offsets.resize(skeleton.bone_count(), None);
        let base = mix_seed(self.param_seed(context.blackboard()), self.seed);
        let mut contributed = false;
        for (index, (channel, channel_state)) in
            self.channels.iter().zip(&mut state.channels).enumerate()
        {
            let goal = channel.goal_magnitude(context.blackboard());
            let magnitude = channel_state.ease(channel, goal, elapsed);
            if magnitude <= 0.0 || !channel.moves() {
                continue;
            }
            let Some(mask) = masks.get(&channel.mask) else {
                continue;
            };
            resolve_mask(mask, skeleton, None, &mut channel_state.mask);
            if !channel_state.mask.missing.is_empty() && !channel_state.warned_missing {
                let names: Vec<&str> = channel_state
                    .mask
                    .missing
                    .iter()
                    .map(|name| &**name)
                    .collect();
                warn!(
                    "procedural motion mask {:?} names bones the skeleton lacks: {}",
                    channel.mask.id(),
                    names.join(", ")
                );
                channel_state.warned_missing = true;
            }
            let stream = mix_seed(base, index as u64);
            let wave = channel.waveform.sample(stream, channel_state.phase);
            let rotation = magnitude * channel.amplitude() * wave;
            if rotation == Vec3::ZERO || !rotation.is_finite() {
                continue;
            }
            for (offset, weight) in state.offsets.iter_mut().zip(&channel_state.mask.weights) {
                if *weight <= 0.0 {
                    continue;
                }
                let turn = Quat::from_scaled_axis(*weight * rotation);
                *offset = Some(turn * offset.unwrap_or(Quat::IDENTITY));
                contributed = true;
            }
        }
        contributed
    }
}

fn apply_offsets(
    state: &mut ProceduralMotionState,
    skeleton: &Skeleton,
    output: &mut AnimationFrame,
) {
    let bones = state.offsets.len().min(output.len());
    state.input_model.clear();
    state.output_model.clear();
    for bone in 0..bones {
        let up = parent_below(skeleton, bone);
        let input_parent = up.map_or(Quat::IDENTITY, |up| state.input_model[up]);
        let output_parent = up.map_or(Quat::IDENTITY, |up| state.output_model[up]);
        let (input_model, output_model) = match output.joint_mut(bone) {
            Some(joint) => {
                let input_model = input_parent * joint.rotation;
                let output_model = match state.offsets[bone] {
                    Some(offset) => {
                        let turned = offset * input_model;
                        joint.rotation = (output_parent.inverse() * turned).normalize();
                        turned
                    }
                    None => output_parent * joint.rotation,
                };
                (input_model, output_model)
            }
            None => (input_parent, output_parent),
        };
        state.input_model.push(input_model);
        state.output_model.push(output_model);
    }
}

impl AnimationNode for ProceduralMotionNode {
    type State = ProceduralMotionState;
    type CreateInput = ();
    type UpdateInput = ();
    type EvaluateInput = Res<'static, AssetStore<BoneMask>>;

    fn input_slots(&self) -> InputSlots {
        InputSlots::Fixed(vec![name!("pose")])
    }

    fn create_state(&self, _: &(), _context: &AnimationGraphContext<'_>) -> ProceduralMotionState {
        let mut state = ProceduralMotionState::default();
        state.keep_up(self);
        state
    }

    fn update(
        &self,
        state: &mut ProceduralMotionState,
        delta_time: f32,
        _: &(),
        _: &AnimationGraphContext<'_>,
    ) {
        state.advance(self, delta_time);
    }

    fn evaluate(
        &self,
        state: &mut ProceduralMotionState,
        masks: &Res<AssetStore<BoneMask>>,
        context: &AnimationGraphContext<'_>,
        _: &[Uuid],
        evaluated_inputs: &[EvaluatedFrame],
        _: &mut FramePool,
        output: &mut AnimationFrame,
    ) {
        let elapsed = state.take_elapsed();
        state.keep_up(self);
        let Some(input) = evaluated_inputs.first() else {
            return;
        };
        output.copy_from(&input.frame);
        let Some(skeleton) = context.skeleton() else {
            return;
        };
        if self.gather_offsets(state, masks, context, skeleton, elapsed) {
            apply_offsets(state, skeleton, output);
        }
    }

    fn reset(&self, state: &mut ProceduralMotionState) {
        state.elapsed = 0.0;
        state.started = false;
        for channel in &mut state.channels {
            channel.fresh = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::TAU;

    use concerto_animation::{
        evaluation::AnimationGraphContext,
        mask::{BoneMask, BoneMaskBranch},
        node::{AnimationNode, InputSlots, ReflectAnimationNode},
        player::AnimationPlayer,
        pose::{AnimationFrame, EvaluatedFrame, FramePool, JointPose},
        root::AnimationRootBone,
    };
    use concerto_app::{App, schedule_groups::LateUpdate};
    use concerto_ecs::{Entity, world::World};
    use concerto_foundation::assets::{AssetId, handle::AssetHandle};
    use concerto_mesh::skeleton::Skeleton;
    use concerto_name::{Name, name};
    use concerto_noise::{mix_seed, unit_from_seed};
    use concerto_reflect::Reflect;
    use glam::{Quat, Vec3};
    use uuid::Uuid;

    use super::{ProceduralMotionChannel, ProceduralMotionNode, Waveform};
    use crate::testing::{
        Bone, Frame, app_with, bone, captured, captured_graph, compile, input_event, insert_asset,
        read_state, skeleton_of, spawn_rigged, write_state,
    };

    const HIPS: usize = 0;
    const SPINE: usize = 1;
    const CHEST: usize = 2;
    const NECK: usize = 3;
    const HEAD: usize = 4;
    const ARM: usize = 5;
    const HAND: usize = 6;
    const PROP: usize = 7;
    const ABOVE_CHEST: [usize; 5] = [CHEST, NECK, HEAD, ARM, HAND];
    const TOLERANCE: f32 = 1e-5;

    fn rig() -> Vec<Bone> {
        vec![
            bone("hips", None, Vec3::Y, Quat::from_rotation_y(0.4)),
            bone(
                "spine",
                Some(0),
                Vec3::new(0.0, 0.2, 0.0),
                Quat::from_rotation_x(0.1),
            ),
            bone(
                "chest",
                Some(1),
                Vec3::new(0.0, 0.25, 0.0),
                Quat::from_euler(glam::EulerRot::XYZ, 0.3, -0.5, 0.7),
            ),
            bone(
                "neck",
                Some(2),
                Vec3::new(0.0, 0.3, 0.0),
                Quat::from_rotation_z(0.15),
            ),
            bone(
                "head",
                Some(3),
                Vec3::new(0.0, 0.15, 0.0),
                Quat::from_rotation_y(-0.2),
            ),
            bone(
                "arm",
                Some(2),
                Vec3::new(0.2, 0.2, 0.0),
                Quat::from_rotation_z(-1.2),
            ),
            bone(
                "hand",
                Some(5),
                Vec3::new(0.3, 0.0, 0.0),
                Quat::from_rotation_x(0.25),
            ),
            bone("prop", None, Vec3::X, Quat::from_rotation_x(0.3)),
        ]
    }

    fn skeleton() -> Skeleton {
        skeleton_of(&rig())
    }

    fn input_joints() -> Vec<JointPose> {
        rig()
            .into_iter()
            .enumerate()
            .map(|(index, bone)| JointPose {
                translation: bone.joint.translation + Vec3::new(0.01, 0.02, 0.03) * index as f32,
                rotation: (Quat::from_rotation_y(0.05 * index as f32) * bone.joint.rotation)
                    .normalize(),
                scale: Vec3::splat(1.0 + 0.1 * index as f32),
            })
            .collect()
    }

    #[derive(Default, Reflect)]
    #[reflect(AnimationNode)]
    struct InputPose {}

    impl AnimationNode for InputPose {
        type State = ();
        type CreateInput = ();
        type UpdateInput = ();
        type EvaluateInput = ();

        fn input_slots(&self) -> InputSlots {
            InputSlots::None
        }

        fn create_state(&self, _: &(), _context: &AnimationGraphContext<'_>) {}

        fn update(&self, _: &mut (), _: f32, _: &(), _: &AnimationGraphContext<'_>) {}

        fn evaluate(
            &self,
            _: &mut (),
            _: &(),
            _: &AnimationGraphContext<'_>,
            _: &[Uuid],
            _: &[EvaluatedFrame],
            _: &mut FramePool,
            output: &mut AnimationFrame,
        ) {
            for (bone, input) in input_joints().into_iter().enumerate() {
                if let Some(joint) = output.joint_mut(bone) {
                    *joint = input;
                }
            }
            output.push_event(input_event());
        }

        fn reset(&self, _: &mut ()) {}
    }

    struct NamedMask {
        path: &'static str,
        mask: BoneMask,
    }

    fn masks() -> Vec<NamedMask> {
        let named = |path, mask| NamedMask { path, mask };
        vec![
            named(
                "chest",
                BoneMask::new().with_branch(BoneMaskBranch::new("chest")),
            ),
            named(
                "chest_only",
                BoneMask::new().with_branch(BoneMaskBranch::new("chest").with_descendants(false)),
            ),
            named(
                "spine_ramp",
                BoneMask::new().with_branch(BoneMaskBranch::new("spine").with_depth(2)),
            ),
            named(
                "hips_only",
                BoneMask::new().with_branch(BoneMaskBranch::new("hips").with_descendants(false)),
            ),
            named(
                "prop",
                BoneMask::new().with_branch(BoneMaskBranch::new("prop")),
            ),
            named(
                "chest_and_tail",
                BoneMask::new()
                    .with_branch(BoneMaskBranch::new("chest").with_descendants(false))
                    .with_branch(BoneMaskBranch::new("tail")),
            ),
        ]
    }

    fn mask(path: &str) -> AssetHandle<BoneMask> {
        AssetHandle::weak(AssetId::from_path(path))
    }

    struct Stage {
        app: App,
        players: Vec<Entity>,
    }

    impl Stage {
        fn new(node: ProceduralMotionNode, seeds: &[u64]) -> Self {
            let mut app = app_with::<InputPose>();
            let bone_count = skeleton().bone_count();
            let world = app.main_mut().world_mut();
            for named in masks() {
                insert_asset(world, AssetId::from_path(named.path), named.mask);
            }
            let skeleton = insert_asset(world, AssetId::new(), skeleton());
            let graph = insert_asset(world, AssetId::new(), captured_graph(node, InputPose {}));
            let players: Vec<Entity> = seeds
                .iter()
                .map(|seed| {
                    let mut player = AnimationPlayer::new(bone_count);
                    player.set_paused(true);
                    player.set_param(name!("seed"), *seed);
                    let rigged = spawn_rigged(world, &skeleton, &graph, player, bone_count);
                    world.insert(AnimationRootBone::default(), rigged.bones[HIPS]);
                    rigged.player
                })
                .collect();
            compile(world);
            world.run_schedule(LateUpdate);
            for player in &players {
                world
                    .get_component_for_entity_mut::<AnimationPlayer>(*player)
                    .unwrap()
                    .set_paused(false);
            }

            Self { app, players }
        }

        fn one(node: ProceduralMotionNode) -> Self {
            Self::new(node, &[0])
        }

        fn world(&mut self) -> &mut World {
            self.app.main_mut().world_mut()
        }

        fn magnitude(&mut self, channel: usize) -> f32 {
            let player = self.players[0];
            read_state::<ProceduralMotionNode, _>(self.world(), player, |_, state| {
                state.channels[channel].magnitude.value
            })
            .expect("the graph holds a procedural motion node")
        }

        fn advance(&mut self, delta_time: f32) {
            for player in self.players.clone() {
                write_state::<ProceduralMotionNode, _>(self.world(), player, |node, state| {
                    state.advance(node, delta_time)
                })
                .expect("the graph holds a procedural motion node");
            }
        }

        fn set_param<T: Reflect + Clone>(&mut self, key: Name, value: T) {
            for player in self.players.clone() {
                self.world()
                    .get_component_for_entity_mut::<AnimationPlayer>(player)
                    .unwrap()
                    .set_param(key, value.clone());
            }
        }

        fn step_all(&mut self, delta_time: f32) -> Vec<Frame> {
            self.advance(delta_time);
            self.world().run_schedule(LateUpdate);
            self.players
                .clone()
                .into_iter()
                .map(|player| captured(self.world(), player))
                .collect()
        }

        fn step(&mut self, delta_time: f32) -> Frame {
            self.step_all(delta_time).remove(0)
        }

        fn run(&mut self, steps: usize, delta_time: f32) -> Frame {
            let mut frame = self.step(delta_time);
            for _ in 1..steps {
                frame = self.step(delta_time);
            }
            frame
        }
    }

    fn model_rotation(joints: &[JointPose], bone: usize) -> Quat {
        let parents: Vec<Option<u16>> = rig().iter().map(|bone| bone.parent).collect();
        let mut rotation = joints[bone].rotation;
        let mut current = bone;
        while let Some(up) = parents[current] {
            current = up as usize;
            rotation = joints[current].rotation * rotation;
        }
        rotation
    }

    fn assert_quat(actual: Quat, expected: Quat) {
        let error = (actual - expected)
            .length()
            .min((actual + expected).length());
        assert!(
            error < TOLERANCE,
            "expected rotation {expected}, got {actual}"
        );
    }

    fn assert_turned(frame: &Frame, bone: usize, turn: Quat) {
        assert_quat(
            model_rotation(&frame.joints, bone),
            turn * model_rotation(&input_joints(), bone),
        );
    }

    fn assert_input(frame: &Frame) {
        assert_eq!(frame.joints, input_joints());
        assert_eq!(frame.events, [input_event()]);
    }

    fn named(name: &str, check: impl FnOnce()) {
        let checked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(check));
        assert!(checked.is_ok(), "{name}");
    }

    fn sine_value(player_seed: u64, node_seed: u64, channel: u64, phase: f32) -> f32 {
        let offset = unit_from_seed(mix_seed(mix_seed(player_seed, node_seed), channel));
        (TAU * (phase + offset)).sin()
    }

    fn sine(path: &str, frequency: f32, amplitude: Vec3) -> ProceduralMotionChannel {
        ProceduralMotionChannel::new(mask(path))
            .with_waveform(Waveform::Sine)
            .with_frequency(frequency)
            .with_amplitude(amplitude)
    }

    fn noise(path: &str) -> ProceduralMotionChannel {
        ProceduralMotionChannel::new(mask(path))
            .with_waveform(Waveform::Noise { octaves: 2 })
            .with_frequency(0.7)
            .with_amplitude(Vec3::new(0.2, 0.15, 0.1))
    }

    fn single(channel: ProceduralMotionChannel) -> ProceduralMotionNode {
        ProceduralMotionNode::new().with_channel(channel)
    }

    fn breathing() -> ProceduralMotionChannel {
        sine("chest_only", 0.25, Vec3::new(0.1, 0.0, 0.0))
    }

    fn breathing_turn(scale: f32) -> Quat {
        Quat::from_rotation_x(scale * 0.1 * sine_value(0, 0, 0, 0.25 * 1.3))
    }

    struct Turned {
        bone: usize,
        turn: Quat,
    }

    struct Motion {
        name: &'static str,
        node: ProceduralMotionNode,
        turned: Vec<Turned>,
        local: &'static [usize],
        untouched: &'static [usize],
    }

    #[test]
    fn each_masked_bone_turns_about_model_axes_by_its_channels_scaled_by_its_mask_weight() {
        let at = |amplitude: Vec3, channel: u64, frequency: f32| {
            Quat::from_scaled_axis(amplitude * sine_value(0, 0, channel, frequency * 1.3))
        };
        let turned = |bones: &[usize], turn: Quat| {
            bones
                .iter()
                .map(|bone| Turned { bone: *bone, turn })
                .collect::<Vec<Turned>>()
        };
        let ramp = Vec3::new(0.1, 0.05, 0.02);
        let rows = [
            Motion {
                name: "breathing pitches the chest and everything above it",
                node: single(sine("chest", 0.25, Vec3::new(0.1, 0.0, 0.0))),
                turned: turned(&ABOVE_CHEST, breathing_turn(1.0)),
                local: &[NECK, HEAD, ARM, HAND],
                untouched: &[HIPS, SPINE, PROP],
            },
            Motion {
                name: "a branch without descendants turns its bone and its children follow",
                node: single(breathing()),
                turned: turned(&ABOVE_CHEST, breathing_turn(1.0)),
                local: &[NECK, HEAD, ARM, HAND],
                untouched: &[HIPS, SPINE, PROP],
            },
            Motion {
                name: "the amplitude turns about model axes whatever the bone's local axes",
                node: single(sine("chest_only", 0.25, Vec3::new(0.0, 0.1, 0.0))),
                turned: turned(&[CHEST], at(Vec3::new(0.0, 0.1, 0.0), 0, 0.25)),
                local: &[],
                untouched: &[HIPS, SPINE, PROP],
            },
            Motion {
                name: "each bone turns by its own mask weight",
                node: single(sine("spine_ramp", 0.25, ramp)),
                turned: [SPINE, CHEST, NECK, HEAD, ARM, HAND]
                    .into_iter()
                    .zip([1.0 / 3.0, 2.0 / 3.0, 1.0, 1.0, 1.0, 1.0])
                    .map(|(bone, weight)| Turned {
                        bone,
                        turn: at(ramp * weight, 0, 0.25),
                    })
                    .collect(),
                local: &[],
                untouched: &[HIPS, PROP],
            },
            Motion {
                name: "a mask on the root bone turns its joint",
                node: single(sine("hips_only", 0.25, Vec3::new(0.1, 0.0, 0.0))),
                turned: turned(&[HIPS], breathing_turn(1.0)),
                local: &[SPINE],
                untouched: &[PROP],
            },
            Motion {
                name: "later channels compose on the left",
                node: ProceduralMotionNode::new()
                    .with_channel(sine("chest_only", 0.25, Vec3::new(0.1, 0.0, 0.0)))
                    .with_channel(sine("chest_only", 0.4, Vec3::new(0.0, 0.1, 0.05))),
                turned: turned(
                    &[CHEST],
                    at(Vec3::new(0.0, 0.1, 0.05), 1, 0.4) * at(Vec3::new(0.1, 0.0, 0.0), 0, 0.25),
                ),
                local: &[],
                untouched: &[HIPS, SPINE, PROP],
            },
        ];

        for row in rows {
            let frame = Stage::one(row.node).step(1.3);

            named(row.name, || {
                let input = input_joints();
                for turned in &row.turned {
                    assert!(turned.turn.angle_between(Quat::IDENTITY) > 0.005);
                    assert_turned(&frame, turned.bone, turned.turn);
                }
                for bone in row.local {
                    assert_quat(frame.joints[*bone].rotation, input[*bone].rotation);
                }
                for bone in row.untouched {
                    assert_eq!(frame.joints[*bone], input[*bone]);
                }
            });
        }
    }

    #[test]
    fn the_magnitude_param_scales_the_turn() {
        for scale in [0.5_f32, 2.0] {
            let mut stage = Stage::one(single(breathing().with_magnitude_param(name!("strength"))));
            stage.set_param(name!("strength"), scale);

            let frame = stage.step(1.3);

            assert_turned(&frame, CHEST, breathing_turn(scale));
        }
    }

    #[test]
    fn two_frame_rates_reach_the_same_pose_at_the_same_time() {
        let node = ProceduralMotionNode::new()
            .with_channel(noise("chest_only"))
            .with_channel(sine("prop", 0.6, Vec3::new(0.1, 0.2, 0.0)));

        let fast = Stage::one(node.clone()).run(60, 1.0 / 60.0);
        let slow = Stage::one(node).run(30, 1.0 / 30.0);

        assert_ne!(fast.joints[CHEST], input_joints()[CHEST]);
        assert_ne!(fast.joints[PROP], input_joints()[PROP]);
        for bone in 0..fast.joints.len() {
            assert_quat(fast.joints[bone].rotation, slow.joints[bone].rotation);
        }
    }

    #[test]
    fn events_translations_scales_and_unmasked_bones_pass_through() {
        let frame = Stage::one(single(noise("chest"))).run(10, 0.1);

        let input = input_joints();
        assert_ne!(frame.joints[CHEST].rotation, input[CHEST].rotation);
        assert_eq!(frame.events, [input_event()]);
        for (joint, input) in frame.joints.iter().zip(&input) {
            assert_eq!(joint.translation, input.translation);
            assert_eq!(joint.scale, input.scale);
        }
        for bone in [HIPS, SPINE, PROP] {
            assert_eq!(frame.joints[bone], input[bone]);
        }
    }

    fn eased(halflife: f32) -> ProceduralMotionNode {
        single(
            breathing()
                .with_magnitude_param(name!("strength"))
                .with_magnitude_halflife(halflife),
        )
    }

    #[test]
    fn the_magnitude_eases_toward_the_param_over_its_halflife_and_back_to_the_input_pose() {
        let mut stage = Stage::one(eased(0.2));
        stage.set_param(name!("strength"), 0.0_f32);
        assert_input(&stage.step(0.1));
        assert_eq!(stage.magnitude(0), 0.0);

        stage.set_param(name!("strength"), 1.0_f32);
        stage.step(0.2);
        let magnitude = stage.magnitude(0);
        assert!(
            (magnitude - 0.5).abs() < 0.01,
            "expected about half the goal one halflife later, got {magnitude}"
        );

        stage.run(5, 0.2);
        stage.set_param(name!("strength"), 0.0_f32);
        let mut settled = false;
        for _ in 0..100 {
            let frame = stage.step(0.2);
            let magnitude = stage.magnitude(0);
            if magnitude == 0.0 {
                assert_input(&frame);
                settled = true;
                break;
            }
            assert!(
                magnitude >= 1e-4,
                "a magnitude of {magnitude} was not snapped"
            );
            assert_ne!(frame.joints[CHEST], input_joints()[CHEST]);
        }
        assert!(settled, "the magnitude never reached 0");
    }
}
