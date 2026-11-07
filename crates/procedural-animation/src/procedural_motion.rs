use std::f64::consts::TAU;

use concerto_animation::{
    blackboard::AnimationBlackboard,
    evaluation::AnimationGraphContext,
    mask::{BoneMask, MaskResolution, resolve_mask},
    node::{AnimationNode, ReflectAnimationNode, model_space::parent},
    pose::{AnimationFrame, EvaluatedFrame, FramePool},
};
use concerto_ecs::Res;
use concerto_foundation::assets::{asset_store::AssetStore, handle::AssetHandle};
use concerto_mesh::Skeleton;
use concerto_name::Name;
use concerto_noise::{fractal_noise, mix_seed, unit_from_seed};
use concerto_reflect::Reflect;
use glam::{Quat, Vec3};
use log::warn;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Adds rotations that wave or wander over time to the bones of each channel's mask; passes its first input through first.
#[derive(Clone, Debug, Default, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(AnimationNode)]
pub struct ProceduralMotionNode {
    pub channels: Vec<ProceduralMotionChannel>,
    pub seed: u64,
}

impl ProceduralMotionNode {
    /// A node with no channels and seed 0.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one channel after the others.
    pub fn with_channel(mut self, channel: ProceduralMotionChannel) -> Self {
        self.channels.push(channel);
        self
    }

    /// Sets the seed mixed with the player's to pick each channel's phase and noise.
    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }
}

