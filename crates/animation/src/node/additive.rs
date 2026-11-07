use concerto_ecs::Res;
use concerto_foundation::assets::{asset_store::AssetStore, handle::AssetHandle};
use concerto_name::Name;
use concerto_reflect::Reflect;
use log::warn;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    clip::{Additive, AnimationClip},
    evaluation::AnimationGraphContext,
    node::{
        AnimationNode, AnimationPlayMode, ClipClock, ClockSpan, ReflectAnimationNode,
        effective_play_rate, effective_weight,
    },
    pose::{AnimationFrame, EvaluatedFrame, FiredEvent, FramePool, apply_additive, twist_about},
};

/// Plays an additive clip's delta over its input, weighted by `weight` times the float under `weight_param`.
#[derive(Clone, Debug, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(AnimationNode)]
pub struct AdditiveNode {
    pub clip: AssetHandle<AnimationClip>,
    pub weight: f32,
    pub weight_param: Option<Name>,
    pub play_mode: AnimationPlayMode,
    pub start_time: f32,
    pub play_rate: f32,
    pub play_rate_param: Option<Name>,
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
    pub fn with_weight_param(mut self, param: Name) -> Self {
        self.weight_param = Some(param);
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
    pub fn with_play_rate_param(mut self, param: Name) -> Self {
        self.play_rate_param = Some(param);
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

#[cfg(test)]
mod tests {
    use concerto_ecs::Res;
    use concerto_foundation::assets::{AssetId, asset_store::AssetStore, handle::AssetHandle};
    use concerto_name::{Name, name};
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
        graph::{AnimationGraph, made_live},
        node::{AnimationNode, AnimationPlayMode},
        player::AnimationPlayer,
        pose::{
            AnimationFrame, EvaluatedFrame, FiredEvent, FramePool, JointPose, Pose, RootMotion,
            apply_additive,
        },
        test_walks::Walks,
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
        clip.build_delta_against(&neutral()).unwrap();
        clip
    }

    fn cold() -> AnimationClip {
        built(affected(), against_neutral())
    }

    fn sampled(clip: &AnimationClip, bone: Uuid, time: f32) -> JointPose {
        let mut pose = JointPose::IDENTITY;
        for channel in clip.get_channels(&bone).unwrap() {
            channel.sample_transform(time, &mut pose);
        }
        pose
    }

    fn base_motion() -> RootMotion {
        RootMotion {
            translation: Vec3::new(0.0, 0.0, 0.3),
            rotation: Quat::from_rotation_y(0.1),
        }
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
        frame.set_root_motion(base_motion());
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

    fn assert_same_frame(frame: &AnimationFrame, expected: &AnimationFrame) {
        assert_eq!(frame.joints(), expected.joints());
        assert_eq!(frame.root_motion(), expected.root_motion());
        assert_eq!(frame.events(), expected.events());
    }

    #[test]
    fn at_weight_0_the_output_is_the_base_bit_for_bit_events_included() {
        let mut rig = Rig::new(cold());
        rig.blackboard.set(name!("warmth"), 0.0_f32);
        let base = base_at(0.5);

        for node in [
            rig.node().with_weight(0.0),
            rig.node().with_weight_param(name!("warmth")),
        ] {
            let frame = rig.played(&node, &[0.5], &input(base_at(0.5)));

            assert_same_frame(&frame, &base);
        }
    }

    #[test]
    fn at_weight_1_the_delta_turns_the_neutral_pose_into_the_affected_one() {
        let rig = Rig::new(cold());
        let base = base_at(0.5);

        let frame = rig.played(&rig.node(), &[0.5], &input(base_at(0.5)));

        let mut expected = base.joints()[1].clone();
        apply_additive(&mut expected, &cold().sample_additive(0.5, &CHILD), 1.0);
        assert_eq!(frame.joints()[1], expected);
        assert_joint_near(&frame.joints()[1], &sampled(&affected(), CHILD, 0.5));
    }

    #[test]
    fn at_half_weight_the_child_turns_and_moves_by_half_the_delta() {
        let rig = Rig::new(cold());
        let base = base_at(0.5);

        let frame = rig.played(&rig.node().with_weight(0.5), &[0.5], &input(base_at(0.5)));

        let child = &frame.joints()[1];
        assert_rotation_near(
            child.rotation,
            Quat::from_rotation_z(0.2) * base.joints()[1].rotation,
        );
        assert_vec_near(child.translation, CHILD_AT + Vec3::new(0.1, 0.0, 0.0));
    }

    struct ParamCase {
        param: Name,
        weight: f32,
    }

    impl ParamCase {
        fn new(param: Name, weight: f32) -> Self {
            Self { param, weight }
        }
    }

    #[test]
    fn the_weight_is_the_weight_times_the_param_clamped_to_0_and_1() {
        let mut rig = Rig::new(cold());
        rig.blackboard.set(name!("half"), 0.5_f32);
        rig.blackboard.set(name!("triple"), 3.0_f32);
        rig.blackboard.set(name!("negative"), -1.0_f32);
        rig.blackboard.set(name!("nan"), f32::NAN);
        rig.blackboard.set(name!("flag"), true);
        let node = rig.node().with_weight(0.8);
        let cases = [
            ParamCase::new(name!("half"), 0.4),
            ParamCase::new(name!("unset"), 0.8),
            ParamCase::new(name!("flag"), 0.8),
            ParamCase::new(name!("triple"), 1.0),
            ParamCase::new(name!("negative"), 0.0),
            ParamCase::new(name!("nan"), 0.0),
        ];

        for case in cases {
            let frame = rig.played(
                &node.clone().with_weight_param(case.param),
                &[],
                &input(base_at(0.0)),
            );

            let moved = frame.joints()[1].translation.x - CHILD_AT.x;
            assert!(
                (moved - 0.1 * case.weight).abs() < 1e-6,
                "param {:?}: expected a move of {}, got {moved}",
                case.param,
                0.1 * case.weight
            );
        }
    }

    #[test]
    fn the_clock_keeps_running_at_weight_0() {
        let mut rig = Rig::new(cold());
        rig.blackboard.set(name!("warmth"), 0.0_f32);
        let node = rig.node().with_weight_param(name!("warmth"));
        let mut state = rig.create(&node);
        rig.step(&node, &mut state, 0.25);
        rig.overlay(&node, &mut state, &input(base_at(0.25)));
        rig.step(&node, &mut state, 0.25);

        rig.blackboard.set(name!("warmth"), 1.0_f32);
        let frame = rig.overlay(&node, &mut state, &input(base_at(0.5)));

        assert_eq!(state.clock.current_time(), 0.5);
        assert_vec_near(
            frame.joints()[1].translation,
            CHILD_AT + Vec3::new(0.2, 0.0, 0.0),
        );
    }

    #[test]
    fn the_clock_runs_over_the_span_of_the_delta_and_wraps_to_the_start_time() {
        let rig = Rig::new(built(ranged(), ranged_base()));
        let node = rig.node().with_start_time(0.25);
        let mut state = rig.create(&node);

        rig.step(&node, &mut state, 0.7);
        assert!((state.clock.current_time() - 0.95).abs() < 1e-6);
        assert!(!state.clock.wrapped);
        rig.step(&node, &mut state, 0.1);
        assert_eq!(state.clock.current_time(), 0.25);
        assert!(state.clock.wrapped);
    }

    #[test]
    fn a_play_once_overlay_finishes_at_the_end_of_its_delta_and_holds() {
        let rig = Rig::new(built(ranged(), ranged_base()));
        let node = rig.node().with_play_mode(AnimationPlayMode::PlayOnce);
        let mut state = rig.create(&node);

        rig.step(&node, &mut state, 0.9);
        assert!(!state.clock.is_finished);
        rig.step(&node, &mut state, 0.2);
        assert!(state.clock.is_finished);
        let finished_at = state.clock.current_time();
        rig.step(&node, &mut state, 0.2);
        assert_eq!(state.clock.current_time(), finished_at);
    }

    #[test]
    fn the_play_rate_times_its_param_scales_how_fast_the_delta_plays() {
        let mut rig = Rig::new(cold());
        rig.blackboard.set(name!("shiver_speed"), 3.0_f32);
        let node = rig
            .node()
            .with_play_rate(0.5)
            .with_play_rate_param(name!("shiver_speed"));
        let mut state = rig.create(&node);

        rig.step(&node, &mut state, 0.2);

        assert!((state.clock.current_time() - 0.3).abs() < 1e-6);
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

    #[test]
    fn at_weight_0_no_event_of_the_clip_is_added() {
        let rig = Rig::new(built(ranged(), ranged_base()));

        let frame = rig.played(
            &rig.node().with_weight(0.0),
            &[0.2, 0.1],
            &input(base_at(0.0)),
        );

        assert_eq!(frame.events(), [base_event()]);
    }

    #[test]
    fn the_root_keeps_the_tilt_and_the_lift_of_its_delta_and_the_base_root_motion() {
        let rig = Rig::new(built(leaning(), against_neutral()));

        let frame = rig.played(&rig.node(), &[], &input(base_at(0.0)));

        assert_rotation_near(frame.joints()[0].rotation, Quat::from_rotation_x(0.3));
        assert_vec_near(frame.joints()[0].translation, Vec3::new(0.0, 1.2, 0.0));
        assert_eq!(frame.root_motion(), base_motion());
    }

    #[test]
    fn a_plain_clip_passes_the_base_through_and_warns_once() {
        let rig = Rig::new(affected());
        let node = rig.node();
        let mut state = rig.create(&node);
        let base = base_at(0.5);

        for _ in 0..3 {
            rig.step(&node, &mut state, 0.25);
            let frame = rig.overlay(&node, &mut state, &input(base_at(0.5)));

            assert_same_frame(&frame, &base);
            assert!(state.warned_plain);
        }
        assert!(!state.warned_inputs);
    }

    #[test]
    fn an_unloaded_clip_or_one_with_no_delta_passes_the_base_through_and_holds_its_clock() {
        let mut undelta = affected();
        undelta.set_additive(Additive::LocalSpace(against_neutral()));
        let rig = Rig::new(undelta);
        let unloaded = AdditiveNode::new(AssetHandle::weak(AssetId::from_path("unloaded")));
        let base = base_at(0.5);

        for node in [unloaded, rig.node()] {
            let mut state = rig.create(&node);
            rig.step(&node, &mut state, 0.5);
            let frame = rig.overlay(&node, &mut state, &input(base_at(0.5)));

            assert_same_frame(&frame, &base);
            assert_eq!(state.clock.current_time(), 0.0);
            assert!(state.clock.fresh);
            assert!(!state.warned_plain);
        }
    }

    #[test]
    fn with_no_input_the_delta_goes_over_the_rest_pose() {
        let rig = Rig::new(cold());

        let frame = rig.played(&rig.node(), &[], &[]);

        let mut expected = rest()[1].clone();
        apply_additive(&mut expected, &cold().sample_additive(0.0, &CHILD), 1.0);
        assert_eq!(frame.joints()[1], expected);
        assert_eq!(frame.root_motion(), RootMotion::IDENTITY);
        assert!(frame.events().is_empty());
    }

    #[test]
    fn a_second_input_warns_and_is_ignored() {
        let rig = Rig::new(cold());
        let node = rig.node();
        let mut state = rig.create(&node);
        let other = AnimationFrame::new(Pose::from_joints(vec![
            joint(
                Vec3::X,
                Quat::from_rotation_y(1.0)
            );
            BONES.len()
        ]));

        let frame = rig.overlay(
            &node,
            &mut state,
            &[weighted(base_at(0.0), 1.0), weighted(other, 1.0)],
        );

        assert!(state.warned_inputs);
        assert_same_frame(&frame, &rig.played(&node, &[], &input(base_at(0.0))));
    }

    #[test]
    fn the_node_weight_of_the_input_does_not_change_the_result() {
        let rig = Rig::new(cold());
        let node = rig.node().with_weight(0.7);

        let faint = rig.played(&node, &[0.25], &[weighted(base_at(0.25), 0.25)]);
        let full = rig.played(&node, &[0.25], &input(base_at(0.25)));

        assert_same_frame(&faint, &full);
    }

    #[test]
    fn a_reset_returns_the_clock_to_the_start_time() {
        let rig = Rig::new(cold());
        let node = rig.node().with_start_time(0.25);
        let mut state = rig.create(&node);
        rig.step(&node, &mut state, 0.3);

        node.reset(&mut state);

        assert_eq!(state.clock.current_time(), 0.25);
    }

    fn overlay_times(player: &mut AnimationPlayer, walks: &Walks) -> Vec<f32> {
        let mut times = Vec::new();
        player.for_each_state_mut::<AdditiveNode>(walks.graphs(), |_, state, _| {
            times.push(state.clock.current_time());
        });
        times
    }

    #[test]
    fn animation_player_play_restarts_an_additive_node() {
        let mut rig = Rig::new(cold());
        let mut graph = AnimationGraph::new();
        let result = graph.result_node().index();
        let index = graph
            .add_node(rig.node().with_start_time(0.25), result)
            .index();
        let handle = AssetHandle::weak(AssetId::from_path("overlay_graph"));
        rig.walks.graphs_mut().insert(handle.id(), made_live(graph));
        let mut player = AnimationPlayer::new(BONES.len());
        rig.walks.initialize(&mut player, handle);
        rig.walks.advance(&mut player, 0.3);
        let advanced = overlay_times(&mut player, &rig.walks);
        assert_eq!(advanced.len(), 1);
        assert!((advanced[0] - 0.55).abs() < 1e-6);

        player.play(&index);

        assert_eq!(overlay_times(&mut player, &rig.walks), [0.25]);
    }

    #[test]
    fn an_overlay_is_always_finished() {
        let rig = Rig::new(cold());
        let node = rig.node();
        let mut state = rig.create(&node);
        assert!(node.is_finished(&state));
        for _ in 0..8 {
            rig.step(&node, &mut state, 0.3);
            assert!(node.is_finished(&state));
        }

        let plain = Rig::new(affected());
        let unloaded = AdditiveNode::new(AssetHandle::weak(AssetId::from_path("unloaded")));
        for node in [plain.node(), unloaded] {
            let mut state = plain.create(&node);
            plain.step(&node, &mut state, 0.3);
            assert!(node.is_finished(&state));
        }
    }

    #[test]
    fn a_clip_that_stops_resolving_fires_nothing_on_the_next_update() {
        let rig = Rig::new(cold());
        let node = rig.node();
        let mut state = rig.create(&node);
        rig.step(&node, &mut state, 0.5);
        assert_eq!(rig.overlay(&node, &mut state, &[]).events().len(), 1);

        let unresolved = AdditiveNode {
            clip: AssetHandle::weak(AssetId::from_path("gone")),
            ..node.clone()
        };
        rig.step(&unresolved, &mut state, 0.5);

        assert!(state.clock.fired.is_empty());
        assert!(rig.overlay(&node, &mut state, &[]).events().is_empty());
    }
}
