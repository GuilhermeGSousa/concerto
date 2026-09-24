use std::any::Any;

use concerto_core::{assets::handle::AssetHandle, utils::AsAny};
use uuid::Uuid;

use crate::{
    clip::AnimationClip,
    evaluation::AnimationGraphContext,
    pose::{EvaluatedPose, Pose, PosePool},
};

pub mod blend_space;
pub mod state_machine;

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub enum AnimationPlayMode {
    #[default]
    Loop,
    PlayOnce,
}

pub trait AnimationNodeInstance: AsAny + Sync + Send {
    fn reset(&mut self);

    fn evaluate(
        &self,
        node: &AnimationNodeKind,
        context: &AnimationGraphContext<'_>,
        bone_ids: &[Uuid],
        evaluated_inputs: &[EvaluatedPose],
        pool: &mut PosePool,
        output: &mut Pose,
    );

    fn update(
        &mut self,
        node: &AnimationNodeKind,
        delta_time: f32,
        context: &AnimationGraphContext<'_>,
    );

    fn is_finished(&self) -> bool {
        true
    }
}

/// Closed set of animation-graph node definitions. `Result` and `Blend` carry no
/// data; the others wrap a definition struct.
#[derive(serde::Serialize, serde::Deserialize)]
pub enum AnimationNodeKind {
    Result,
    // TODO(asset-trait-merge): an imported-then-loaded graph carries Weak
    // AssetHandles; needs an upgrade pass like scene components. No graph is
    // imported yet.
    Clip(AnimationClipNode),
    Blend,
    BlendSpace2D(crate::node::blend_space::BlendSpace2DNode),
    StateMachine(crate::node::state_machine::AnimationStateMachine),
}

impl AnimationNodeKind {
    pub(crate) fn create_instance(
        &self,
        creation_context: &AnimationGraphContext,
    ) -> Box<dyn AnimationNodeInstance> {
        match self {
            AnimationNodeKind::Result | AnimationNodeKind::Blend => Box::new(NoneInstance),
            AnimationNodeKind::Clip(def) => {
                Box::new(AnimationClipNodeInstance::new().with_start_time(def.start_time))
            }
            AnimationNodeKind::BlendSpace2D(def) => Box::new(
                crate::node::blend_space::BlendSpace2DInstanceNode::new(def.points()),
            ),
            AnimationNodeKind::StateMachine(fsm) => fsm.create_instance_boxed(creation_context),
        }
    }
}

#[derive(AsAny)]
pub struct NoneInstance;

impl AnimationNodeInstance for NoneInstance {
    fn reset(&mut self) {}

    fn update(
        &mut self,
        _node: &AnimationNodeKind,
        _delta_time: f32,
        _context: &AnimationGraphContext<'_>,
    ) {
    }

    fn evaluate(
        &self,
        _node: &AnimationNodeKind,
        _context: &AnimationGraphContext<'_>,
        _bone_ids: &[Uuid],
        evaluated_inputs: &[EvaluatedPose],
        _pool: &mut PosePool,
        output: &mut Pose,
    ) {
        // Pass-through: forward the first input pose if there is one, otherwise leave the
        // output at its acquired (identity) state.
        if let Some(input) = evaluated_inputs.first() {
            output.copy_from(&input.pose);
        }
    }
}

#[derive(AsAny)]
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

impl AnimationNodeInstance for AnimationClipNodeInstance {
    fn reset(&mut self) {
        self.time = self.start_time;
        self.is_finished = false;
        self.is_paused = false;
    }

    fn evaluate(
        &self,
        node: &AnimationNodeKind,
        context: &AnimationGraphContext<'_>,
        bone_ids: &[Uuid],
        _evaluated_inputs: &[EvaluatedPose],
        _pool: &mut PosePool,
        output: &mut Pose,
    ) {
        let AnimationNodeKind::Clip(clip_node) = node else {
            return;
        };
        let Some(animation_clip) = context.animation_clips.get(&clip_node.clip) else {
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
                    animation_channel.sample_transform(self.current_time(), joint_pose);
                }
            });
    }

    fn update(
        &mut self,
        node: &AnimationNodeKind,
        delta_time: f32,
        context: &AnimationGraphContext<'_>,
    ) {
        let AnimationNodeKind::Clip(clip_node) = node else {
            return;
        };

        if self.is_finished {
            self.is_finished = false;
        }

        if self.is_paused {
            return;
        }

        let Some(clip) = context.animation_clips.get(&clip_node.clip) else {
            return;
        };

        self.time += delta_time * self.play_rate;

        match clip_node.play_mode {
            AnimationPlayMode::Loop => {
                if self.time > clip.duration() {
                    self.time = self.start_time;
                    self.is_finished = true;
                }
            }
            AnimationPlayMode::PlayOnce => {
                if self.time > clip.duration() {
                    self.is_paused = true;
                    self.is_finished = true;
                }
            }
        }
    }

    fn is_finished(&self) -> bool {
        self.is_finished
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct AnimationClipNode {
    clip: AssetHandle<AnimationClip>,
    play_mode: AnimationPlayMode,
    start_time: f32,
}

impl AnimationClipNode {
    pub fn new(clip: AssetHandle<AnimationClip>) -> Self {
        Self {
            clip,
            play_mode: AnimationPlayMode::Loop,
            start_time: 0.0,
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
}
