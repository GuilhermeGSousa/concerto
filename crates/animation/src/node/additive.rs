use concerto_ecs::Res;
use concerto_foundation::assets::{asset_store::AssetStore, handle::AssetHandle};
use concerto_name::name;
use concerto_reflect::{Reflect, ReflectDefault};
use log::warn;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    clip::{Additive, AnimationClip},
    evaluation::AnimationGraphContext,
    node::{
        AnimationNode, AnimationPlayMode, ClipClock, ClockSpan, EditorAnimationNode, InputSlots,
        ReflectAnimationNode, effective_play_rate, effective_weight,
    },
    param::Param,
    pose::{AnimationFrame, EvaluatedFrame, FiredEvent, FramePool, apply_additive, twist_about},
};

/// Plays an additive clip's delta over its input, weighted by `weight` times the float under `weight_param`.
#[derive(Clone, Debug, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(AnimationNode, Default)]
pub struct AdditiveNode {
    pub clip: AssetHandle<AnimationClip>,
    pub weight: f32,
    pub weight_param: Option<Param<f32>>,
    pub play_mode: AnimationPlayMode,
    pub start_time: f32,
    pub play_rate: f32,
    pub play_rate_param: Option<Param<f32>>,
}

impl Default for AdditiveNode {
    fn default() -> Self {
        Self::new(AssetHandle::default())
    }
}

impl AdditiveNode {
    /// An overlay of `clip` at full weight, looping from the start of its delta at rate 1.
    pub fn new(clip: AssetHandle<AnimationClip>) -> Self {
        Self {
            clip,
            weight: 1.0,
            weight_param: None,
            play_mode: AnimationPlayMode::Loop,
            start_time: 0.0,
            play_rate: 1.0,
            play_rate_param: None,
        }
    }

    /// Sets the weight, before the param scales it.
    pub fn with_weight(mut self, weight: f32) -> Self {
        self.weight = weight;
        self
    }

    /// Multiplies the weight by the `f32` blackboard param named `param`, when it holds one.
    pub fn with_weight_param(mut self, param: impl Into<Param<f32>>) -> Self {
        self.weight_param = Some(param.into());
        self
    }

    /// Sets whether the delta loops or plays once.
    pub fn with_play_mode(mut self, play_mode: AnimationPlayMode) -> Self {
        self.play_mode = play_mode;
        self
    }

    /// Sets where the delta starts and loops back to, in seconds from the start of its range.
    pub fn with_start_time(mut self, start_time: f32) -> Self {
        self.start_time = start_time;
        self
    }

    /// Plays the delta `play_rate` times faster than authored; never below 0.
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

/// An [`AdditiveNode`]'s clock over the delta, and which warnings it has given.
pub struct AdditiveNodeState {
    clock: ClipClock,
    warned_plain: bool,
    warned_inputs: bool,
}

impl AdditiveNodeState {
    /// Restarts the delta from the node's start time.
    pub fn play(&mut self) {
        self.clock.play();
    }
}

fn delta_span(clip: &AnimationClip) -> Option<ClockSpan> {
    let Additive::LocalSpace(base) = clip.additive() else {
        return None;
    };
    let length = clip.additive_duration();
    if length <= 0.0 {
        return None;
    }
    Some(ClockSpan {
        length,
        offset: base.range.map_or(0.0, |range| range.start),
    })
}

impl AnimationNode for AdditiveNode {
    type State = AdditiveNodeState;
    type CreateInput = ();
    type UpdateInput = Res<'static, AssetStore<AnimationClip>>;
    type EvaluateInput = Res<'static, AssetStore<AnimationClip>>;

    fn create_state(&self, _: &(), _context: &AnimationGraphContext<'_>) -> AdditiveNodeState {
        AdditiveNodeState {
            clock: ClipClock::new(self.start_time),
            warned_plain: false,
            warned_inputs: false,
        }
    }

    fn update(
        &self,
        state: &mut AdditiveNodeState,
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
        let Some(span) = delta_span(clip) else {
            return;
        };
        let rate = effective_play_rate(self.play_rate, self.play_rate_param, context.blackboard());
        state
            .clock
            .advance(clip, span, delta_time, rate, &self.play_mode);
    }