/// One motion: the bones it moves, its waveform and frequency in hertz, its peak rotation in radians about each model axis, and the param that scales it.
#[derive(Clone, Debug, PartialEq, Reflect, Serialize, Deserialize)]
pub struct ProceduralMotionChannel {
    pub mask: AssetHandle<BoneMask>,
    pub waveform: Waveform,
    pub frequency: f32,
    pub amplitude: Vec3,
    pub magnitude_param: Option<Name>,
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
    pub fn with_magnitude_param(mut self, param: Name) -> Self {
        self.magnitude_param = Some(param);
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
            .and_then(|param| blackboard.get_float(param))
            .unwrap_or(1.0);
        if magnitude.is_finite() && magnitude > 0.0 {
            magnitude
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
    channels: Vec<ChannelState>,
    offsets: Vec<Option<Quat>>,
    input_model: Vec<Quat>,
    output_model: Vec<Quat>,
}

#[derive(Default)]
struct ChannelState {
    phase: f64,
    mask: MaskResolution,
    warned_missing: bool,
}

impl ProceduralMotionState {
    fn keep_up(&mut self, node: &ProceduralMotionNode) {
        self.channels
            .resize_with(node.channels.len(), ChannelState::default);
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
    ) -> bool {
        state.offsets.clear();
        state.offsets.resize(skeleton.bone_count(), None);
        let base = mix_seed(context.seed(), self.seed);
        let mut contributed = false;
        for (index, (channel, channel_state)) in
            self.channels.iter().zip(&mut state.channels).enumerate()
        {
            let magnitude = channel.goal_magnitude(context.blackboard());
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
        state.elapsed = 0.0;
        state.keep_up(self);
        let Some(input) = evaluated_inputs.first() else {
            return;
        };
        output.copy_from(&input.frame);
        let Some(skeleton) = context.skeleton() else {
            return;
        };
        if self.gather_offsets(state, masks, context, skeleton) {
            apply_offsets(state, skeleton, output);
        }
    }

    fn reset(&self, state: &mut ProceduralMotionState) {
        state.elapsed = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::TAU;

    use concerto_animation::{
        evaluation::AnimationGraphContext,
        graph::AnimationGraph,
        mask::{BoneMask, BoneMaskBranch},
        node::{AnimationNode, ReflectAnimationNode},
        player::{AnimationHandleComponent, AnimationPlayer},
        plugin::{AnimationAppExt, AnimationPlugin, register_animation_types},
        pose::{AnimationFrame, EvaluatedFrame, FiredEvent, FramePool, JointPose, RootMotion},
        root::AnimationRootBone,
    };
    use concerto_app::{App, plugins::AssetManagerPlugin, schedule_groups::LateUpdate};
    use concerto_ecs::{
        Entity,
        component::scene::SceneEntityRef,
        system::{executor::single_thread::SingleThreadedExecutor, schedule::Schedules},
        world::World,
    };
    use concerto_foundation::{
        assets::{AssetId, asset_store::AssetStore, handle::AssetHandle},
        time::Time,
        transform::Transform,
    };
    use concerto_mesh::skeleton::{Skeleton, SkeletonComponent};
    use concerto_name::{Name, name};
    use concerto_noise::{mix_seed, unit_from_seed};
    use concerto_reflect::{Reflect, TypeRegistry};
    use glam::{Mat4, Quat, Vec3};
    use uuid::Uuid;

    use super::{ProceduralMotionChannel, ProceduralMotionNode, Waveform};
    use crate::plugin::ProceduralAnimationPlugin;

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

    struct RigBone {
        name: &'static str,
        parent: Option<u16>,
        joint: JointPose,
    }

    fn rig_bone(
        name: &'static str,
        parent: Option<u16>,
        translation: Vec3,
        rotation: Quat,
    ) -> RigBone {
        RigBone {
            name,
            parent,
            joint: JointPose {
                translation,
                rotation,
                scale: Vec3::ONE,
            },
        }
    }

    fn rig() -> Vec<RigBone> {
        vec![
            rig_bone("hips", None, Vec3::Y, Quat::from_rotation_y(0.4)),
            rig_bone(
                "spine",
                Some(0),
                Vec3::new(0.0, 0.2, 0.0),
                Quat::from_rotation_x(0.1),
            ),
            rig_bone(
                "chest",
                Some(1),
                Vec3::new(0.0, 0.25, 0.0),
                Quat::from_euler(glam::EulerRot::XYZ, 0.3, -0.5, 0.7),
            ),
            rig_bone(
                "neck",
                Some(2),
                Vec3::new(0.0, 0.3, 0.0),
                Quat::from_rotation_z(0.15),
            ),
            rig_bone(
                "head",
                Some(3),
                Vec3::new(0.0, 0.15, 0.0),
                Quat::from_rotation_y(-0.2),
            ),
            rig_bone(
                "arm",
                Some(2),
                Vec3::new(0.2, 0.2, 0.0),
                Quat::from_rotation_z(-1.2),
            ),
            rig_bone(
                "hand",
                Some(5),
                Vec3::new(0.3, 0.0, 0.0),
                Quat::from_rotation_x(0.25),
            ),
            rig_bone("prop", None, Vec3::X, Quat::from_rotation_x(0.3)),
        ]
    }

    fn skeleton() -> Skeleton {
        let bones = rig();
        Skeleton::new(
            vec![Mat4::IDENTITY; bones.len()],
            bones.iter().map(|bone| bone.parent).collect(),
            bones.iter().map(|bone| bone.name.into()).collect(),
            bones.iter().map(|bone| bone.joint.clone()).collect(),
            (0..bones.len() as u128).map(Uuid::from_u128).collect(),
        )
        .unwrap()
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

    fn input_motion() -> RootMotion {
        RootMotion {
            translation: Vec3::new(0.1, 0.0, 0.3),
            rotation: Quat::from_rotation_y(0.05),
        }
    }

    fn input_event() -> FiredEvent {
        FiredEvent {
            clip: AssetId::from_path("motion_input"),
            key: 1,
            time: 0.3,
            weight: 0.5,
            mirrored: false,
        }
    }

    #[derive(Default, Reflect)]
    #[reflect(AnimationNode)]
    struct InputPose {}

    impl AnimationNode for InputPose {
        type State = ();
        type CreateInput = ();
        type UpdateInput = ();
        type EvaluateInput = ();

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
            output.set_root_motion(input_motion());
            output.push_event(input_event());
        }

        fn reset(&self, _: &mut ()) {}
    }

    #[derive(Clone, Debug, PartialEq)]
    struct Frame {
        joints: Vec<JointPose>,
        root_motion: RootMotion,
        events: Vec<FiredEvent>,
    }

    #[derive(Default, Reflect)]
    #[reflect(AnimationNode)]
    struct Capture {}

    impl AnimationNode for Capture {
        type State = Option<Frame>;
        type CreateInput = ();
        type UpdateInput = ();
        type EvaluateInput = ();

        fn create_state(&self, _: &(), _context: &AnimationGraphContext<'_>) -> Option<Frame> {
            None
        }

        fn update(&self, _: &mut Option<Frame>, _: f32, _: &(), _: &AnimationGraphContext<'_>) {}

        fn evaluate(
            &self,
            state: &mut Option<Frame>,
            _: &(),
            _: &AnimationGraphContext<'_>,
            _: &[Uuid],
            evaluated_inputs: &[EvaluatedFrame],
            _: &mut FramePool,
            output: &mut AnimationFrame,
        ) {
            let Some(input) = evaluated_inputs.first() else {
                return;
            };
            output.copy_from(&input.frame);
            *state = Some(Frame {
                joints: input.frame.joints().to_vec(),
                root_motion: input.frame.root_motion(),
                events: input.frame.events().to_vec(),
            });
        }

        fn reset(&self, state: &mut Option<Frame>) {
            *state = None;
        }
    }

    fn graph_of(node: ProceduralMotionNode) -> AnimationGraph {
        let mut registry = TypeRegistry::default();
        register_animation_types(&mut registry);
        registry.register::<ProceduralMotionNode>();
        registry.register::<InputPose>();
        registry.register::<Capture>();
        let mut graph = AnimationGraph::new();
        graph.result_node().with_input(Capture {}, |mut capture| {
            capture.with_input(node, |mut motion| {
                motion.with_input(InputPose {}, |_| {});
            });
        });
        graph.make_live(&registry, None).unwrap();
        graph
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
        node: ProceduralMotionNode,
        players: Vec<Entity>,
    }

    impl Stage {
        fn new(node: ProceduralMotionNode, seeds: &[u64]) -> Self {
            let mut app = App::new();
            app.register_plugin(AssetManagerPlugin);
            app.insert_resource(Time::new());
            app.register_plugin(AnimationPlugin);
            app.register_plugin(ProceduralAnimationPlugin);
            app.register_animation_node::<InputPose>();
            app.register_animation_node::<Capture>();

            let bone_count = skeleton().bone_count();
            let world = app.main_mut().world_mut();
            world.register_component::<Transform>();
            for named in masks() {
                world
                    .get_resource_mut::<AssetStore<BoneMask>>()
                    .unwrap()
                    .insert(AssetId::from_path(named.path), named.mask);
            }
            let skeleton_handle = AssetHandle::<Skeleton>::weak(AssetId::new());
            world
                .get_resource_mut::<AssetStore<Skeleton>>()
                .unwrap()
                .insert(skeleton_handle.id(), skeleton());
            let graph_handle = AssetHandle::<AnimationGraph>::weak(AssetId::new());
            world
                .get_resource_mut::<AssetStore<AnimationGraph>>()
                .unwrap()
                .insert(graph_handle.id(), graph_of(node.clone()));
            let players = seeds
                .iter()
                .map(|seed| {
                    let bones: Vec<Entity> = (0..bone_count)
                        .map(|_| world.spawn(Transform::IDENTITY))
                        .collect();
                    world.insert(AnimationRootBone::default(), bones[HIPS]);
                    let mut player = AnimationPlayer::new(bone_count);
                    player.set_paused(true);
                    player.set_seed(*seed);
                    world.spawn((
                        player,
                        AnimationHandleComponent::new(graph_handle.clone()),
                        SkeletonComponent::new(
                            skeleton_handle.clone(),
                            bones
                                .iter()
                                .map(|bone| SceneEntityRef::Entity(*bone))
                                .collect(),
                            None,
                        ),
                    ))
                })
                .collect::<Vec<Entity>>();
            let schedules = world.remove_resource::<Schedules>().unwrap();
            let compiled = schedules.compile::<SingleThreadedExecutor>(world);
            world.insert_resource(compiled);
            world.run_schedule(LateUpdate);
            for player in &players {
                world
                    .get_component_for_entity_mut::<AnimationPlayer>(*player)
                    .unwrap()
                    .set_paused(false);
            }

            Self { app, node, players }
        }

        fn one(node: ProceduralMotionNode) -> Self {
            Self::new(node, &[0])
        }

        fn world(&mut self) -> &mut World {
            self.app.main_mut().world_mut()
        }

        fn set_param<T: Reflect + Clone>(&mut self, key: Name, value: T) {
            for player in self.players.clone() {
                self.world()
                    .get_component_for_entity_mut::<AnimationPlayer>(player)
                    .unwrap()
                    .set_param(key, value.clone());
            }
        }

        fn state_of<N: AnimationNode, R>(
            &mut self,
            player: Entity,
            read: impl FnOnce(&N, &mut N::State) -> R,
        ) -> Option<R> {
            let mut read = Some(read);
            let mut result = None;
            self.world()
                .resource_scope::<AssetStore<AnimationGraph>>(|world, graphs| {
                    world
                        .get_component_for_entity_mut::<AnimationPlayer>(player)
                        .unwrap()
                        .for_each_state_mut::<N>(graphs, |node, state, _| {
                            if let Some(read) = read.take() {
                                result = Some(read(node, state));
                            }
                        });
                });
            result
        }

        fn step_all(&mut self, delta_time: f32) -> Vec<Frame> {
            for player in self.players.clone() {
                self.state_of::<ProceduralMotionNode, _>(player, |node, state| {
                    state.advance(node, delta_time)
                })
                .expect("the graph holds a procedural motion node");
            }
            self.world().run_schedule(LateUpdate);
            self.players
                .clone()
                .into_iter()
                .map(|player| {
                    self.state_of::<Capture, _>(player, |_, state: &mut Option<Frame>| state.take())
                        .flatten()
                        .expect("the procedural motion node was evaluated")
                })
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
        assert_eq!(frame.root_motion, input_motion());
        assert_eq!(frame.events, [input_event()]);
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

    fn breathing_turn(scale: f32) -> Quat {
        Quat::from_rotation_x(scale * 0.1 * sine_value(0, 0, 0, 0.25 * 1.3))
    }

    #[test]
    fn breathing_pitches_the_chest_and_everything_above_it_about_model_x() {
        let node = single(sine("chest", 0.25, Vec3::new(0.1, 0.0, 0.0)));
        let frame = Stage::one(node).step(1.3);

        let turn = breathing_turn(1.0);
        assert!(turn.angle_between(Quat::IDENTITY) > 0.01);
        for bone in ABOVE_CHEST {
            assert_turned(&frame, bone, turn);
        }
        for bone in [NECK, HEAD, ARM, HAND] {
            assert_quat(frame.joints[bone].rotation, input_joints()[bone].rotation);
        }
        for bone in [HIPS, SPINE, PROP] {
            assert_eq!(frame.joints[bone], input_joints()[bone]);
        }
    }

    #[test]
    fn the_amplitude_turns_about_model_axes_whatever_the_bone_local_axes() {
        let node = single(sine("chest", 0.25, Vec3::new(0.0, 0.1, 0.0)));
        let frame = Stage::one(node).step(1.3);

        let turn = Quat::from_rotation_y(0.1 * sine_value(0, 0, 0, 0.25 * 1.3));
        assert!(turn.angle_between(Quat::IDENTITY) > 0.01);
        assert_turned(&frame, CHEST, turn);
    }

    #[test]
    fn each_bone_turns_by_its_own_mask_weight() {
        let amplitude = Vec3::new(0.1, 0.05, 0.02);
        let node = single(sine("spine_ramp", 0.25, amplitude));
        let frame = Stage::one(node).step(1.3);

        let rotation = amplitude * sine_value(0, 0, 0, 0.25 * 1.3);
        let bones = [SPINE, CHEST, NECK, HEAD, ARM, HAND];
        let weights = [1.0 / 3.0, 2.0 / 3.0, 1.0, 1.0, 1.0, 1.0];
        for (bone, weight) in bones.into_iter().zip(weights) {
            assert_turned(&frame, bone, Quat::from_scaled_axis(weight * rotation));
        }
        assert_eq!(frame.joints[HIPS], input_joints()[HIPS]);
    }

    #[test]
    fn a_branch_without_descendants_turns_its_bone_and_its_children_follow() {
        let node = single(sine("chest_only", 0.25, Vec3::new(0.1, 0.0, 0.0)));
        let frame = Stage::one(node).step(1.3);

        let turn = breathing_turn(1.0);
        for bone in ABOVE_CHEST {
            assert_turned(&frame, bone, turn);
        }
        for bone in [NECK, HEAD, ARM, HAND] {
            assert_eq!(frame.joints[bone].rotation, input_joints()[bone].rotation);
        }
    }

    #[test]
    fn a_mask_on_the_root_bone_moves_its_joint_and_keeps_the_root_motion() {
        let node = single(sine("hips_only", 0.25, Vec3::new(0.1, 0.0, 0.0)));
        let frame = Stage::one(node).step(1.3);

        assert_turned(&frame, HIPS, breathing_turn(1.0));
        assert_ne!(frame.joints[HIPS].rotation, input_joints()[HIPS].rotation);
        assert_eq!(frame.root_motion, input_motion());
    }

    #[test]
    fn later_channels_compose_on_the_left() {
        let first = Vec3::new(0.1, 0.0, 0.0);
        let second = Vec3::new(0.0, 0.1, 0.05);
        let node = ProceduralMotionNode::new()
            .with_channel(sine("chest_only", 0.25, first))
            .with_channel(sine("chest_only", 0.4, second));
        let frame = Stage::one(node).step(1.3);

        let first_turn = Quat::from_scaled_axis(first * sine_value(0, 0, 0, 0.25 * 1.3));
        let second_turn = Quat::from_scaled_axis(second * sine_value(0, 0, 1, 0.4 * 1.3));
        assert_turned(&frame, CHEST, second_turn * first_turn);
    }

    #[test]
    fn players_with_the_same_seeds_move_alike_and_other_seeds_differ() {
        let node = single(noise("chest"));

        let same = Stage::new(node.clone(), &[0, 0]).step_all(1.3);
        assert_eq!(same[0], same[1]);

        let seeded = Stage::new(node.clone(), &[0, 1]).step_all(1.3);
        assert_ne!(seeded[0].joints, seeded[1].joints);

        let unseeded = Stage::one(node.clone()).step(1.3);
        let node_seeded = Stage::one(node.with_seed(1)).step(1.3);
        assert_ne!(unseeded.joints, node_seeded.joints);
    }

    #[test]
    fn identical_noise_channels_at_different_indices_differ() {
        let first = Stage::one(single(noise("chest_only"))).step(1.3);
        let second = Stage::one(
            ProceduralMotionNode::new()
                .with_channel(ProceduralMotionChannel {
                    mask: mask("unloaded"),
                    ..noise("chest_only")
                })
                .with_channel(noise("chest_only")),
        )
        .step(1.3);

        assert_ne!(first.joints[CHEST], input_joints()[CHEST]);
        assert_ne!(second.joints[CHEST], input_joints()[CHEST]);
        assert_ne!(first.joints[CHEST], second.joints[CHEST]);
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

    fn with_magnitude(param: Name) -> ProceduralMotionNode {
        single(sine("chest_only", 0.25, Vec3::new(0.1, 0.0, 0.0)).with_magnitude_param(param))
    }

    #[test]
    fn the_magnitude_param_scales_the_rotation() {
        for scale in [0.5, 2.0] {
            let mut stage = Stage::one(with_magnitude(name!("strength")));
            stage.set_param(name!("strength"), scale);
            let frame = stage.step(1.3);
            assert_turned(&frame, CHEST, breathing_turn(scale));
        }
    }

    #[test]
    fn a_zero_negative_or_non_finite_magnitude_leaves_the_input_pose() {
        for scale in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            let mut stage = Stage::one(with_magnitude(name!("strength")));
            stage.set_param(name!("strength"), scale);
            assert_input(&stage.step(1.3));
        }
    }

    #[test]
    fn a_missing_or_non_float_magnitude_param_acts_as_one() {
        let frame = Stage::one(with_magnitude(name!("strength"))).step(1.3);
        assert_turned(&frame, CHEST, breathing_turn(1.0));

        let mut stage = Stage::one(with_magnitude(name!("strength")));
        stage.set_param(name!("strength"), Vec3::ONE);
        assert_turned(&stage.step(1.3), CHEST, breathing_turn(1.0));
    }

    #[test]
    fn channels_that_cannot_move_add_nothing() {
        let amplitude = Vec3::new(0.1, 0.0, 0.0);
        let inert = [
            sine("chest", 0.0, amplitude),
            sine("chest", -1.0, amplitude),
            sine("chest", f32::NAN, amplitude),
            sine("unloaded", 0.25, amplitude),
        ];
        for channel in inert {
            assert_input(&Stage::one(single(channel)).step(1.3));
        }
    }

    #[test]
    fn a_nan_amplitude_component_adds_nothing_on_its_axis() {
        let node = single(sine("chest_only", 0.25, Vec3::new(f32::NAN, 0.1, 0.0)));
        let frame = Stage::one(node).step(1.3);

        let turn = Quat::from_rotation_y(0.1 * sine_value(0, 0, 0, 0.25 * 1.3));
        assert_turned(&frame, CHEST, turn);
    }

    #[test]
    fn a_mask_naming_a_missing_bone_still_moves_the_bones_it_has() {
        let node = single(sine("chest_and_tail", 0.25, Vec3::new(0.1, 0.0, 0.0)));
        let frame = Stage::one(node).step(1.3);

        assert_turned(&frame, CHEST, breathing_turn(1.0));
    }

    #[test]
    fn root_motion_events_translations_scales_and_unmasked_bones_pass_through() {
        let frame = Stage::one(single(noise("chest"))).run(10, 0.1);

        let input = input_joints();
        assert_ne!(frame.joints[CHEST].rotation, input[CHEST].rotation);
        assert_eq!(frame.root_motion, input_motion());
        assert_eq!(frame.events, [input_event()]);
        for (joint, input) in frame.joints.iter().zip(&input) {
            assert_eq!(joint.translation, input.translation);
            assert_eq!(joint.scale, input.scale);
        }
        for bone in [HIPS, SPINE, PROP] {
            assert_eq!(frame.joints[bone], input[bone]);
        }
    }

    #[test]
    fn a_node_without_channels_passes_its_input_through() {
        assert_input(&Stage::one(ProceduralMotionNode::new()).step(1.3));
    }

    #[test]
    fn a_channel_added_since_the_state_was_made_is_kept_up_with() {
        let mut stage = Stage::one(single(sine("chest_only", 0.25, Vec3::new(0.1, 0.0, 0.0))));
        let player = stage.players[0];
        let original = stage.node.clone();
        let grown = original.clone().with_channel(noise("prop"));
        stage
            .state_of::<ProceduralMotionNode, _>(player, |_, state| {
                state.advance(&grown, 0.5);
                assert_eq!(state.channels.len(), 2);
                assert_eq!(state.channels[0].phase, 0.125);
                assert_eq!(state.channels[1].phase, 0.5 * 0.7_f32 as f64);
                state.advance(&original, 0.5);
                assert_eq!(state.channels.len(), 1);
                assert_eq!(state.channels[0].phase, 0.25);
            })
            .unwrap();
    }
}
