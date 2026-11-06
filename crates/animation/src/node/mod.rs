use concerto_ecs::Res;
use concerto_foundation::assets::{asset_store::AssetStore, handle::AssetHandle};
use concerto_name::Name;
use concerto_reflect::Reflect;
use uuid::Uuid;

use crate::{
    blackboard::AnimationBlackboard,
    clip::{AnimationChannel, AnimationClip},
    evaluation::{AnimationGraphContext, RootBone},
    pose::{
        AnimationFrame, EvaluatedFrame, FiredEvent, FramePool, JointPose, RootMotion, twist_about,
    },
};

pub mod blend_space;
pub mod definition;
pub mod model_space;
pub mod state_machine;
pub mod two_bone_ik;

pub use definition::{
    AnimationNode, CreateNodeInputs, ErasedAnimationNode, EvaluateNodeInputs, NodeState,
    PlaceholderNode, ReflectAnimationNode, UpdateNodeInputs, register_node_inputs,
};
pub use two_bone_ik::{IkGoal, IkTarget, TipRotation, TwoBoneIkNode, TwoBoneIkState};

/// The rate a node plays at: `play_rate` times the `f32` under `param`, or `play_rate` alone when `param` is `None` or holds no `f32`; never negative.
pub fn effective_play_rate(
    play_rate: f32,
    param: Option<Name>,
    blackboard: &AnimationBlackboard,
) -> f32 {
    let scale = param.and_then(|param| blackboard.get_float(param));
    let rate = play_rate * scale.unwrap_or(1.0);
    if rate.is_finite() && rate > 0.0 {
        rate
    } else {
        0.0
    }
}

#[derive(Clone, Default, Reflect, serde::Serialize, serde::Deserialize)]
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

    fn reset(&self, _: &mut ()) {}
}

pub struct AnimationClipNodeState {
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

impl AnimationClipNodeState {
    pub fn new() -> Self {
        Self {
            time: 0.0,
            previous_time: 0.0,
            wrapped: false,
            start_time: 0.0,
            is_paused: false,
            is_finished: false,
            fresh: true,
            fired: Vec::new(),
        }
    }

    pub fn with_start_time(mut self, start_time: f32) -> Self {
        self.start_time = start_time;
        self.time = start_time;
        self.previous_time = start_time;
        self
    }

    pub fn play(&mut self) {
        self.restart();
        self.is_paused = false;
        self.is_finished = false;
    }

    fn restart(&mut self) {
        self.time = self.start_time;
        self.previous_time = self.start_time;
        self.wrapped = false;
        self.fresh = true;
    }

    fn fire(&mut self, clip: &AnimationClip, from: f32, to: f32, include_from: bool) {
        for key in clip.events_in(from, to, include_from) {
            self.fired.push(CrossedKey {
                key: key as u32,
                time: clip.events()[key].time,
            });
        }
    }

