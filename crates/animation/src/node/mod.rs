use concerto_ecs::Res;
use concerto_foundation::assets::{asset_store::AssetStore, handle::AssetHandle};
use concerto_name::name;
use concerto_reflect::{Reflect, ReflectDefault};
use uuid::Uuid;

use crate::{
    blackboard::AnimationBlackboard,
    clip::{AnimationChannel, AnimationClip, KeyCursor},
    evaluation::{AnimationGraphContext, RootBone},
    param::Param,
    pose::{
        AnimationFrame, EvaluatedFrame, FiredEvent, FramePool, JointPose, RootMotion,
        WeightedMotion, twist_about,
    },
};

pub mod additive;
mod aim_offset;
pub mod blend_space;
pub mod definition;
pub mod layer;
pub mod model_space;
pub mod state_machine;
pub mod two_bone_ik;

pub use additive::{AdditiveNode, AdditiveNodeState};
pub use aim_offset::{AimOffsetNode, AimOffsetState, AimPoint};
pub use definition::{
    AnimationNode, CreateNodeInputs, ErasedAnimationNode, EvaluateNodeInputs, InputSlots,
    NodeOutcome, NodeReport, NodeState, PlaceholderNode, ReflectAnimationNode, StateMachineReport,
    TransitionReport, UpdateNodeInputs, register_node_inputs,
};
pub use layer::{LayerNode, LayerSpace};
pub use two_bone_ik::{IkGoal, IkTarget, TipRotation, TwoBoneIkNode, TwoBoneIkState};

/// The rate a node plays at: `play_rate` times the `f32` under `param`, or `play_rate` alone when `param` is `None` or holds no `f32`; never negative.
pub fn effective_play_rate(
    play_rate: f32,
    param: Option<Param<f32>>,
    blackboard: &AnimationBlackboard,
) -> f32 {
    let scale = param.and_then(|param| blackboard.read(&param).copied());
    let rate = play_rate * scale.unwrap_or(1.0);
    if rate.is_finite() && rate > 0.0 {
        rate
    } else {
        0.0
    }
}

/// The weight a node applies: `weight` times the `f32` under `param`, or `weight` alone when `param` is `None` or holds no `f32`; clamped to [0, 1], and 0 when NaN.
pub fn effective_weight(
    weight: f32,
    param: Option<Param<f32>>,
    blackboard: &AnimationBlackboard,
) -> f32 {
    let scale = param.and_then(|param| blackboard.read(&param).copied());
    let weight = weight * scale.unwrap_or(1.0);
    if weight.is_nan() {
        0.0
    } else {
        weight.clamp(0.0, 1.0)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Reflect, serde::Serialize, serde::Deserialize)]
pub enum AnimationPlayMode {
    #[default]
    Loop,
    PlayOnce,
}

/// The graph's output: passes its first input through.
#[derive(Default, Reflect)]
#[reflect(AnimationNode)]
pub struct ResultNode {}

/// Blends its inputs, each by its node weight; inputs at zero weight are left out.
#[derive(Default, Reflect)]
#[reflect(AnimationNode)]
pub struct BlendNode {}

fn pass_through(evaluated_inputs: &[EvaluatedFrame], output: &mut AnimationFrame) {
    if let Some(input) = evaluated_inputs.first() {
        output.copy_from(&input.frame);
    }
}

impl AnimationNode for ResultNode {
    type State = ();
    type CreateInput = ();
    type UpdateInput = ();
    type EvaluateInput = ();

    fn input_slots(&self) -> InputSlots {
        InputSlots::Fixed(vec![name!("pose")])
    }