    fn evaluate(
        &self,
        state: &mut AdditiveNodeState,
        clips: &Res<AssetStore<AnimationClip>>,
        context: &AnimationGraphContext<'_>,
        bone_ids: &[Uuid],
        evaluated_inputs: &[EvaluatedFrame],
        _: &mut FramePool,
        output: &mut AnimationFrame,
    ) {
        if evaluated_inputs.len() > 1 && !state.warned_inputs {
            warn!(
                "an additive node has {} inputs and ignores all after the first",
                evaluated_inputs.len()
            );
            state.warned_inputs = true;
        }
        if let Some(base) = evaluated_inputs.first() {
            output.copy_from(&base.frame);
        }
        let Some(clip) = clips.get(&self.clip) else {
            return;
        };
        if matches!(clip.additive(), Additive::None) {
            if !state.warned_plain {
                warn!(
                    "additive node clip {:?} is not additive, so the node passes its input through",
                    self.clip.id()
                );
                state.warned_plain = true;
            }
            return;
        }
        if clip.additive_duration() <= 0.0 {
            return;
        }
        let weight = effective_weight(self.weight, self.weight_param, context.blackboard());
        if weight <= 0.0 {
            return;
        }

        for crossed in &state.clock.fired {
            output.push_event(FiredEvent {
                clip: self.clip.id(),
                key: crossed.key,
                time: crossed.time,
                weight,
                mirrored: false,
            });
        }

        let time = state.clock.current_time();
        let root = context.root();
        for (bone_index, bone) in bone_ids.iter().enumerate() {
            let Some(joint) = output.joint_mut(bone_index) else {
                continue;
            };
            let mut delta = clip.sample_additive(time, bone);
            if let Some(root) = root.filter(|root| root.index == bone_index) {
                delta.rotation = twist_about(delta.rotation, root.up).inverse() * delta.rotation;
                delta.translation = root.up * delta.translation.dot(root.up);
            }
            apply_additive(joint, &delta, weight);
        }
    }

    fn reset(&self, state: &mut AdditiveNodeState) {
        state.clock.reset();
    }
}

impl EditorAnimationNode for AdditiveNode {
    fn input_slots(&self) -> InputSlots {
        InputSlots::Fixed(vec![name!("pose")])
    }
}

#[cfg(test)]
mod tests {
    use concerto_ecs::Res;
    use concerto_foundation::assets::{AssetId, asset_store::AssetStore, handle::AssetHandle};
    use concerto_name::name;
    use glam::{Quat, Vec3};
    use uuid::Uuid;

    use super::{AdditiveNode, AdditiveNodeState};
    use crate::{
        blackboard::AnimationBlackboard,
        clip::{
            Additive, AdditiveBase, AnimationChanelOutput, AnimationChannel, AnimationClip,
            ClipRange,
        },
        evaluation::{AnimationGraphContext, RootBone},
        event::AnimationEvent,
        node::AnimationNode,
        pose::{AnimationFrame, EvaluatedFrame, FiredEvent, FramePool, JointPose, Pose},
        testing::Walks,
    };

    const ROOT: Uuid = Uuid::from_u128(1);
    const CHILD: Uuid = Uuid::from_u128(2);
    const BONES: [Uuid; 2] = [ROOT, CHILD];
    const CHILD_AT: Vec3 = Vec3::new(0.0, 0.5, 0.0);

    fn turn() -> Quat {
        Quat::from_rotation_z(0.4)
    }

    fn joint(translation: Vec3, rotation: Quat) -> JointPose {
        JointPose {
            translation,
            rotation,
            scale: Vec3::ONE,
        }
    }

    fn rotations(times: Vec<f32>, keys: Vec<Quat>) -> AnimationChannel {
        AnimationChannel::new(times, AnimationChanelOutput::Rotation(keys))
    }

    fn translations(times: Vec<f32>, keys: Vec<Vec3>) -> AnimationChannel {
        AnimationChannel::new(times, AnimationChanelOutput::Translation(keys))
    }

    fn still_rotation(rotation: Quat) -> AnimationChannel {
        rotations(vec![0.0, 1.0], vec![rotation, rotation])
    }

    fn still_translation(translation: Vec3) -> AnimationChannel {
        translations(vec![0.0, 1.0], vec![translation, translation])
    }

    fn neutral() -> AnimationClip {
        let mut clip = AnimationClip::default();
        clip.add_channel(ROOT, still_translation(Vec3::Y));
        clip.add_channel(ROOT, still_rotation(Quat::IDENTITY));
        clip.add_channel(CHILD, still_translation(CHILD_AT));
        clip.add_channel(
            CHILD,
            rotations(
                vec![0.0, 1.0],
                vec![Quat::IDENTITY, Quat::from_rotation_x(0.2)],
            ),
        );
        clip
    }