    pub fn current_time(&self) -> f32 {
        self.time
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
        state.fired.clear();
        state.previous_time = state.time;
        state.wrapped = false;

        if state.is_finished {
            state.is_finished = false;
        }

        if state.is_paused {
            return;
        }

        let Some(clip) = clips.get(&self.clip) else {
            return;
        };

        let rate = effective_play_rate(self.play_rate, self.play_rate_param, context.blackboard());
        state.time += delta_time * rate;

        match self.play_mode {
            AnimationPlayMode::Loop => {
                if state.time > clip.duration() {
                    state.time = state.start_time;
                    state.wrapped = true;
                    state.is_finished = true;
                }
            }
            AnimationPlayMode::PlayOnce => {
                if state.time > clip.duration() {
                    state.is_paused = true;
                    state.is_finished = true;
                }
            }
        }

        if rate == 0.0 {
            return;
        }
        let fresh = std::mem::take(&mut state.fresh);
        if state.wrapped {
            let start_fired = fresh && state.previous_time <= state.start_time;
            state.fire(clip, state.previous_time, clip.duration(), fresh);
            state.fire(clip, state.start_time, state.time, !start_fired);
        } else {
            let to = state.time.min(clip.duration());
            state.fire(clip, state.previous_time, to, fresh);
        }
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
        for crossed in &state.fired {
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

        bone_ids
            .iter()
            .map(|uuid| animation_clip.get_channels(uuid))
            .enumerate()
            .for_each(|(bone_index, animation_channels)| {
                let Some(animation_channels) = animation_channels else {
                    return;
                };

                let Some(joint_pose) = output.joint_mut(bone_index) else {
                    return;
                };

                for animation_channel in animation_channels {
                    animation_channel.sample_transform(state.current_time(), joint_pose);
                }

                if let Some(root) = context.root().filter(|root| root.index == bone_index) {
                    net_of_root_motion(joint_pose, root);
                }
            });

        output.set_root_motion(root_motion(state, animation_clip, context.root(), bone_ids));
    }

    fn reset(&self, state: &mut AnimationClipNodeState) {
        state.restart();
        state.is_finished = false;
        state.is_paused = false;
    }

    fn is_finished(&self, state: &AnimationClipNodeState) -> bool {
        state.is_finished
    }
}

fn net_of_root_motion(joint: &mut JointPose, root: &RootBone) {
    let turned = joint.rotation * root.rest.rotation.inverse();
    joint.rotation = twist_about(turned, root.up).inverse() * joint.rotation;
    let lifted = (joint.translation - root.rest.translation).dot(root.up);
    joint.translation = root.rest.translation + root.up * lifted;
}

fn root_motion(
    state: &AnimationClipNodeState,
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

    if state.wrapped {
        stretch(state.previous_time, clip.duration()).then(stretch(state.start_time, state.time))
    } else {
        stretch(state.previous_time, state.time)
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
#[reflect(AnimationNode)]
pub struct AnimationClipNode {
    clip: AssetHandle<AnimationClip>,
    play_mode: AnimationPlayMode,
    start_time: f32,
    play_rate: f32,
    play_rate_param: Option<Name>,
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
    pub fn with_play_rate_param(mut self, param: Name) -> Self {
        self.play_rate_param = Some(param);
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
        effective_play_rate,
    };
    use crate::{
        blackboard::AnimationBlackboard,
        clip::{AnimationChanelOutput, AnimationChannel, AnimationClip},
        evaluation::{AnimationGraphContext, RootBone},
        event::AnimationEvent,
        pose::{
            AnimationFrame, EvaluatedFrame, FiredEvent, FramePool, JointPose, Pose, RootMotion,
        },
        test_walks::Walks,
    };

    const ROOT: Uuid = Uuid::from_u128(1);

    fn clips<'a>(context: &'a AnimationGraphContext<'a>) -> &'a Res<'a, AssetStore<AnimationClip>> {
        context
            .inputs
            .get::<Res<'static, AssetStore<AnimationClip>>>()
            .unwrap()
    }
    const OTHER: Uuid = Uuid::from_u128(2);

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
            let mut pool = FramePool::new(1);
            let mut output = pool.acquire();
            self.evaluate(node, state, &[ROOT], &mut pool, &mut output);
            output.root_motion()
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

    #[test]
    fn a_step_fires_the_events_after_the_previous_time_up_to_the_new_one() {
        let rig = events_rig(&[0.1, 0.3, 0.5]);
        let node = rig.node();
        let mut state = rig.create(&node);
        rig.fire(&node, &mut state, 0.2);

        assert_eq!(rig.fire(&node, &mut state, 0.2), [0.3]);
    }

    #[test]
    fn a_fired_event_names_its_clip_and_key_at_full_weight() {
        let rig = events_rig(&[0.1, 0.3]);
        let node = rig.node();
        let mut state = rig.create(&node);
        rig.walks.update(&rig.blackboard, |context| {
            node.update(&mut state, 0.2, clips(context), context)
        });
        let mut pool = FramePool::new(1);
        let mut output = pool.acquire();
        rig.evaluate(&node, &mut state, &[ROOT], &mut pool, &mut output);

        assert_eq!(
            output.events(),
            [FiredEvent {
                clip: rig.clip.id(),
                key: 0,
                time: 0.1,
                weight: 1.0,
                mirrored: false,
            }]
        );
    }

    #[test]
    fn the_first_update_after_play_fires_the_start_time() {
        let rig = events_rig(&[0.0, 0.25, 0.3]);
        let node = rig.node();
        let mut state = rig.create(&node);

        assert_eq!(rig.fire(&node, &mut state, 0.1), [0.0]);
        assert!(rig.fire(&node, &mut state, 0.1).is_empty());
        state.play();
        assert_eq!(rig.fire(&node, &mut state, 0.1), [0.0]);

        let node = rig.node().with_start_time(0.25);
        let mut state = rig.create(&node);
        assert_eq!(rig.fire(&node, &mut state, 0.1), [0.25, 0.3]);
    }

    #[test]
    fn a_reset_clip_fires_its_start_time_again() {
        let rig = events_rig(&[0.0]);
        let node = rig.node();
        let mut state = rig.create(&node);
        rig.fire(&node, &mut state, 0.1);

        node.reset(&mut state);

        assert_eq!(rig.fire(&node, &mut state, 0.1), [0.0]);
    }

    #[test]
    fn a_loop_wrap_fires_to_the_end_and_then_the_start() {
        let rig = events_rig(&[0.2, 0.5, 0.9, 1.0]);
        let node = rig.node().with_start_time(0.2);
        let mut state = rig.create(&node);

        assert_eq!(rig.fire(&node, &mut state, 0.4), [0.2, 0.5]);
        assert_eq!(rig.fire(&node, &mut state, 0.3), [0.9]);
        assert_eq!(rig.fire(&node, &mut state, 0.4), [1.0, 0.2]);
        assert_eq!(rig.fire(&node, &mut state, 0.4), [0.5]);
    }

    #[test]
    fn a_fresh_loop_that_wraps_on_its_first_update_fires_its_start_time_once() {
        let rig = events_rig(&[0.0, 0.25]);
        let node = rig.node();
        let mut state = rig.create(&node);

        assert_eq!(rig.fire(&node, &mut state, 1.5), [0.0, 0.25]);

        let node = rig.node().with_start_time(0.25);
        let mut state = rig.create(&node);
        assert_eq!(rig.fire(&node, &mut state, 1.5), [0.25]);
    }

    #[test]
    fn a_play_once_clip_fires_up_to_its_end_and_then_nothing() {
        let rig = events_rig(&[0.9, 1.0]);
        let node = rig.node().with_play_mode(AnimationPlayMode::PlayOnce);
        let mut state = rig.create(&node);
        rig.fire(&node, &mut state, 0.75);

        assert_eq!(rig.fire(&node, &mut state, 0.5), [0.9, 1.0]);
        assert!(rig.fire(&node, &mut state, 0.5).is_empty());
    }

    #[test]
    fn a_paused_clip_fires_nothing_and_drops_what_it_fired_before() {
        let rig = events_rig(&[0.0, 0.1]);
        let node = rig.node();
        let mut state = rig.create(&node);
        rig.walks.update(&rig.blackboard, |context| {
            node.update(&mut state, 0.2, clips(context), context)
        });
        state.is_paused = true;

        assert!(rig.fire(&node, &mut state, 0.2).is_empty());
    }

    #[test]
    fn a_clip_at_rate_zero_fires_nothing_and_keeps_its_start_for_later() {
        let mut rig = events_rig(&[0.0, 0.1]);
        rig.blackboard.set(name!("speed"), 0.0_f32);
        let node = rig.node().with_play_rate_param(name!("speed"));
        let mut state = rig.create(&node);

        assert!(rig.fire(&node, &mut state, 0.2).is_empty());
        rig.blackboard.set(name!("speed"), 1.0_f32);
        assert_eq!(rig.fire(&node, &mut state, 0.2), [0.0, 0.1]);
    }

    #[test]
    fn a_clip_that_is_not_loaded_fires_nothing_and_keeps_its_start_for_later() {
        let rig = events_rig(&[0.0]);
        let node = AnimationClipNode::new(AssetHandle::weak(AssetId::new()));
        let mut state = rig.create(&node);
        rig.fire(&node, &mut state, 0.1);

        assert!(state.fresh);
    }

    #[test]
    fn an_event_exactly_at_the_new_time_fires_once() {
        let rig = events_rig(&[0.5]);
        let node = rig.node();
        let mut state = rig.create(&node);
        rig.fire(&node, &mut state, 0.25);

        assert_eq!(rig.fire(&node, &mut state, 0.25), [0.5]);
        assert!(rig.fire(&node, &mut state, 0.25).is_empty());
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

    fn assert_motion(actual: RootMotion, translation: Vec3, rotation: Quat) {
        assert!(
            actual.translation.abs_diff_eq(translation, 1e-4),
            "expected translation {translation}, got {}",
            actual.translation
        );
        assert!(
            actual.rotation.angle_between(rotation) < 1e-3,
            "expected rotation {rotation}, got {}",
            actual.rotation
        );
    }

    fn held(rotation: Quat) -> AnimationChannel {
        rotation_channel(rotation, rotation)
    }

    fn assert_rotation(actual: Quat, expected: Quat) {
        assert!(
            actual.angle_between(expected) < 1e-3,
            "expected rotation {expected}, got {actual}"
        );
    }

    #[test]
    fn a_clip_gives_up_the_yaw_of_its_root_and_keeps_the_pitch() {
        let rest = turned_armature();
        let pitch = Quat::from_rotation_x(0.4);
        let rig = Rig::new(
            clip(vec![held(Quat::from_rotation_y(1.0) * pitch * rest)]),
            at_rest(rest),
        );

        assert_rotation(rig.pose(&[ROOT]).joints()[0].rotation, pitch * rest);
    }

    #[test]
    fn a_clip_measures_its_yaw_about_the_up_of_the_root() {
        let rest = turned_armature();
        let pitch = Quat::from_rotation_x(0.4);
        let rig = Rig::new(
            clip(vec![held(Quat::from_rotation_z(1.0) * pitch * rest)]),
            at_rest(rest),
        )
        .with_up(Vec3::Z);

        assert_rotation(rig.pose(&[ROOT]).joints()[0].rotation, pitch * rest);
    }

    #[test]
    fn a_clip_keeps_the_height_of_its_root_and_leaves_the_ground_translation_at_rest() {
        let keyed = Vec3::new(3.0, 1.75, -2.0);
        let rig = Rig::new(
            clip(vec![translation_channel(keyed, keyed)]),
            at_rest(Quat::IDENTITY),
        );

        assert_eq!(
            rig.pose(&[ROOT]).joints()[0].translation,
            Vec3::new(0.0, 1.75, 0.0)
        );
    }

    #[test]
    fn a_clip_reads_the_height_of_its_root_along_up() {
        let keyed = Vec3::new(3.0, 5.0, 0.5);
        let rig = Rig::new(
            clip(vec![translation_channel(keyed, keyed)]),
            at_rest(Quat::IDENTITY),
        )
        .with_up(Vec3::Z);

        assert_eq!(
            rig.pose(&[ROOT]).joints()[0].translation,
            Vec3::new(0.0, 1.0, 0.5)
        );
    }

    #[test]
    fn a_joint_that_is_not_the_root_keeps_what_the_clip_gives_it() {
        let moved = Vec3::new(3.0, 2.0, 1.0);
        let turned = Quat::from_rotation_y(1.0);
        let mut both = clip(vec![held(turned)]);
        both.add_channel(OTHER, translation_channel(moved, moved));
        both.add_channel(OTHER, held(turned));
        let rig = Rig::new(both, at_rest(Quat::IDENTITY));

        let pose = rig.pose(&[ROOT, OTHER]);

        assert_eq!(pose.joints()[1].translation, moved);
        assert_rotation(pose.joints()[1].rotation, turned);
    }

    #[test]
    fn blending_two_clips_that_turn_differently_leaves_no_yaw() {
        let rest = turned_armature();
        let pitch = Quat::from_rotation_x(0.4);
        let turning_by = |angle: f32| {
            let turned = Quat::from_rotation_y(angle) * pitch * rest;
            Rig::new(clip(vec![held(turned)]), at_rest(rest)).pose(&[ROOT])
        };

        let mut blended = turning_by(1.0);
        blended.blend(&turning_by(-0.6), 0.5);

        assert_rotation(blended.joints()[0].rotation, pitch * rest);
    }

    #[test]
    fn a_clip_delta_is_the_difference_of_two_samples_measured_from_a_turned_rest() {
        let rest = turned_armature();
        let rig = Rig::new(
            clip(vec![
                translation_channel(Vec3::ZERO, Vec3::new(2.0, 0.0, 4.0)),
                rotation_channel(rest, Quat::from_rotation_y(1.0) * rest),
            ]),
            at_rest(rest),
        );
        let node = rig.node();
        let mut state = rig.create(&node);

        assert_motion(
            rig.step(&node, &mut state, 0.25),
            Vec3::new(0.5, 0.0, 1.0),
            Quat::from_rotation_y(0.25),
        );
        assert_motion(
            rig.step(&node, &mut state, 0.25),
            Quat::from_rotation_y(-0.25) * Vec3::new(0.5, 0.0, 1.0),
            Quat::from_rotation_y(0.25),
        );
    }

    #[test]
    fn a_root_with_no_rotation_channel_reads_as_its_rest() {
        let rest = turned_armature();
        let moving = translation_channel(Vec3::ZERO, Vec3::new(2.0, 0.0, 4.0));
        let rig = Rig::new(clip(vec![moving]), at_rest(rest));
        let node = rig.node();
        let mut state = rig.create(&node);

        assert_motion(
            rig.step(&node, &mut state, 0.5),
            Vec3::new(1.0, 0.0, 2.0),
            Quat::IDENTITY,
        );
    }

    #[test]
    fn a_root_with_a_one_key_rotation_channel_reads_as_its_rest() {
        let rest = turned_armature();
        let moving = translation_channel(Vec3::ZERO, Vec3::new(2.0, 0.0, 4.0));
        let held = AnimationChannel::new(
            vec![0.0],
            AnimationChanelOutput::Rotation(vec![Quat::from_rotation_y(1.0)]),
        );
        let rig = Rig::new(clip(vec![moving, held]), at_rest(rest));
        let node = rig.node();
        let mut state = rig.create(&node);

        assert_motion(
            rig.step(&node, &mut state, 0.5),
            Vec3::new(1.0, 0.0, 2.0),
            Quat::IDENTITY,
        );
    }

    #[test]
    fn a_clip_with_no_channels_for_the_root_gives_no_root_motion() {
        let mut other_bone_only = AnimationClip::default();
        other_bone_only.add_channel(OTHER, translation_channel(Vec3::ZERO, Vec3::X));
        let rig = Rig::new(other_bone_only, at_rest(Quat::IDENTITY));
        let node = rig.node();
        let mut state = rig.create(&node);

        assert_eq!(
            rig.step(&node, &mut state, 0.5),
            RootMotion::IDENTITY,
            "a root the clip does not animate has not moved"
        );
    }

    #[test]
    fn a_delta_turned_by_the_root_reads_forward_from_the_way_it_faced_last_frame() {
        let rest = turned_armature();
        let facing_x = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let rig = Rig::new(
            clip(vec![
                translation_channel(Vec3::ZERO, Vec3::new(2.0, 0.0, 0.0)),
                rotation_channel(facing_x * rest, facing_x * rest),
            ]),
            at_rest(rest),
        );
        let node = rig.node();
        let mut state = rig.create(&node);

        assert_motion(
            rig.step(&node, &mut state, 0.5),
            Vec3::new(0.0, 0.0, 1.0),
            Quat::IDENTITY,
        );
    }

    #[test]
    fn a_loop_delta_across_the_wrap_is_the_distance_covered_and_not_a_jump_back() {
        let rig = Rig::new(
            clip(vec![
                translation_channel(Vec3::ZERO, Vec3::new(2.0, 0.0, 0.0)),
                rotation_channel(Quat::IDENTITY, Quat::from_rotation_y(1.0)),
            ]),
            at_rest(Quat::IDENTITY),
        );
        let node = rig.node().with_start_time(0.2);
        let mut state = rig.create(&node);

        rig.step(&node, &mut state, 0.4);
        rig.step(&node, &mut state, 0.3);
        assert_motion(
            rig.step(&node, &mut state, 0.4),
            Quat::from_rotation_y(-0.9) * Vec3::new(0.2, 0.0, 0.0),
            Quat::from_rotation_y(0.1),
        );
        assert_motion(
            rig.step(&node, &mut state, 0.4),
            Quat::from_rotation_y(-0.2) * Vec3::new(0.8, 0.0, 0.0),
            Quat::from_rotation_y(0.4),
        );
    }

    #[test]
    fn a_clip_restarted_after_a_wrap_gives_no_root_motion() {
        let moving = translation_channel(Vec3::ZERO, Vec3::new(2.0, 0.0, 0.0));
        let rig = Rig::new(clip(vec![moving]), at_rest(Quat::IDENTITY));
        let node = rig.node();

        let mut state = rig.create(&node);
        rig.step(&node, &mut state, 0.8);
        rig.step(&node, &mut state, 0.8);
        node.reset(&mut state);
        assert_eq!(rig.motion(&node, &mut state), RootMotion::IDENTITY);

        let mut state = rig.create(&node);
        rig.step(&node, &mut state, 0.8);
        rig.step(&node, &mut state, 0.8);
        state.play();
        assert_eq!(rig.motion(&node, &mut state), RootMotion::IDENTITY);
    }

    #[test]
    fn a_start_time_gives_no_delta_on_the_first_evaluation() {
        let moving = translation_channel(Vec3::ZERO, Vec3::new(2.0, 0.0, 0.0));
        let rig = Rig::new(clip(vec![moving]), at_rest(Quat::IDENTITY));
        let node = rig.node().with_start_time(0.5);
        let mut state = rig.create(&node);

        assert_eq!(rig.motion(&node, &mut state), RootMotion::IDENTITY);
    }

    #[test]
    fn a_play_once_clip_gives_its_last_delta_and_then_none() {
        let moving = translation_channel(Vec3::ZERO, Vec3::new(2.0, 0.0, 0.0));
        let rig = Rig::new(clip(vec![moving]), at_rest(Quat::IDENTITY));
        let node = rig.node().with_play_mode(AnimationPlayMode::PlayOnce);
        let mut state = rig.create(&node);

        rig.step(&node, &mut state, 0.75);
        assert_motion(
            rig.step(&node, &mut state, 0.5),
            Vec3::new(0.5, 0.0, 0.0),
            Quat::IDENTITY,
        );
        assert_eq!(
            rig.step(&node, &mut state, 0.5),
            RootMotion::IDENTITY,
            "a finished clip must not repeat its last delta"
        );
    }

    #[test]
    fn a_float_param_multiplies_the_play_rate_of_a_clip() {
        let moving = translation_channel(Vec3::ZERO, Vec3::new(2.0, 0.0, 4.0));
        let mut rig = Rig::new(clip(vec![moving]), at_rest(Quat::IDENTITY));
        rig.blackboard.set(name!("draw_speed"), 3.0_f32);
        let node = rig
            .node()
            .with_play_rate(0.5)
            .with_play_rate_param(name!("draw_speed"));
        let mut state = rig.create(&node);

        assert_motion(
            rig.step(&node, &mut state, 0.2),
            Vec3::new(0.6, 0.0, 1.2),
            Quat::IDENTITY,
        );
        assert!((state.current_time() - 0.3).abs() < 1e-6);
    }

    #[test]
    fn no_param_or_one_that_is_missing_or_not_a_float_leaves_the_play_rate_alone() {
        let mut blackboard = AnimationBlackboard::default();
        blackboard.set(name!("draw_speed"), 4.0_f32);
        blackboard.set(name!("crouched"), true);

        assert_eq!(effective_play_rate(1.5, None, &blackboard), 1.5);
        assert_eq!(
            effective_play_rate(1.5, Some(name!("missing")), &blackboard),
            1.5
        );
        assert_eq!(
            effective_play_rate(1.5, Some(name!("crouched")), &blackboard),
            1.5
        );
    }

    #[test]
    fn a_negative_play_rate_holds_the_clip() {
        let moving = translation_channel(Vec3::ZERO, Vec3::new(2.0, 0.0, 4.0));
        let mut rig = Rig::new(clip(vec![moving]), at_rest(Quat::IDENTITY));
        rig.blackboard.set(name!("draw_speed"), -2.0_f32);
        let node = rig
            .node()
            .with_start_time(0.25)
            .with_play_rate_param(name!("draw_speed"));
        let mut state = rig.create(&node);

        assert_eq!(rig.step(&node, &mut state, 0.3), RootMotion::IDENTITY);
        assert_eq!(state.current_time(), 0.25);
    }

    #[test]
    fn a_play_rate_that_is_not_finite_is_zero() {
        let mut blackboard = AnimationBlackboard::default();
        blackboard.set(name!("nan"), f32::NAN);
        blackboard.set(name!("infinite"), f32::INFINITY);

        assert_eq!(
            effective_play_rate(1.0, Some(name!("nan")), &blackboard),
            0.0
        );
        assert_eq!(
            effective_play_rate(1.0, Some(name!("infinite")), &blackboard),
            0.0
        );
        assert_eq!(
            effective_play_rate(0.0, Some(name!("infinite")), &blackboard),
            0.0
        );
    }

    fn weighted(translation: Vec3, weight: f32) -> EvaluatedFrame {
        let mut frame = AnimationFrame::new(Pose::from_joints(vec![JointPose {
            translation,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        }]));
        frame.set_root_motion(RootMotion {
            translation,
            rotation: Quat::IDENTITY,
        });
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

    fn assert_translation(frame: &AnimationFrame, expected: Vec3) {
        let joint = frame.joints()[0].translation;
        assert!(
            joint.abs_diff_eq(expected, 1e-5),
            "expected joint translation {expected}, got {joint}"
        );
        let root = frame.root_motion().translation;
        assert!(
            root.abs_diff_eq(expected, 1e-5),
            "expected root motion translation {expected}, got {root}"
        );
    }

    #[test]
    fn a_blend_of_two_inputs_at_equal_weight_lands_halfway() {
        let blended = blend(&[
            weighted(Vec3::ZERO, 1.0),
            weighted(Vec3::new(4.0, 0.0, 2.0), 1.0),
        ]);

        assert_translation(&blended, Vec3::new(2.0, 0.0, 1.0));
    }

    #[test]
    fn a_blend_leans_toward_its_heavier_input() {
        let blended = blend(&[weighted(Vec3::ZERO, 3.0), weighted(Vec3::X * 4.0, 1.0)]);

        assert_translation(&blended, Vec3::X);
    }

    #[test]
    fn a_blend_of_three_inputs_is_their_weighted_average() {
        let blended = blend(&[
            weighted(Vec3::ZERO, 1.0),
            weighted(Vec3::X * 4.0, 1.0),
            weighted(Vec3::Z * 4.0, 2.0),
        ]);

        assert_translation(&blended, Vec3::new(1.0, 0.0, 2.0));
    }

    #[test]
    fn an_input_at_zero_weight_has_no_part_in_the_blend() {
        let blended = blend(&[weighted(Vec3::X * 4.0, 0.0), weighted(Vec3::Z * 2.0, 1.0)]);

        assert_translation(&blended, Vec3::Z * 2.0);
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
    fn a_blend_gives_each_input_event_the_weight_of_its_input() {
        let blended = blend(&[firing(0, 0.25), firing(1, 0.75)]);

        assert_eq!(
            event_weights(&blended),
            [key_weight(0, 0.25), key_weight(1, 0.75)]
        );
    }

    #[test]
    fn an_input_at_zero_weight_fires_nothing() {
        let blended = blend(&[firing(0, 0.0), firing(1, 1.0)]);

        assert_eq!(event_weights(&blended), [key_weight(1, 1.0)]);
    }

    #[test]
    fn a_blend_with_no_weighted_input_keeps_the_rest_pose() {
        let blended = blend(&[weighted(Vec3::X * 4.0, 0.0)]);

        assert_translation(&blended, Vec3::ZERO);
    }
}
