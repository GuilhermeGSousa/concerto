use concerto_ecs::{CommandQueue, Entity, signal::EntitySignal, system::input::Inputs};
use concerto_mesh::Skeleton;
use concerto_reflect::Reflect;
use glam::{Affine3A, Vec3};

use crate::{
    blackboard::AnimationBlackboard,
    message::AnimationMessage,
    pose::{EvaluatedFrame, JointPose},
};

/// The stack of frames a graph evaluation passes from each node to the node that consumes it.
///
/// Nodes are evaluated in post order, so a node's inputs are the top entries of the stack, first
/// input first.
pub struct AnimationGraphEvaluator {
    pub(crate) evaluation_stack: Vec<EvaluatedFrame>,
}

impl AnimationGraphEvaluator {
    /// An empty stack.
    pub fn new() -> Self {
        Self {
            evaluation_stack: Vec::new(),
        }
    }

    /// Pushes a node's evaluated frame.
    pub fn push_evaluation(&mut self, evaluated_frame: EvaluatedFrame) {
        self.evaluation_stack.push(evaluated_frame);
    }

    /// Pops the frame on top, or `None` if the stack is empty.
    pub fn pop_evaluation(&mut self) -> Option<EvaluatedFrame> {
        self.evaluation_stack.pop()
    }

    /// The number of frames on the stack.
    pub fn stack_len(&self) -> usize {
        self.evaluation_stack.len()
    }

    /// The frames from index `start` to the top, oldest first.
    ///
    /// # Panics
    ///
    /// Panics if `start` is greater than [`Self::stack_len`].
    pub fn view_stack(&self, start: usize) -> &[EvaluatedFrame] {
        &self.evaluation_stack[start..]
    }
}

impl Default for AnimationGraphEvaluator {
    fn default() -> Self {
        Self::new()
    }
}

/// The bone root motion is read from: its index in the pose, its rest joint and the unit up of its parent's space.
#[derive(Clone, Debug, PartialEq)]
pub struct RootBone {
    /// The bone's index in the skeleton's bone list, which is also its index in the pose.
    pub index: usize,
    /// The bone's rest pose, local to its parent.
    pub rest: JointPose,
    /// The unit up direction of the bone's parent space.
    pub up: Vec3,
}

type TriggerFn = fn(&dyn Reflect, Entity, &mut CommandQueue);

fn trigger_message<M: AnimationMessage + EntitySignal>(
    message: &dyn Reflect,
    entity: Entity,
    cmd: &mut CommandQueue,
) {
    if let Some(message) = message.downcast_ref::<M>() {
        cmd.entity(entity).trigger(message.clone());
    }
}

pub(crate) struct RaisedMessage {
    message: Box<dyn Reflect>,
    trigger: Option<TriggerFn>,
}

impl RaisedMessage {
    pub(crate) fn message(&self) -> &dyn Reflect {
        self.message.as_ref()
    }

    pub(crate) fn trigger(&self, entity: Entity, cmd: &mut CommandQueue) {
        if let Some(trigger) = self.trigger {
            trigger(self.message.as_ref(), entity, cmd);
        }
    }
}

/// What every node of one player's graph is walked with: the player's own data and the walk's node inputs.
pub struct AnimationGraphContext<'a> {
    pub(crate) blackboard: &'a AnimationBlackboard,
    pub(crate) root: Option<RootBone>,
    pub(crate) skeleton: Option<&'a Skeleton>,
    pub(crate) model_to_world: Option<Affine3A>,
    pub(crate) inputs: &'a Inputs<'a, 'a>,
    pub(crate) messages: &'a [Box<dyn Reflect>],
    pub(crate) raised: Vec<RaisedMessage>,
}

impl<'a> AnimationGraphContext<'a> {
    /// The blackboard of the player being walked.
    pub fn blackboard(&self) -> &AnimationBlackboard {
        self.blackboard
    }

    /// In `update`, the messages the game sent the player since its last update, in the order
    /// sent; empty in every other walk.
    ///
    /// The game sends them with [`AnimationPlayer::send`](crate::player::AnimationPlayer::send)
    /// or [`SendAnimationMessage`](crate::message::SendAnimationMessage). A message a node raised
    /// is never among them.
    pub fn messages(&self) -> impl Iterator<Item = &'a dyn Reflect> + use<'a> {
        self.messages.iter().map(|message| message.as_ref())
    }

    /// The messages of type `M` among [`Self::messages`], in the order sent.
    pub fn message<M: AnimationMessage>(&self) -> impl Iterator<Item = &'a M> + use<'a, M> {
        self.messages()
            .filter_map(|message| message.downcast_ref::<M>())
    }

    /// In `update`, queues `message` for every state machine and retarget this node plays in,
    /// which can act on it, and, once it leaves the player's graph, triggers it on the player's
    /// entity.
    pub fn raise<M: AnimationMessage + EntitySignal>(&mut self, message: M) {
        self.pass_up(RaisedMessage {
            message: Box::new(message),
            trigger: Some(trigger_message::<M>),
        });
    }

    pub(crate) fn raise_boxed(&mut self, message: Box<dyn Reflect>) {
        self.pass_up(RaisedMessage {
            message,
            trigger: None,
        });
    }

    pub(crate) fn pass_up(&mut self, message: RaisedMessage) {
        self.raised.push(message);
    }

    pub(crate) fn child(&self) -> AnimationGraphContext<'a> {
        AnimationGraphContext {
            blackboard: self.blackboard,
            root: self.root.clone(),
            skeleton: self.skeleton,
            model_to_world: self.model_to_world,
            inputs: self.inputs,
            messages: self.messages,
            raised: Vec::new(),
        }
    }

    pub(crate) fn into_raised(self) -> Vec<RaisedMessage> {
        self.raised
    }

    /// The root bone of the player being evaluated, if it has one.
    pub fn root(&self) -> Option<&RootBone> {
        self.root.as_ref()
    }

    /// The skeleton of the player being walked, once its asset is loaded.
    pub fn skeleton(&self) -> Option<&'a Skeleton> {
        self.skeleton
    }

    /// In `evaluate`, the world transform of the skeleton's model space as of this frame.
    ///
    /// It is taken after the character moved by its root motion: the world transform of bone
    /// 0's parent, or identity when bone 0 has none.
    pub fn model_to_world(&self) -> Option<Affine3A> {
        self.model_to_world
    }
}