    fn add_affected_child(clip: &mut AnimationClip) {
        clip.add_channel(
            CHILD,
            translations(
                vec![0.0, 1.0],
                vec![Vec3::new(0.1, 0.5, 0.0), Vec3::new(0.3, 0.5, 0.0)],
            ),
        );
        clip.add_channel(
            CHILD,
            rotations(
                vec![0.0, 1.0],
                vec![turn(), turn() * Quat::from_rotation_x(0.2)],
            ),
        );
    }

    fn affected() -> AnimationClip {
        let mut clip = AnimationClip::default();
        clip.add_channel(ROOT, still_translation(Vec3::Y));
        clip.add_channel(ROOT, still_rotation(Quat::IDENTITY));
        add_affected_child(&mut clip);
        clip.add_event(
            0.25,
            AnimationEvent {
                name: name!("shiver"),
            },
        );
        clip
    }

    fn leaning() -> AnimationClip {
        let mut clip = AnimationClip::default();
        clip.add_channel(ROOT, still_translation(Vec3::new(0.4, 1.2, -0.3)));
        clip.add_channel(
            ROOT,
            still_rotation(Quat::from_rotation_y(0.6) * Quat::from_rotation_x(0.3)),
        );
        add_affected_child(&mut clip);
        clip
    }

    fn ranged() -> AnimationClip {
        let mut clip = AnimationClip::default();
        clip.add_channel(
            CHILD,
            translations(vec![0.0, 2.0], vec![CHILD_AT, CHILD_AT + Vec3::X * 2.0]),
        );
        clip.add_event(
            0.1,
            AnimationEvent {
                name: name!("before"),
            },
        );
        clip.add_event(
            0.75,
            AnimationEvent {
                name: name!("inside"),
            },
        );
        clip.add_event(
            1.8,
            AnimationEvent {
                name: name!("after"),
            },
        );
        clip
    }

    fn against_neutral() -> AdditiveBase {
        AdditiveBase::new(AssetHandle::weak(AssetId::from_path("neutral")))
    }

    fn ranged_base() -> AdditiveBase {
        against_neutral().with_range(ClipRange::new(0.5, 1.5))
    }

    fn built(mut clip: AnimationClip, base: AdditiveBase) -> AnimationClip {
        clip.set_frame_rate(4.0);
        clip.set_additive(Additive::LocalSpace(base));
        clip.with_delta_against(&neutral())
    }

    fn sampled(clip: &AnimationClip, bone: Uuid, time: f32) -> JointPose {
        let mut pose = JointPose::IDENTITY;
        for channel in clip.get_channels(&bone).unwrap() {
            channel.sample_transform(time, &mut pose);
        }
        pose
    }

    fn base_event() -> FiredEvent {
        FiredEvent {
            clip: AssetId::from_path("walk"),
            key: 3,
            time: 0.4,
            weight: 0.8,
            mirrored: false,
        }
    }

    fn base_at(time: f32) -> AnimationFrame {
        let neutral = neutral();
        let joints: Vec<JointPose> = BONES
            .iter()
            .map(|bone| sampled(&neutral, *bone, time))
            .collect();
        let mut frame = AnimationFrame::new(Pose::from_joints(joints));
        frame.push_event(base_event());
        frame
    }

    fn weighted(frame: AnimationFrame, weight: f32) -> EvaluatedFrame {
        EvaluatedFrame { frame, weight }
    }

    fn input(frame: AnimationFrame) -> Vec<EvaluatedFrame> {
        vec![weighted(frame, 1.0)]
    }

    fn rest() -> Vec<JointPose> {
        vec![
            joint(Vec3::Y, Quat::IDENTITY),
            joint(CHILD_AT, Quat::from_rotation_y(0.3)),
        ]
    }

    fn root_bone() -> RootBone {
        RootBone {
            index: 0,
            rest: rest()[0].clone(),
            up: Vec3::Y,
        }
    }

    fn clips<'a>(context: &'a AnimationGraphContext<'a>) -> &'a Res<'a, AssetStore<AnimationClip>> {
        context
            .inputs
            .get::<Res<'static, AssetStore<AnimationClip>>>()
            .unwrap()
    }

    struct Rig {
        walks: Walks,
        blackboard: AnimationBlackboard,
        clip: AssetHandle<AnimationClip>,
    }

    impl Rig {
        fn new(clip: AnimationClip) -> Self {
            let id = AssetId::from_path("overlay");
            let mut clips = AssetStore::<AnimationClip>::new();
            clips.insert(id, clip);
            Self {
                walks: Walks::new(clips, AssetStore::new()),
                blackboard: AnimationBlackboard::default(),
                clip: AssetHandle::weak(id),
            }
        }

