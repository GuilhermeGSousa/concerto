use concerto_foundation::assets::handle::AssetHandle;
use concerto_reflect::Reflect;
use uuid::Uuid;

use crate::{
    clip::{AnimationChannel, AnimationClip},
    evaluation::{AnimationGraphContext, RootBone},
    pose::{EvaluatedPose, JointPose, Pose, PosePool, RootMotion},
};

pub mod blend_space;
pub mod definition;
pub mod state_machine;

pub use definition::{
    AnimationNode, ErasedAnimationNode, NodeInstance, PlaceholderNode, ReflectAnimationNode,
};

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

/// Passes its first input through.
#[derive(Default, Reflect)]
#[reflect(AnimationNode)]
pub struct BlendNode {}

fn pass_through(evaluated_inputs: &[EvaluatedPose], output: &mut Pose) {
    if let Some(input) = evaluated_inputs.first() {
        output.copy_from(&input.pose);
    }
}

impl AnimationNode for ResultNode {
    type Instance = ();

    fn create_instance(&self, _context: &AnimationGraphContext<'_>) {}

    fn update(&self, _: &mut (), _: f32, _: &AnimationGraphContext<'_>) {}

    fn evaluate(
        &self,
        _: &mut (),
        _: &AnimationGraphContext<'_>,
        _: &[Uuid],
        evaluated_inputs: &[EvaluatedPose],
        _: &mut PosePool,
        output: &mut Pose,
    ) {
        pass_through(evaluated_inputs, output);
    }

    fn reset(&self, _: &mut ()) {}
}

impl AnimationNode for BlendNode {
    type Instance = ();

    fn create_instance(&self, _context: &AnimationGraphContext<'_>) {}

    fn update(&self, _: &mut (), _: f32, _: &AnimationGraphContext<'_>) {}

    fn evaluate(
        &self,
        _: &mut (),
        _: &AnimationGraphContext<'_>,
        _: &[Uuid],
        evaluated_inputs: &[EvaluatedPose],
        _: &mut PosePool,
        output: &mut Pose,
    ) {
        pass_through(evaluated_inputs, output);
    }

    fn reset(&self, _: &mut ()) {}
}

pub struct AnimationClipNodeInstance {
    time: f32,
    previous_time: f32,
    wrapped: bool,
    start_time: f32,
    is_paused: bool,
    play_rate: f32,
    is_finished: bool,
}

impl AnimationClipNodeInstance {
    pub fn new() -> Self {
        Self {
            time: 0.0,
            previous_time: 0.0,
            wrapped: false,
            start_time: 0.0,
            is_paused: false,
            play_rate: 1.0,
            is_finished: false,
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
    }

    pub fn current_time(&self) -> f32 {
        self.time
    }
}

impl Default for AnimationClipNodeInstance {
    fn default() -> Self {
        Self::new()
    }
}

impl AnimationNode for AnimationClipNode {
    type Instance = AnimationClipNodeInstance;

    fn create_instance(&self, _context: &AnimationGraphContext<'_>) -> AnimationClipNodeInstance {
        AnimationClipNodeInstance::new().with_start_time(self.start_time)
    }

    fn update(
        &self,
        instance: &mut AnimationClipNodeInstance,
        delta_time: f32,
        context: &AnimationGraphContext<'_>,
    ) {
        instance.previous_time = instance.time;
        instance.wrapped = false;

        if instance.is_finished {
            instance.is_finished = false;
        }

        if instance.is_paused {
            return;
        }

        let Some(clip) = context.animation_clips.get(&self.clip) else {
            return;
        };

        instance.time += delta_time * instance.play_rate * self.play_rate;

        match self.play_mode {
            AnimationPlayMode::Loop => {
                if instance.time > clip.duration() {
                    instance.time = instance.start_time;
                    instance.wrapped = true;
                    instance.is_finished = true;
                }
            }
            AnimationPlayMode::PlayOnce => {
                if instance.time > clip.duration() {
                    instance.is_paused = true;
                    instance.is_finished = true;
                }
            }
        }
    }

    fn evaluate(
        &self,
        instance: &mut AnimationClipNodeInstance,
        context: &AnimationGraphContext<'_>,
        bone_ids: &[Uuid],
        _evaluated_inputs: &[EvaluatedPose],
        _pool: &mut PosePool,
        output: &mut Pose,
    ) {
        let Some(animation_clip) = context.animation_clips.get(&self.clip) else {
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

                let Some(joint_pose) = output.get_joint_pose_mut(bone_index) else {
                    return;
                };

                for animation_channel in animation_channels {
                    animation_channel.sample_transform(instance.current_time(), joint_pose);
                }
            });

        output.set_root_motion(root_motion(
            instance,
            animation_clip,
            context.root(),
            bone_ids,
        ));
    }

    fn reset(&self, instance: &mut AnimationClipNodeInstance) {
        instance.restart();
        instance.is_finished = false;
        instance.is_paused = false;
    }

    fn is_finished(&self, instance: &AnimationClipNodeInstance) -> bool {
        instance.is_finished
    }
}

fn root_motion(
    instance: &AnimationClipNodeInstance,
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

    if instance.wrapped {
        stretch(instance.previous_time, clip.duration())
            .then(stretch(instance.start_time, instance.time))
    } else {
        stretch(instance.previous_time, instance.time)
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
}

#[cfg(test)]
mod tests {
    use concerto_foundation::assets::{
        asset_server::AssetServer, asset_store::AssetStore, handle::AssetHandle,
    };
    use glam::{Quat, Vec3};
    use uuid::Uuid;

    use super::{AnimationClipNode, AnimationClipNodeInstance, AnimationNode, AnimationPlayMode};
    use crate::{
        blackboard::AnimationBlackboard,
        clip::{AnimationChanelOutput, AnimationChannel, AnimationClip},
        evaluation::{AnimationGraphContext, RootBone},
        graph::AnimationGraph,
        pose::{JointPose, PosePool, RootMotion},
    };

    const ROOT: Uuid = Uuid::from_u128(1);
    const OTHER: Uuid = Uuid::from_u128(2);

    struct Rig {
        _server: AssetServer,
        clips: AssetStore<AnimationClip>,
        graphs: AssetStore<AnimationGraph>,
        blackboard: AnimationBlackboard,
        rest: JointPose,
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
                clips,
                graphs: AssetStore::new(),
                blackboard: AnimationBlackboard::default(),
                rest,
                clip: handle,
            }
        }

        fn context(&self) -> AnimationGraphContext<'_> {
            AnimationGraphContext {
                animation_clips: &self.clips,
                animation_graphs: &self.graphs,
                blackboard: &self.blackboard,
                root: Some(RootBone {
                    index: 0,
                    rest: self.rest.clone(),
                }),
            }
        }

        fn node(&self) -> AnimationClipNode {
            AnimationClipNode::new(self.clip.clone())
        }

        fn motion(
            &self,
            node: &AnimationClipNode,
            instance: &mut AnimationClipNodeInstance,
        ) -> RootMotion {
            let mut pool = PosePool::new(1);
            let mut output = pool.acquire();
            node.evaluate(
                instance,
                &self.context(),
                &[ROOT],
                &[],
                &mut pool,
                &mut output,
            );
            output.root_motion()
        }

        fn step(
            &self,
            node: &AnimationClipNode,
            instance: &mut AnimationClipNodeInstance,
            delta_time: f32,
        ) -> RootMotion {
            node.update(instance, delta_time, &self.context());
            self.motion(node, instance)
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
        let mut instance = node.create_instance(&rig.context());

        assert_motion(
            rig.step(&node, &mut instance, 0.25),
            Vec3::new(0.5, 0.0, 1.0),
            Quat::from_rotation_y(0.25),
        );
        assert_motion(
            rig.step(&node, &mut instance, 0.25),
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
        let mut instance = node.create_instance(&rig.context());

        assert_motion(
            rig.step(&node, &mut instance, 0.5),
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
        let mut instance = node.create_instance(&rig.context());

        assert_motion(
            rig.step(&node, &mut instance, 0.5),
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
        let mut instance = node.create_instance(&rig.context());

        assert_eq!(
            rig.step(&node, &mut instance, 0.5),
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
        let mut instance = node.create_instance(&rig.context());

        assert_motion(
            rig.step(&node, &mut instance, 0.5),
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
        let mut instance = node.create_instance(&rig.context());

        rig.step(&node, &mut instance, 0.4);
        rig.step(&node, &mut instance, 0.3);
        assert_motion(
            rig.step(&node, &mut instance, 0.4),
            Quat::from_rotation_y(-0.9) * Vec3::new(0.2, 0.0, 0.0),
            Quat::from_rotation_y(0.1),
        );
        assert_motion(
            rig.step(&node, &mut instance, 0.4),
            Quat::from_rotation_y(-0.2) * Vec3::new(0.8, 0.0, 0.0),
            Quat::from_rotation_y(0.4),
        );
    }

    #[test]
    fn a_clip_restarted_after_a_wrap_gives_no_root_motion() {
        let moving = translation_channel(Vec3::ZERO, Vec3::new(2.0, 0.0, 0.0));
        let rig = Rig::new(clip(vec![moving]), at_rest(Quat::IDENTITY));
        let node = rig.node();

        let mut instance = node.create_instance(&rig.context());
        rig.step(&node, &mut instance, 0.8);
        rig.step(&node, &mut instance, 0.8);
        node.reset(&mut instance);
        assert_eq!(rig.motion(&node, &mut instance), RootMotion::IDENTITY);

        let mut instance = node.create_instance(&rig.context());
        rig.step(&node, &mut instance, 0.8);
        rig.step(&node, &mut instance, 0.8);
        instance.play();
        assert_eq!(rig.motion(&node, &mut instance), RootMotion::IDENTITY);
    }

    #[test]
    fn a_start_time_gives_no_delta_on_the_first_evaluation() {
        let moving = translation_channel(Vec3::ZERO, Vec3::new(2.0, 0.0, 0.0));
        let rig = Rig::new(clip(vec![moving]), at_rest(Quat::IDENTITY));
        let node = rig.node().with_start_time(0.5);
        let mut instance = node.create_instance(&rig.context());

        assert_eq!(rig.motion(&node, &mut instance), RootMotion::IDENTITY);
    }

    #[test]
    fn a_play_once_clip_gives_its_last_delta_and_then_none() {
        let moving = translation_channel(Vec3::ZERO, Vec3::new(2.0, 0.0, 0.0));
        let rig = Rig::new(clip(vec![moving]), at_rest(Quat::IDENTITY));
        let node = rig.node().with_play_mode(AnimationPlayMode::PlayOnce);
        let mut instance = node.create_instance(&rig.context());

        rig.step(&node, &mut instance, 0.75);
        assert_motion(
            rig.step(&node, &mut instance, 0.5),
            Vec3::new(0.5, 0.0, 0.0),
            Quat::IDENTITY,
        );
        assert_eq!(
            rig.step(&node, &mut instance, 0.5),
            RootMotion::IDENTITY,
            "a finished clip must not repeat its last delta"
        );
    }
}
