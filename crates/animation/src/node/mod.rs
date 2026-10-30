use concerto_foundation::assets::handle::AssetHandle;
use concerto_reflect::Reflect;
use uuid::Uuid;

use crate::{
    clip::AnimationClip,
    evaluation::AnimationGraphContext,
    pose::{EvaluatedPose, Pose, PosePool},
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
    start_time: f32,
    is_paused: bool,
    play_rate: f32,
    is_finished: bool,
}

impl AnimationClipNodeInstance {
    pub fn new() -> Self {
        Self {
            time: 0.0,
            start_time: 0.0,
            is_paused: false,
            play_rate: 1.0,
            is_finished: false,
        }
    }

    pub fn with_start_time(mut self, start_time: f32) -> Self {
        self.start_time = start_time;
        self.time = start_time;
        self
    }

    pub fn play(&mut self) {
        self.time = self.start_time;
        self.is_paused = false;
        self.is_finished = false;
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
    }

    fn reset(&self, instance: &mut AnimationClipNodeInstance) {
        instance.time = instance.start_time;
        instance.is_finished = false;
        instance.is_paused = false;
    }

    fn is_finished(&self, instance: &AnimationClipNodeInstance) -> bool {
        instance.is_finished
    }
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