        fn node(&self) -> AdditiveNode {
            AdditiveNode::new(self.clip.clone())
        }

        fn create(&self, node: &AdditiveNode) -> AdditiveNodeState {
            self.walks.create(&self.blackboard, |context| {
                AnimationNode::create_state(node, &(), context)
            })
        }

        fn step(&self, node: &AdditiveNode, state: &mut AdditiveNodeState, delta_time: f32) {
            self.walks.update(&self.blackboard, |context| {
                node.update(state, delta_time, clips(context), context)
            });
        }

        fn overlay(
            &self,
            node: &AdditiveNode,
            state: &mut AdditiveNodeState,
            inputs: &[EvaluatedFrame],
        ) -> AnimationFrame {
            let mut pool = FramePool::new(BONES.len());
            pool.set_rest(&rest());
            let mut output = pool.acquire();
            self.walks
                .evaluate(&self.blackboard, Some(root_bone()), |context| {
                    node.evaluate(
                        state,
                        clips(context),
                        context,
                        &BONES,
                        inputs,
                        &mut pool,
                        &mut output,
                    )
                });
            output
        }

        fn played(
            &self,
            node: &AdditiveNode,
            steps: &[f32],
            inputs: &[EvaluatedFrame],
        ) -> AnimationFrame {
            let mut state = self.create(node);
            for step in steps {
                self.step(node, &mut state, *step);
            }
            self.overlay(node, &mut state, inputs)
        }
    }

    fn assert_vec_near(actual: Vec3, expected: Vec3) {
        assert!(
            actual.abs_diff_eq(expected, 1e-5),
            "expected {expected}, got {actual}"
        );
    }

    fn assert_rotation_near(actual: Quat, expected: Quat) {
        assert!(
            actual.abs_diff_eq(expected, 1e-5) || actual.abs_diff_eq(-expected, 1e-5),
            "expected rotation {expected}, got {actual}"
        );
    }

    fn assert_joint_near(actual: &JointPose, expected: &JointPose) {
        assert_vec_near(actual.translation, expected.translation);
        assert_vec_near(actual.scale, expected.scale);
        assert_rotation_near(actual.rotation, expected.rotation);
    }

    struct WeightCase {
        weight: f32,
        child: JointPose,
    }

    #[test]
    fn the_child_moves_by_the_weight_times_the_delta_and_the_root_keeps_its_tilt_and_lift() {
        let base = base_at(0.5);
        let cases = [
            WeightCase {
                weight: 1.0,
                child: sampled(&affected(), CHILD, 0.5),
            },
            WeightCase {
                weight: 0.5,
                child: joint(
                    CHILD_AT + Vec3::new(0.1, 0.0, 0.0),
                    Quat::from_rotation_z(0.2) * base.joints()[1].rotation,
                ),
            },
        ];
        let rig = Rig::new(built(leaning(), against_neutral()));

        for case in cases {
            let frame = rig.played(
                &rig.node().with_weight(case.weight),
                &[0.5],
                &input(base_at(0.5)),
            );

            assert_joint_near(&frame.joints()[1], &case.child);
            assert_eq!(frame.events(), [base_event()]);
        }
        let full = rig.played(&rig.node(), &[0.5], &input(base_at(0.5)));
        assert_rotation_near(full.joints()[0].rotation, Quat::from_rotation_x(0.3));
        assert_vec_near(full.joints()[0].translation, Vec3::new(0.0, 1.2, 0.0));
    }

    #[test]
    fn a_key_in_the_range_fires_at_the_weight_and_one_outside_it_never_does() {
        let rig = Rig::new(built(ranged(), ranged_base()));
        let node = rig.node().with_weight(0.5);
        let mut state = rig.create(&node);
        let events_after = |state: &mut AdditiveNodeState, step: f32| {
            rig.step(&node, state, step);
            rig.overlay(&node, state, &input(base_at(0.0)))
                .events()
                .to_vec()
        };

        assert_eq!(events_after(&mut state, 0.2), [base_event()]);
        assert_eq!(
            events_after(&mut state, 0.1),
            [
                base_event(),
                FiredEvent {
                    clip: rig.clip.id(),
                    key: 1,
                    time: 0.75,
                    weight: 0.5,
                    mirrored: false,
                }
            ]
        );
        assert_eq!(events_after(&mut state, 0.8), [base_event()]);
        assert_eq!(events_after(&mut state, 0.1), [base_event()]);
    }
}
