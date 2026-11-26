use std::{
    any::{Any, TypeId},
    cell::RefCell,
    collections::HashMap,
    sync::Arc,
};

use concerto_ecs::{CommandQueue, Entity, signal::EntitySignal, system::input::Inputs};
use concerto_mesh::Skeleton;
use concerto_reflect::Reflect;
use glam::{Affine3A, Vec3};

use crate::{
    blackboard::AnimationBlackboard,
    message::AnimationMessage,
    pose::{EvaluatedFrame, JointPose},
};

pub struct AnimationGraphEvaluator {
    pub(crate) evaluation_stack: Vec<EvaluatedFrame>,
}

impl AnimationGraphEvaluator {
    pub fn new() -> Self {
        Self {
            evaluation_stack: Vec::new(),
        }
    }

    pub fn push_evaluation(&mut self, evaluated_frame: EvaluatedFrame) {
        self.evaluation_stack.push(evaluated_frame);
    }

    pub fn pop_evaluation(&mut self) -> Option<EvaluatedFrame> {
        self.evaluation_stack.pop()
    }

    pub fn stack_len(&self) -> usize {
        self.evaluation_stack.len()
    }

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
    pub index: usize,
    pub rest: JointPose,
    pub up: Vec3,
}

#[derive(Default)]
pub(crate) struct Extras {
    values: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl Extras {
    pub(crate) fn set<T: Send + Sync + 'static>(&mut self, value: T) {
        self.values.insert(TypeId::of::<T>(), Box::new(value));
    }

    pub(crate) fn get<T: Send + Sync + 'static>(&self) -> Option<&T> {
        self.values.get(&TypeId::of::<T>())?.downcast_ref::<T>()
    }
}

pub(crate) trait EmittedSignal: Send + Sync {
    fn trigger(self: Box<Self>, entity: Entity, cmd: &mut CommandQueue);
}

impl<S: EntitySignal> EmittedSignal for S {
    fn trigger(self: Box<Self>, entity: Entity, cmd: &mut CommandQueue) {
        cmd.entity(entity).trigger(*self);
    }
}

#[derive(Default)]
pub(crate) struct Outbox {
    signals: RefCell<Vec<Box<dyn EmittedSignal>>>,
}

impl Outbox {
    pub(crate) fn into_signals(self) -> Vec<Box<dyn EmittedSignal>> {
        self.signals.into_inner()
    }
}

#[derive(Default)]
pub(crate) struct Raised {
    messages: RefCell<Vec<Arc<dyn Reflect>>>,
}

impl Raised {
    pub(crate) fn into_messages(self) -> Vec<Arc<dyn Reflect>> {
        self.messages.into_inner()
    }
}

/// What every node of one player's graph is walked with: the player's own data and the walk's node inputs.
pub struct AnimationGraphContext<'a> {
    pub(crate) blackboard: &'a AnimationBlackboard,
    pub(crate) root: Option<RootBone>,
    pub(crate) skeleton: Option<&'a Skeleton>,
    pub(crate) model_to_world: Option<Affine3A>,
    pub(crate) inputs: &'a Inputs<'a, 'a>,
    pub(crate) extras: &'a Extras,
    pub(crate) outbox: &'a Outbox,
    pub(crate) messages: &'a [Box<dyn Reflect>],
    pub(crate) raised: Option<&'a Raised>,
}

impl<'a> AnimationGraphContext<'a> {
    pub fn blackboard(&self) -> &AnimationBlackboard {
        self.blackboard
    }

    /// Queues `signal`, triggered on the player's entity when `animate_targets` next runs.
    pub fn emit<S: EntitySignal>(&self, signal: S) {
        self.outbox.signals.borrow_mut().push(Box::new(signal));
    }

    /// The value of type `T` the player was handed with `AnimationPlayer::set_extra`.
    pub fn extra<T: Send + Sync + 'static>(&self) -> Option<&'a T> {
        self.extras.get::<T>()
    }

    /// In `update`, the messages the game sent the player since its last update, in the order sent; empty in every other walk.
    pub fn messages(&self) -> impl Iterator<Item = &'a dyn Reflect> + 'a {
        self.messages.iter().map(|message| message.as_ref())
    }

    /// The messages of type `M` among `messages`, in the order sent.
    pub fn message<M: AnimationMessage>(&self) -> impl Iterator<Item = &'a M> + 'a {
        self.messages()
            .filter_map(|message| message.downcast_ref::<M>())
    }

    /// In `update`, queues `message` for every state machine, slot and retarget this node plays in, which can act on it; dropped in every other walk and above the player's graph.
    pub fn raise<M: AnimationMessage>(&self, message: M) {
        self.raise_boxed(Box::new(message));
    }

    pub(crate) fn raise_boxed(&self, message: Box<dyn Reflect>) {
        self.raise_shared(Arc::from(message));
    }

    pub(crate) fn raise_shared(&self, message: Arc<dyn Reflect>) {
        if let Some(raised) = self.raised {
            raised.messages.borrow_mut().push(message);
        }
    }

    pub(crate) fn collecting<'b>(&'b self, raised: &'b Raised) -> AnimationGraphContext<'b> {
        AnimationGraphContext {
            blackboard: self.blackboard,
            root: self.root.clone(),
            skeleton: self.skeleton,
            model_to_world: self.model_to_world,
            inputs: self.inputs,
            extras: self.extras,
            outbox: self.outbox,
            messages: self.messages,
            raised: Some(raised),
        }
    }

    /// The root bone of the player being evaluated, if it has one.
    pub fn root(&self) -> Option<&RootBone> {
        self.root.as_ref()
    }

    /// The skeleton of the player being walked, once its asset is loaded.
    pub fn skeleton(&self) -> Option<&'a Skeleton> {
        self.skeleton
    }

    /// In `evaluate`, the world transform of the skeleton's model space as of this frame, after the character moved by its root motion: bone 0's parent's, or identity when it has none.
    pub fn model_to_world(&self) -> Option<Affine3A> {
        self.model_to_world
    }
}

/// What every node of one player's graph is driven with in one physics step: the step's length and the player's blackboard, root, skeleton, extras and outbox.
pub struct DriveContext<'a> {
    pub(crate) delta_time: f32,
    pub(crate) graph: &'a AnimationGraphContext<'a>,
}

impl<'a> DriveContext<'a> {
    /// The length of the physics step, in seconds.
    pub fn delta_time(&self) -> f32 {
        self.delta_time
    }

    /// The player's blackboard.
    pub fn blackboard(&self) -> &'a AnimationBlackboard {
        self.graph.blackboard
    }

    /// The root bone of the player being driven, if it has one.
    pub fn root(&self) -> Option<&'a RootBone> {
        self.graph.root.as_ref()
    }

    /// The skeleton of the player being driven, once its asset is loaded.
    pub fn skeleton(&self) -> Option<&'a Skeleton> {
        self.graph.skeleton
    }

    /// The value of type `T` the player was handed with `AnimationPlayer::set_extra`.
    pub fn extra<T: Send + Sync + 'static>(&self) -> Option<&'a T> {
        self.graph.extra::<T>()
    }

    /// Queues `signal`, triggered on the player's entity when `animate_targets` next runs.
    pub fn emit<S: EntitySignal>(&self, signal: S) {
        self.graph.emit(signal);
    }
}