    fn create_state(&self, _: &(), _context: &AnimationGraphContext<'_>) {}

    fn update(&self, _: &mut (), _: f32, _: &(), _: &AnimationGraphContext<'_>) {}

    fn evaluate(
        &self,
        _: &mut (),
        _: &(),
        _: &AnimationGraphContext<'_>,
        _: &[Uuid],
        evaluated_inputs: &[EvaluatedFrame],
        _: &mut FramePool,
        output: &mut AnimationFrame,
    ) {
        pass_through(evaluated_inputs, output);
    }

    fn reset(&self, _: &mut ()) {}
}

impl AnimationNode for BlendNode {
    type State = ();
    type CreateInput = ();
    type UpdateInput = ();
    type EvaluateInput = ();

    fn input_slots(&self) -> InputSlots {
        InputSlots::Variable
    }

    fn create_state(&self, _: &(), _context: &AnimationGraphContext<'_>) {}

    fn update(&self, _: &mut (), _: f32, _: &(), _: &AnimationGraphContext<'_>) {}

    fn evaluate(
        &self,
        _: &mut (),
        _: &(),
        _: &AnimationGraphContext<'_>,
        _: &[Uuid],
        evaluated_inputs: &[EvaluatedFrame],
        _: &mut FramePool,
        output: &mut AnimationFrame,
    ) {
        let mut weighted = evaluated_inputs.iter().filter(|input| input.weight > 0.0);
        let Some(first) = weighted.next() else {
            return;
        };
        output.copy_from(&first.frame);
        let mut total_weight = first.weight;
        for input in weighted {
            total_weight += input.weight;
            output.blend(&input.frame, input.weight / total_weight);
        }
    }

    fn root_motion(
        &self,
        _: &mut (),
        _: &(),
        _: &AnimationGraphContext<'_>,
        _: &[Uuid],
        inputs: &[WeightedMotion],
    ) -> RootMotion {
        let mut weighted = inputs.iter().filter(|input| input.weight > 0.0);
        let Some(first) = weighted.next() else {
            return RootMotion::IDENTITY;
        };
        let mut motion = first.motion;
        let mut total_weight = first.weight;
        for input in weighted {
            total_weight += input.weight;
            motion = motion.blend(input.motion, input.weight / total_weight);
        }
        motion
    }

    fn reset(&self, _: &mut ()) {}
}

pub(crate) struct ClipClock {
    time: f32,
    previous_time: f32,
    wrapped: bool,
    start_time: f32,
    is_paused: bool,
    is_finished: bool,
    fresh: bool,
    fired: Vec<CrossedKey>,
}

struct CrossedKey {
    key: u32,
    time: f32,
}

pub(crate) struct ClockSpan {
    pub(crate) length: f32,
    pub(crate) offset: f32,
}

impl ClipClock {
    pub(crate) fn new(start_time: f32) -> Self {
        Self {
            time: start_time,
            previous_time: start_time,
            wrapped: false,
            start_time,
            is_paused: false,
            is_finished: false,
            fresh: true,
            fired: Vec::new(),
        }
    }

    pub(crate) fn begin_step(&mut self) -> bool {
        self.fired.clear();
        self.previous_time = self.time;
        self.wrapped = false;
        self.is_finished = false;
        self.is_paused
    }

    pub(crate) fn advance(
        &mut self,
        clip: &AnimationClip,
        span: ClockSpan,
        delta_time: f32,
        rate: f32,
        play_mode: &AnimationPlayMode,
    ) {
        self.time += delta_time * rate;

        match play_mode {
            AnimationPlayMode::Loop => {
                if self.time > span.length {
                    self.time = self.start_time;
                    self.wrapped = true;
                    self.is_finished = true;
                }
            }
            AnimationPlayMode::PlayOnce => {
                if self.time > span.length {
                    self.is_paused = true;
                    self.is_finished = true;
                }
            }
        }

        if rate == 0.0 {
            return;
        }
        let fresh = std::mem::take(&mut self.fresh);
        if self.wrapped {
            let start_fired = fresh && self.previous_time <= self.start_time;
            self.fire(clip, &span, self.previous_time, span.length, fresh);
            self.fire(clip, &span, self.start_time, self.time, !start_fired);
        } else {
            let to = self.time.min(span.length);
            self.fire(clip, &span, self.previous_time, to, fresh);
        }
    }

    fn fire(
        &mut self,
        clip: &AnimationClip,
        span: &ClockSpan,
        from: f32,
        to: f32,
        include_from: bool,
    ) {
        for key in clip.events_in(span.offset + from, span.offset + to, include_from) {
            self.fired.push(CrossedKey {
                key: key as u32,
                time: clip.events()[key].time,
            });
        }
    }

    pub(crate) fn play(&mut self) {
        self.restart();
        self.is_paused = false;
        self.is_finished = false;
    }

    pub(crate) fn restart(&mut self) {
        self.time = self.start_time;
        self.previous_time = self.start_time;
        self.wrapped = false;
        self.fresh = true;
    }

    pub(crate) fn reset(&mut self) {
        self.restart();
        self.is_finished = false;
        self.is_paused = false;
    }

    pub(crate) fn current_time(&self) -> f32 {
        self.time
    }
}

pub struct AnimationClipNodeState {
    clock: ClipClock,
    cursors: Vec<KeyCursor>,
}

impl AnimationClipNodeState {
    pub fn new() -> Self {
        Self {
            clock: ClipClock::new(0.0),
            cursors: Vec::new(),
        }
    }

    pub fn with_start_time(mut self, start_time: f32) -> Self {
        self.clock = ClipClock::new(start_time);
        self
    }

    pub fn play(&mut self) {
        self.clock.play();
    }

    pub fn current_time(&self) -> f32 {
        self.clock.current_time()
    }
}

impl Default for AnimationClipNodeState {
    fn default() -> Self {
        Self::new()
    }
}

impl AnimationNode for AnimationClipNode {
    type State = AnimationClipNodeState;
    type CreateInput = ();
    type UpdateInput = Res<'static, AssetStore<AnimationClip>>;
    type EvaluateInput = Res<'static, AssetStore<AnimationClip>>;

    fn input_slots(&self) -> InputSlots {
        InputSlots::None
    }

    fn create_state(&self, _: &(), _context: &AnimationGraphContext<'_>) -> AnimationClipNodeState {
        AnimationClipNodeState::new().with_start_time(self.start_time)
    }

    fn update(
        &self,
        state: &mut AnimationClipNodeState,
        delta_time: f32,
        clips: &Res<AssetStore<AnimationClip>>,
        context: &AnimationGraphContext<'_>,
    ) {
        if state.clock.begin_step() {
            return;
        }

        let Some(clip) = clips.get(&self.clip) else {
            return;
        };

        let rate = effective_play_rate(self.play_rate, self.play_rate_param, context.blackboard());
        let span = ClockSpan {
            length: clip.duration(),
            offset: 0.0,
        };
        state
            .clock
            .advance(clip, span, delta_time, rate, &self.play_mode);
    }

    fn evaluate(
        &self,
        state: &mut AnimationClipNodeState,
        clips: &Res<AssetStore<AnimationClip>>,
        context: &AnimationGraphContext<'_>,
        bone_ids: &[Uuid],
        _evaluated_inputs: &[EvaluatedFrame],
        _pool: &mut FramePool,
        output: &mut AnimationFrame,
    ) {
        for crossed in &state.clock.fired {
            output.push_event(FiredEvent {
                clip: self.clip.id(),
                key: crossed.key,
                time: crossed.time,
                weight: 1.0,
                mirrored: false,
            });
        }

        let Some(animation_clip) = clips.get(&self.clip) else {
            return;
        };

        let time = state.current_time();
        let mut sampled = 0;
        for (bone_index, uuid) in bone_ids.iter().enumerate() {
            let Some(animation_channels) = animation_clip.get_channels(uuid) else {
                continue;
            };

            let Some(joint_pose) = output.joint_mut(bone_index) else {
                continue;
            };

            let cursors = sampled..sampled + animation_channels.len();
            sampled = cursors.end;
            if state.cursors.len() < cursors.end {
                state.cursors.resize(cursors.end, KeyCursor::default());
            }
            for (animation_channel, cursor) in
                animation_channels.iter().zip(&mut state.cursors[cursors])
            {
                animation_channel.sample_transform_from(time, cursor, joint_pose);
            }

            if let Some(root) = context.root().filter(|root| root.index == bone_index) {
                net_of_root_motion(joint_pose, root);
            }
        }
    }

    fn root_motion(
        &self,
        state: &mut AnimationClipNodeState,
        clips: &Res<AssetStore<AnimationClip>>,
        context: &AnimationGraphContext<'_>,
        bone_ids: &[Uuid],
        _inputs: &[WeightedMotion],
    ) -> RootMotion {
        clips.get(&self.clip).map_or(RootMotion::IDENTITY, |clip| {
            root_motion(&state.clock, clip, context.root(), bone_ids)
        })
    }

    fn report(&self, state: &AnimationClipNodeState) -> NodeReport {
        NodeReport {
            clip_time: Some(state.current_time()),
            state_machine: None,
        }
    }

    fn reset(&self, state: &mut AnimationClipNodeState) {
        state.clock.reset();
    }

    fn is_finished(&self, state: &AnimationClipNodeState) -> bool {
        state.clock.is_finished
    }
}

fn net_of_root_motion(joint: &mut JointPose, root: &RootBone) {
    let turned = joint.rotation * root.rest.rotation.inverse();
    joint.rotation = twist_about(turned, root.up).inverse() * joint.rotation;
    let lifted = (joint.translation - root.rest.translation).dot(root.up);
    joint.translation = root.rest.translation + root.up * lifted;
}

fn root_motion(
    clock: &ClipClock,
    clip: &AnimationClip,
    root: Option<&RootBone>,
    bone_ids: &[Uuid],
) -> RootMotion {
    let Some((root, channels)) = root.and_then(|root| {
        let channels = clip.get_channels(bone_ids.get(root.index)?)?;
        Some((root, channels))
    }) else {
        return RootMotion::IDENTITY;
    };

    let stretch = |from: f32, to: f32| {
        RootMotion::between(
            &sample_root(channels, &root.rest, from),
            &sample_root(channels, &root.rest, to),
            root.rest.rotation,
        )
    };

    if clock.wrapped {
        stretch(clock.previous_time, clip.duration()).then(stretch(clock.start_time, clock.time))
    } else {
        stretch(clock.previous_time, clock.time)
    }
}

fn sample_root(channels: &[AnimationChannel], rest: &JointPose, time: f32) -> JointPose {
    let mut joint = rest.clone();
    for channel in channels {
        channel.sample_transform(time, &mut joint);
    }
    joint
}

#[derive(Clone, Reflect, serde::Serialize, serde::Deserialize)]
#[reflect(AnimationNode, Default)]
pub struct AnimationClipNode {
    clip: AssetHandle<AnimationClip>,
    play_mode: AnimationPlayMode,
    start_time: f32,
    play_rate: f32,
    play_rate_param: Option<Param<f32>>,
}

impl Default for AnimationClipNode {
    fn default() -> Self {
        Self::new(AssetHandle::default())
    }
}

impl AnimationClipNode {
    pub fn new(clip: AssetHandle<AnimationClip>) -> Self {
        Self {
            clip,
            play_mode: AnimationPlayMode::Loop,
            start_time: 0.0,
            play_rate: 1.0,
            play_rate_param: None,
        }
    }

    pub fn clip(&self) -> &AssetHandle<AnimationClip> {
        &self.clip
    }

    pub fn with_start_time(mut self, start_time: f32) -> Self {
        self.start_time = start_time;
        self
    }

    pub fn with_play_mode(mut self, play_mode: AnimationPlayMode) -> Self {
        self.play_mode = play_mode;
        self
    }

    /// Plays the clip `play_rate` times faster than authored (`0.5` is half
    /// speed).
    pub fn with_play_rate(mut self, play_rate: f32) -> Self {
        self.play_rate = play_rate.max(0.0);
        self
    }

    /// Multiplies the play rate by the `f32` blackboard param named `param`, when it holds one.
    pub fn with_play_rate_param(mut self, param: impl Into<Param<f32>>) -> Self {
        self.play_rate_param = Some(param.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use concerto_ecs::Res;
    use concerto_foundation::assets::{
        AssetId, asset_server::AssetServer, asset_store::AssetStore, handle::AssetHandle,
    };
    use concerto_name::{Name, name};
    use glam::{Quat, Vec3};
    use uuid::Uuid;

    use super::{
        AnimationClipNode, AnimationClipNodeState, AnimationNode, AnimationPlayMode, BlendNode,
        effective_play_rate, effective_weight,
    };
    use crate::{
        blackboard::AnimationBlackboard,
        clip::{AnimationChanelOutput, AnimationChannel, AnimationClip},
        evaluation::{AnimationGraphContext, RootBone},
        event::AnimationEvent,
        pose::{
            AnimationFrame, EvaluatedFrame, FiredEvent, FramePool, JointPose, Pose, RootMotion,
            WeightedMotion,
        },
        testing::Walks,
    };

    const ROOT: Uuid = Uuid::from_u128(1);

    fn clips<'a>(context: &'a AnimationGraphContext<'a>) -> &'a Res<'a, AssetStore<AnimationClip>> {
        context
            .inputs
            .get::<Res<'static, AssetStore<AnimationClip>>>()
            .unwrap()
    }

    struct Rig {
        _server: AssetServer,
        walks: Walks,
        blackboard: AnimationBlackboard,
        rest: JointPose,
        up: Vec3,
        clip: AssetHandle<AnimationClip>,
    }

    impl Rig {
        fn new(clip: AnimationClip, rest: JointPose) -> Self {
            let mut server = AssetServer::new();
            let mut clips = AssetStore::<AnimationClip>::new();
            server.register_asset(&clips);
            let handle = server.add(AnimationClip::default());
            clips.insert(handle.id(), clip);
            Self {
                _server: server,
                walks: Walks::new(clips, AssetStore::new()),
                blackboard: AnimationBlackboard::default(),
                rest,
                up: Vec3::Y,
                clip: handle,
            }
        }

        fn with_up(mut self, up: Vec3) -> Self {
            self.up = up;
            self
        }

        fn root(&self) -> RootBone {
            RootBone {
                index: 0,
                rest: self.rest.clone(),
                up: self.up,
            }
        }

        fn create(&self, node: &AnimationClipNode) -> AnimationClipNodeState {
            self.walks.create(&self.blackboard, |context| {
                AnimationNode::create_state(node, &(), context)
            })
        }

        fn evaluate(
            &self,
            node: &AnimationClipNode,
            state: &mut AnimationClipNodeState,
            bones: &[Uuid],
            pool: &mut FramePool,
            output: &mut AnimationFrame,
        ) {
            self.walks
                .evaluate(&self.blackboard, Some(self.root()), |context| {
                    node.evaluate(state, clips(context), context, bones, &[], pool, output)
                });
        }

        fn node(&self) -> AnimationClipNode {
            AnimationClipNode::new(self.clip.clone())
        }

        fn pose(&self, bones: &[Uuid]) -> AnimationFrame {
            let node = self.node();
            let mut state = self.create(&node);
            let mut pool = FramePool::new(bones.len());
            pool.set_rest(&vec![self.rest.clone(); bones.len()]);
            let mut output = pool.acquire();
            self.evaluate(&node, &mut state, bones, &mut pool, &mut output);
            output
        }

        fn motion(
            &self,
            node: &AnimationClipNode,
            state: &mut AnimationClipNodeState,
        ) -> RootMotion {
            self.walks
                .pass(&self.blackboard, Some(self.root()), |context| {
                    AnimationNode::root_motion(node, state, clips(context), context, &[ROOT], &[])
                })
        }

        fn step(
            &self,
            node: &AnimationClipNode,
            state: &mut AnimationClipNodeState,
            delta_time: f32,
        ) -> RootMotion {
            self.walks.update(&self.blackboard, |context| {
                node.update(state, delta_time, clips(context), context)
            });
            self.motion(node, state)
        }

        fn fire(
            &self,
            node: &AnimationClipNode,
            state: &mut AnimationClipNodeState,
            delta_time: f32,
        ) -> Vec<f32> {
            self.walks.update(&self.blackboard, |context| {
                node.update(state, delta_time, clips(context), context)
            });
            let mut pool = FramePool::new(1);
            let mut output = pool.acquire();
            self.evaluate(node, state, &[ROOT], &mut pool, &mut output);
            output.events().iter().map(|event| event.time).collect()
        }
    }

    fn with_events(times: &[f32]) -> AnimationClip {
        let mut clip = clip(vec![translation_channel(Vec3::ZERO, Vec3::X)]);
        for (index, time) in times.iter().enumerate() {
            clip.add_event(
                *time,
                AnimationEvent {
                    name: Name::new(&format!("event_{index}")),
                },
            );
        }
        clip
    }

    fn events_rig(times: &[f32]) -> Rig {
        Rig::new(with_events(times), at_rest(Quat::IDENTITY))
    }

    struct Fire {
        dt: f32,
        fired: Vec<f32>,
    }

    fn fire(dt: f32, fired: &[f32]) -> Fire {
        Fire {
            dt,
            fired: fired.to_vec(),
        }
    }

    struct FiringCase {
        name: &'static str,
        times: Vec<f32>,
        start: f32,
        play_once: bool,
        steps: Vec<Fire>,
    }

    fn firing_case(name: &'static str, times: &[f32], steps: Vec<Fire>) -> FiringCase {
        FiringCase {
            name,
            times: times.to_vec(),
            start: 0.0,
            play_once: false,
            steps,
        }
    }

    #[test]
    fn a_clip_node_fires_each_event_it_crosses_once() {
        let cases = [
            firing_case(
                "a step fires the events after the previous time up to the new one",
                &[0.1, 0.3, 0.5],
                vec![fire(0.2, &[0.1]), fire(0.2, &[0.3])],
            ),
            FiringCase {
                start: 0.2,
                ..firing_case(
                    "a loop wrap fires to the end and then the start",
                    &[0.2, 0.5, 0.9, 1.0],
                    vec![
                        fire(0.4, &[0.2, 0.5]),
                        fire(0.3, &[0.9]),
                        fire(0.4, &[1.0, 0.2]),
                        fire(0.4, &[0.5]),
                    ],
                )
            },
            FiringCase {
                play_once: true,
                ..firing_case(
                    "a play once clip fires up to its end and then nothing",
                    &[0.9, 1.0],
                    vec![fire(0.75, &[]), fire(0.5, &[0.9, 1.0]), fire(0.5, &[])],
                )
            },
            firing_case(
                "an event exactly at the new time fires once",
                &[0.5],
                vec![fire(0.25, &[]), fire(0.25, &[0.5]), fire(0.25, &[])],
            ),
        ];

        for case in cases {
            let rig = events_rig(&case.times);
            let node = rig.node().with_start_time(case.start);
            let node = if case.play_once {
                node.with_play_mode(AnimationPlayMode::PlayOnce)
            } else {
                node
            };
            let mut state = rig.create(&node);

            for (index, step) in case.steps.into_iter().enumerate() {
                assert_eq!(
                    rig.fire(&node, &mut state, step.dt),
                    step.fired,
                    "{}: step {index}",
                    case.name
                );
            }
        }
    }

    fn translation_channel(from: Vec3, to: Vec3) -> AnimationChannel {
        AnimationChannel::new(
            vec![0.0, 1.0],
            AnimationChanelOutput::Translation(vec![from, to]),
        )
    }

    fn rotation_channel(from: Quat, to: Quat) -> AnimationChannel {
        AnimationChannel::new(
            vec![0.0, 1.0],
            AnimationChanelOutput::Rotation(vec![from, to]),
        )
    }

    fn clip(channels: Vec<AnimationChannel>) -> AnimationClip {
        let mut clip = AnimationClip::default();
        for channel in channels {
            clip.add_channel(ROOT, channel);
        }
        clip
    }

    fn at_rest(rotation: Quat) -> JointPose {
        JointPose {
            translation: Vec3::new(0.0, 1.0, 0.0),
            rotation,
            scale: Vec3::ONE,
        }
    }

    fn turned_armature() -> Quat {
        Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)
    }

    fn held(rotation: Quat) -> AnimationChannel {
        rotation_channel(rotation, rotation)
    }

    struct PoseCase {
        name: &'static str,
        channels: Vec<AnimationChannel>,
        rest: Quat,
        up: Vec3,
        translation: Vec3,
        rotation: Quat,
    }

    #[test]
    fn a_clip_root_keeps_its_pitch_and_height_and_gives_up_its_yaw_and_ground_translation() {
        let rest = turned_armature();
        let pitch = Quat::from_rotation_x(0.4);
        let standing = Vec3::new(0.0, 1.0, 0.0);
        let cases = [
            PoseCase {
                name: "its yaw about y is given up and its pitch kept",
                channels: vec![held(Quat::from_rotation_y(1.0) * pitch * rest)],
                rest,
                up: Vec3::Y,
                translation: standing,
                rotation: pitch * rest,
            },
            PoseCase {
                name: "its yaw is measured about the up of the root",
                channels: vec![held(Quat::from_rotation_z(1.0) * pitch * rest)],
                rest,
                up: Vec3::Z,
                translation: standing,
                rotation: pitch * rest,
            },
            PoseCase {
                name: "its height is kept and its ground translation left at rest",
                channels: vec![translation_channel(
                    Vec3::new(3.0, 1.75, -2.0),
                    Vec3::new(3.0, 1.75, -2.0),
                )],
                rest: Quat::IDENTITY,
                up: Vec3::Y,
                translation: Vec3::new(0.0, 1.75, 0.0),
                rotation: Quat::IDENTITY,
            },
        ];

        for case in cases {
            let rig = Rig::new(clip(case.channels), at_rest(case.rest)).with_up(case.up);

            let root = rig.pose(&[ROOT]).joints()[0].clone();

            assert!(
                root.translation.abs_diff_eq(case.translation, 1e-5),
                "{}: translation {}",
                case.name,
                root.translation
            );
            assert!(
                root.rotation.angle_between(case.rotation) < 1e-3,
                "{}: rotation {}",
                case.name,
                root.rotation
            );
        }
    }

    enum Move {
        Step(f32),
        Reset,
    }

    struct Motion {
        then: Move,
        translation: Vec3,
        rotation: Quat,
    }

    fn motion(then: Move, translation: Vec3, rotation: Quat) -> Motion {
        Motion {
            then,
            translation,
            rotation,
        }
    }

    fn none(then: Move) -> Motion {
        motion(then, Vec3::ZERO, Quat::IDENTITY)
    }

    struct MotionCase {
        name: &'static str,
        clip: AnimationClip,
        rest: Quat,
        node: AnimationClipNode,
        motions: Vec<Motion>,
    }

    fn moving(name: &'static str, clip: AnimationClip, motions: Vec<Motion>) -> MotionCase {
        MotionCase {
            name,
            clip,
            rest: Quat::IDENTITY,
            node: AnimationClipNode::new(AssetHandle::default()),
            motions,
        }
    }

    #[test]
    fn a_clip_root_motion_is_the_delta_of_its_root_since_the_last_frame_read_from_where_it_faced() {
        use Move::*;
        let rest = turned_armature();
        let along_x = || {
            clip(vec![translation_channel(
                Vec3::ZERO,
                Vec3::new(2.0, 0.0, 0.0),
            )])
        };
        let along_xz = || translation_channel(Vec3::ZERO, Vec3::new(2.0, 0.0, 4.0));
        let node = AnimationClipNode::new(AssetHandle::default());
        let y = Quat::from_rotation_y;
        let cases = [
            MotionCase {
                rest,
                ..moving(
                    "a delta measured from a turned rest",
                    clip(vec![
                        along_xz(),
                        rotation_channel(rest, Quat::from_rotation_y(1.0) * rest),
                    ]),
                    vec![
                        motion(Step(0.25), Vec3::new(0.5, 0.0, 1.0), y(0.25)),
                        motion(Step(0.25), y(-0.25) * Vec3::new(0.5, 0.0, 1.0), y(0.25)),
                    ],
                )
            },
            MotionCase {
                node: node.clone().with_start_time(0.2),
                ..moving(
                    "a loop delta across the wrap is the distance covered and not a jump back",
                    clip(vec![
                        translation_channel(Vec3::ZERO, Vec3::new(2.0, 0.0, 0.0)),
                        rotation_channel(Quat::IDENTITY, Quat::from_rotation_y(1.0)),
                    ]),
                    vec![
                        motion(Step(0.4), y(-0.2) * Vec3::new(0.8, 0.0, 0.0), y(0.4)),
                        motion(Step(0.3), y(-0.6) * Vec3::new(0.6, 0.0, 0.0), y(0.3)),
                        motion(Step(0.4), y(-0.9) * Vec3::new(0.2, 0.0, 0.0), y(0.1)),
                        motion(Step(0.4), y(-0.2) * Vec3::new(0.8, 0.0, 0.0), y(0.4)),
                    ],
                )
            },
            moving(
                "a clip reset after a wrap gives no root motion",
                along_x(),
                vec![
                    motion(Step(0.8), Vec3::new(1.6, 0.0, 0.0), Quat::IDENTITY),
                    motion(Step(0.8), Vec3::new(0.4, 0.0, 0.0), Quat::IDENTITY),
                    none(Reset),
                ],
            ),
            MotionCase {
                node: node.clone().with_play_mode(AnimationPlayMode::PlayOnce),
                ..moving(
                    "a play once clip gives its last delta and then none",
                    along_x(),
                    vec![
                        motion(Step(0.75), Vec3::new(1.5, 0.0, 0.0), Quat::IDENTITY),
                        motion(Step(0.5), Vec3::new(0.5, 0.0, 0.0), Quat::IDENTITY),
                        none(Step(0.5)),
                    ],
                )
            },
        ];

        for case in cases {
            let rig = Rig::new(case.clip, at_rest(case.rest));
            let node = AnimationClipNode {
                clip: rig.clip.clone(),
                ..case.node
            };
            let mut state = rig.create(&node);

            for (index, expected) in case.motions.into_iter().enumerate() {
                let actual = match expected.then {
                    Step(delta_time) => rig.step(&node, &mut state, delta_time),
                    Reset => {
                        node.reset(&mut state);
                        rig.motion(&node, &mut state)
                    }
                };

                assert!(
                    actual.translation.abs_diff_eq(expected.translation, 1e-4)
                        && actual.rotation.angle_between(expected.rotation) < 1e-3,
                    "{}: motion {index} is {actual:?}, not {} {}",
                    case.name,
                    expected.translation,
                    expected.rotation
                );
            }
        }
    }

    struct RateCase {
        name: &'static str,
        rate: f32,
        param: Option<Name>,
        effective: f32,
    }

    #[test]
    fn the_effective_play_rate_is_the_rate_times_a_float_param_and_zero_when_not_finite() {
        let mut blackboard = AnimationBlackboard::default();
        blackboard.set(name!("draw_speed"), 4.0_f32);
        blackboard.set(name!("infinite"), f32::INFINITY);
        let rate = |name, rate, param, effective| RateCase {
            name,
            rate,
            param,
            effective,
        };
        let cases = [
            rate("a float param", 1.5, Some(name!("draw_speed")), 6.0),
            rate("an infinite param", 1.0, Some(name!("infinite")), 0.0),
        ];

        for case in cases {
            assert_eq!(
                effective_play_rate(case.rate, case.param.map(Into::into), &blackboard),
                case.effective,
                "{}",
                case.name
            );
        }
    }

    #[test]
    fn the_effective_weight_is_the_weight_times_a_float_param_clamped_to_0_and_1() {
        let mut blackboard = AnimationBlackboard::default();
        blackboard.set(name!("half"), 0.5_f32);
        blackboard.set(name!("triple"), 3.0_f32);
        blackboard.set(name!("nan"), f32::NAN);
        let weight = |name, rate, param, effective| RateCase {
            name,
            rate,
            param,
            effective,
        };
        let cases = [
            weight("a float param", 0.5, Some(name!("half")), 0.25),
            weight("above 1", 0.5, Some(name!("triple")), 1.0),
            weight("a NaN param", 0.5, Some(name!("nan")), 0.0),
        ];

        for case in cases {
            assert_eq!(
                effective_weight(case.rate, case.param.map(Into::into), &blackboard),
                case.effective,
                "{}",
                case.name
            );
        }
    }

    fn weighted(translation: Vec3, weight: f32) -> EvaluatedFrame {
        let frame = AnimationFrame::new(Pose::from_joints(vec![JointPose {
            translation,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        }]));
        EvaluatedFrame { frame, weight }
    }

    fn blend(inputs: &[EvaluatedFrame]) -> AnimationFrame {
        let walks = Walks::new(AssetStore::new(), AssetStore::new());
        let mut pool = FramePool::new(1);
        let mut output = pool.acquire();
        walks.evaluate(&AnimationBlackboard::default(), None, |context| {
            BlendNode {}.evaluate(&mut (), &(), context, &[], inputs, &mut pool, &mut output)
        });
        output
    }

    struct BlendCase {
        name: &'static str,
        inputs: Vec<EvaluatedFrame>,
        translation: Vec3,
    }

    #[test]
    fn a_blend_is_the_weighted_average_of_its_inputs() {
        let blend_case = |name, inputs, translation| BlendCase {
            name,
            inputs,
            translation,
        };
        let cases = [
            blend_case(
                "three inputs give their weighted average",
                vec![
                    weighted(Vec3::ZERO, 1.0),
                    weighted(Vec3::X * 4.0, 1.0),
                    weighted(Vec3::Z * 4.0, 2.0),
                ],
                Vec3::new(1.0, 0.0, 2.0),
            ),
            blend_case(
                "an input at zero weight has no part in the blend",
                vec![weighted(Vec3::X * 4.0, 0.0), weighted(Vec3::Z * 2.0, 1.0)],
                Vec3::Z * 2.0,
            ),
            blend_case(
                "no weighted input keeps the rest pose",
                vec![weighted(Vec3::X * 4.0, 0.0)],
                Vec3::ZERO,
            ),
        ];

        for case in cases {
            let blended = blend(&case.inputs);
            let motions: Vec<WeightedMotion> = case
                .inputs
                .iter()
                .map(|input| WeightedMotion {
                    motion: RootMotion {
                        translation: input.frame.joints()[0].translation,
                        rotation: Quat::IDENTITY,
                    },
                    weight: input.weight,
                })
                .collect();
            let walks = Walks::new(AssetStore::new(), AssetStore::new());
            let pass = walks.evaluate(&AnimationBlackboard::default(), None, |context| {
                AnimationNode::root_motion(&BlendNode {}, &mut (), &(), context, &[], &motions)
            });
            assert!(
                pass.translation.abs_diff_eq(case.translation, 1e-5)
                    && pass.rotation.abs_diff_eq(Quat::IDENTITY, 1e-5),
                "{}: the pass gives {pass:?}",
                case.name
            );

            let joint = blended.joints()[0].translation;
            assert!(
                joint.abs_diff_eq(case.translation, 1e-5),
                "{}: joint {joint}",
                case.name
            );
        }
    }

    fn firing(key: u32, weight: f32) -> EvaluatedFrame {
        let mut input = weighted(Vec3::ZERO, weight);
        input.frame.push_event(FiredEvent {
            clip: AssetId::new(),
            key,
            time: 0.0,
            weight: 1.0,
            mirrored: false,
        });
        input
    }

    #[derive(Debug, PartialEq)]
    struct KeyWeight {
        key: u32,
        weight: f32,
    }

    fn key_weight(key: u32, weight: f32) -> KeyWeight {
        KeyWeight { key, weight }
    }

    fn event_weights(frame: &AnimationFrame) -> Vec<KeyWeight> {
        let mut weights: Vec<_> = frame
            .events()
            .iter()
            .map(|event| key_weight(event.key, event.weight))
            .collect();
        weights.sort_by_key(|weight| weight.key);
        weights
    }

    #[test]
    fn a_blend_gives_each_input_event_the_weight_of_its_input_and_drops_those_of_unweighted_ones() {
        let weighted_events = blend(&[firing(0, 0.25), firing(1, 0.75)]);
        let unweighted_event = blend(&[firing(0, 0.0), firing(1, 1.0)]);

        assert_eq!(
            event_weights(&weighted_events),
            [key_weight(0, 0.25), key_weight(1, 0.75)]
        );
        assert_eq!(event_weights(&unweighted_event), [key_weight(1, 1.0)]);
    }
}
