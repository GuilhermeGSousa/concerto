use std::{
    any::{Any, TypeId},
    cell::RefCell,
    collections::HashMap,
};

use concerto_ecs::{CommandQueue, Entity, signal::EntitySignal, system::input::Inputs};
use concerto_mesh::Skeleton;
use glam::{Affine3A, Vec3};

use crate::{
    blackboard::AnimationBlackboard,
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

/// What every node of one player's graph is walked with: the player's own data and the walk's node inputs.
pub struct AnimationGraphContext<'a> {
    pub(crate) blackboard: &'a AnimationBlackboard,
    pub(crate) root: Option<RootBone>,
    pub(crate) skeleton: Option<&'a Skeleton>,
    pub(crate) model_to_world: Option<Affine3A>,
    pub(crate) inputs: &'a Inputs<'a, 'a>,
    pub(crate) extras: &'a Extras,
    pub(crate) outbox: &'a Outbox,
}

impl<'a> AnimationGraphContext<'a> {
    pub fn blackboard(&self) -> &AnimationBlackboard {
        self.blackboard
    }

    /// Queues `signal`, triggered on the player's entity when `animate_targets` next runs.
    pub fn emit<S: EntitySignal + Clone>(&self, signal: S) {
        self.outbox.signals.borrow_mut().push(Box::new(signal));
    }

    /// The value of type `T` the player was handed with `AnimationPlayer::set_extra`.
    pub fn extra<T: Send + Sync + 'static>(&self) -> Option<&'a T> {
        self.extras.get::<T>()
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
