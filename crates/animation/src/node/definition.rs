use std::any::Any;

use concerto_ecs::{
    World,
    system::input::{ReadOnlySystemInput, SystemInputData, register_input},
};
use concerto_foundation::assets::handle::{AssetHandle, LiveLeaves};
use concerto_name::{Name, name};
use concerto_reflect::{
    EncodeError, FromType, Reflect, ReflectedValue, TypeData, ValueWarning, apply_value_with,
    to_value,
};
use log::warn;
use uuid::Uuid;

use crate::{
    evaluation::AnimationGraphContext,
    graph::{AnimationGraph, AnimationGraphState},
    pose::{AnimationFrame, EvaluatedFrame, FramePool},
};

/// The runtime state of one node in one player, erased.
pub type NodeState = Box<dyn Any + Send + Sync>;

/// A kind of animation graph node: reflected data plus the per-player state it evaluates with.
pub trait AnimationNode: Reflect + Send + Sync + 'static {
    type State: Send + Sync + 'static;
    /// What the node reads to create its state.
    type CreateInput: ReadOnlySystemInput + 'static;
    /// What the node reads to advance.
    type UpdateInput: ReadOnlySystemInput + 'static;
    /// What the node reads to write its pose.
    type EvaluateInput: ReadOnlySystemInput + 'static;

    fn create_state(
        &self,
        input: &SystemInputData<'_, '_, Self::CreateInput>,
        context: &AnimationGraphContext<'_>,
    ) -> Self::State;

    fn update(
        &self,
        state: &mut Self::State,
        delta_time: f32,
        input: &SystemInputData<'_, '_, Self::UpdateInput>,
        context: &AnimationGraphContext<'_>,
    );

    fn evaluate(
        &self,
        state: &mut Self::State,
        input: &SystemInputData<'_, '_, Self::EvaluateInput>,
        context: &AnimationGraphContext<'_>,
        bone_ids: &[Uuid],
        evaluated_inputs: &[EvaluatedFrame],
        pool: &mut FramePool,
        output: &mut AnimationFrame,
    );

    fn reset(&self, state: &mut Self::State);

    fn is_finished(&self, _state: &Self::State) -> bool {
        true
    }

    /// Whether `evaluate` reads the frame of the input in slot `input`, whose node weight is `weight`; `true` unless the node says otherwise.
    fn reads_input(&self, _state: &Self::State, _input: usize, _weight: f32) -> bool {
        true
    }

    /// Whether `evaluate` does nothing but write `output`, so a graph leaves the node out while nothing reads its frame; `false` unless the node says otherwise.
    fn evaluate_only_writes_output(&self) -> bool {
        false
    }

    /// The inputs this node takes, in the order `evaluate` receives them.
    fn input_slots(&self) -> InputSlots;

    /// Called when an editor gives the node an input at `index`.
    fn input_added(&mut self, _index: usize) {}

    /// Called when an editor takes the node's input at `index` away.
    fn input_removed(&mut self, _index: usize) {}

    /// Visits the graph assets this node plays inside itself.
    fn inner_graphs(&self, _visit: &mut dyn FnMut(&AssetHandle<AnimationGraph>)) {}

    /// Visits the graph states this node's state holds for the graphs it plays inside itself, each with the state that plays it when the node is a state machine.
    fn inner_states(
        &self,
        _state: &Self::State,
        _visit: &mut dyn FnMut(Option<Name>, &AnimationGraphState),
    ) {
    }

    /// What the node shows an editor of its state as it plays; nothing unless the node says otherwise.
    fn report(&self, _state: &Self::State) -> NodeReport {
        NodeReport::default()
    }

    /// Visits, mutably, the graph states this node's state holds for the graphs it plays inside itself.
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    fn inner_states_mut(
        &self,
        _state: &mut Self::State,
        _visit: &mut dyn FnMut(&mut AnimationGraphState),
    ) {
    }
}

/// What a playing node shows an editor beyond its weight.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NodeReport {
    /// The time a clip node plays at, in seconds.
    pub clip_time: Option<f32>,
    /// A state machine's current state and transition.
    pub state_machine: Option<StateMachineReport>,
}

/// The state a playing state machine is in and the transition it is blending through.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StateMachineReport {
    pub current: Option<Name>,
    pub transition: Option<TransitionReport>,
}

/// A transition in flight; `progress` runs from 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransitionReport {
    pub from: Name,
    pub to: Name,
    pub progress: f32,
}

/// The inputs a kind of node takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputSlots {
    /// No inputs: a clip, motion matching, a state machine.
    None,
    /// A fixed set, each named: a layer's `base` and `overlay`.
    Fixed(Vec<Name>),
    /// Any number, unnamed: a blend.
    Variable,
}

/// The graph walk that creates node states; its input set holds every registered node's `CreateInput`.
pub struct CreateNodeInputs;

/// The graph walk that advances nodes; its input set holds every registered node's `UpdateInput`.
pub struct UpdateNodeInputs;

/// The graph walk that writes poses; its input set holds every registered node's `EvaluateInput`.
pub struct EvaluateNodeInputs;

/// Registers `N`'s three inputs with the graph walks of `world`, before its schedules compile.
pub fn register_node_inputs<N: AnimationNode>(world: &mut World) {
    register_input::<CreateNodeInputs, N::CreateInput>(world);
    register_input::<UpdateNodeInputs, N::UpdateInput>(world);
    register_input::<EvaluateNodeInputs, N::EvaluateInput>(world);
}

/// An [`AnimationNode`] with its state type erased; implemented for every node.
pub trait ErasedAnimationNode: Send + Sync {
    fn as_reflect(&self) -> &dyn Reflect;

    fn as_reflect_mut(&mut self) -> &mut dyn Reflect;

    fn create_state(&self, context: &AnimationGraphContext<'_>) -> Option<NodeState>;

    fn update(&self, state: &mut NodeState, delta_time: f32, context: &AnimationGraphContext<'_>);

    fn evaluate(
        &self,
        state: &mut NodeState,
        context: &AnimationGraphContext<'_>,
        bone_ids: &[Uuid],
        evaluated_inputs: &[EvaluatedFrame],
        pool: &mut FramePool,
        output: &mut AnimationFrame,
    );

    fn reset(&self, state: &mut NodeState);

    fn is_finished(&self, state: &NodeState) -> bool;

    fn reads_input(&self, state: &NodeState, input: usize, weight: f32) -> bool;

    fn evaluate_only_writes_output(&self) -> bool;

    fn inner_graphs(&self, visit: &mut dyn FnMut(&AssetHandle<AnimationGraph>));

    fn input_slots(&self) -> InputSlots;

    fn input_added(&mut self, index: usize);

    fn input_removed(&mut self, index: usize);

    fn inner_states(
        &self,
        state: &NodeState,
        visit: &mut dyn FnMut(Option<Name>, &AnimationGraphState),
    );

    fn report(&self, state: &NodeState) -> NodeReport;

    #[cfg(any(test, feature = "test-support"))]
    fn inner_states_mut(
        &self,
        state: &mut NodeState,
        visit: &mut dyn FnMut(&mut AnimationGraphState),
    );

    fn state_as_any<'a>(&self, state: &'a NodeState) -> &'a dyn Any;

    fn state_as_any_mut<'a>(&self, state: &'a mut NodeState) -> &'a mut dyn Any;
}

impl<T: AnimationNode> ErasedAnimationNode for T {
    fn as_reflect(&self) -> &dyn Reflect {
        self
    }

    fn as_reflect_mut(&mut self) -> &mut dyn Reflect {
        self
    }

    fn create_state(&self, context: &AnimationGraphContext<'_>) -> Option<NodeState> {
        let input = fetched::<T, T::CreateInput>(context)?;
        Some(Box::new(AnimationNode::create_state(self, input, context)))
    }

    fn update(&self, state: &mut NodeState, delta_time: f32, context: &AnimationGraphContext<'_>) {
        if let Some(state) = state.downcast_mut::<T::State>()
            && let Some(input) = fetched::<T, T::UpdateInput>(context)
        {
            AnimationNode::update(self, state, delta_time, input, context);
        }
    }

    fn evaluate(
        &self,
        state: &mut NodeState,
        context: &AnimationGraphContext<'_>,
        bone_ids: &[Uuid],
        evaluated_inputs: &[EvaluatedFrame],
        pool: &mut FramePool,
        output: &mut AnimationFrame,
    ) {
        if let Some(state) = state.downcast_mut::<T::State>()
            && let Some(input) = fetched::<T, T::EvaluateInput>(context)
        {
            AnimationNode::evaluate(
                self,
                state,
                input,
                context,
                bone_ids,
                evaluated_inputs,
                pool,
                output,
            );
        }
    }

    fn reset(&self, state: &mut NodeState) {
        if let Some(state) = state.downcast_mut::<T::State>() {
            AnimationNode::reset(self, state);
        }
    }

    fn is_finished(&self, state: &NodeState) -> bool {
        state
            .downcast_ref::<T::State>()
            .is_none_or(|state| AnimationNode::is_finished(self, state))
    }

    fn reads_input(&self, state: &NodeState, input: usize, weight: f32) -> bool {
        state
            .downcast_ref::<T::State>()
            .is_none_or(|state| AnimationNode::reads_input(self, state, input, weight))
    }

    fn evaluate_only_writes_output(&self) -> bool {
        AnimationNode::evaluate_only_writes_output(self)
    }

    fn inner_graphs(&self, visit: &mut dyn FnMut(&AssetHandle<AnimationGraph>)) {
        AnimationNode::inner_graphs(self, visit);
    }

    fn input_slots(&self) -> InputSlots {
        AnimationNode::input_slots(self)
    }

    fn input_added(&mut self, index: usize) {
        AnimationNode::input_added(self, index);
    }

    fn input_removed(&mut self, index: usize) {
        AnimationNode::input_removed(self, index);
    }

    fn inner_states(
        &self,
        state: &NodeState,
        visit: &mut dyn FnMut(Option<Name>, &AnimationGraphState),
    ) {
        if let Some(state) = state.downcast_ref::<T::State>() {
            AnimationNode::inner_states(self, state, visit);
        }
    }

    fn report(&self, state: &NodeState) -> NodeReport {
        state
            .downcast_ref::<T::State>()
            .map_or_else(NodeReport::default, |state| {
                AnimationNode::report(self, state)
            })
    }

    #[cfg(any(test, feature = "test-support"))]
    fn inner_states_mut(
        &self,
        state: &mut NodeState,
        visit: &mut dyn FnMut(&mut AnimationGraphState),
    ) {
        if let Some(state) = state.downcast_mut::<T::State>() {
            AnimationNode::inner_states_mut(self, state, visit);
        }
    }

    fn state_as_any<'a>(&self, state: &'a NodeState) -> &'a dyn Any {
        &**state
    }

    fn state_as_any_mut<'a>(&self, state: &'a mut NodeState) -> &'a mut dyn Any {
        &mut **state
    }
}

fn fetched<'a, T: AnimationNode, P: ReadOnlySystemInput + 'static>(
    context: &'a AnimationGraphContext<'a>,
) -> Option<&'a SystemInputData<'a, 'a, P>> {
    context
        .inputs
        .get::<P>()
        .inspect_err(|error| warn!("{} skipped: {error}", std::any::type_name::<T>()))
        .ok()
}

/// How a graph builds a node from stored data and stores it back.
#[derive(Clone, Copy)]
pub struct ReflectAnimationNode {
    pub from_value:
        fn(&ReflectedValue, &mut LiveLeaves) -> (Box<dyn ErasedAnimationNode>, Vec<ValueWarning>),
    pub to_value: fn(&dyn ErasedAnimationNode) -> Option<Result<ReflectedValue, EncodeError>>,
}

impl TypeData for ReflectAnimationNode {}

impl<T: AnimationNode + Default> FromType<T> for ReflectAnimationNode {
    fn from_type() -> Self {
        Self {
            from_value: from_value::<T>,
            to_value: to_value_of::<T>,
        }
    }
}

fn from_value<T: AnimationNode + Default>(
    stored: &ReflectedValue,
    leaves: &mut LiveLeaves,
) -> (Box<dyn ErasedAnimationNode>, Vec<ValueWarning>) {
    let mut node = T::default();
    let warnings = apply_value_with(&mut node, stored, leaves);
    (Box::new(node), warnings)
}

fn to_value_of<T: AnimationNode>(
    node: &dyn ErasedAnimationNode,
) -> Option<Result<ReflectedValue, EncodeError>> {
    let node = node.as_reflect().downcast_ref::<T>()?;
    Some(to_value(node))
}

/// Stands in for a stored node whose type is not registered: passes its first input through.
#[derive(Default, Reflect)]
#[reflect(AnimationNode)]
pub struct PlaceholderNode {}

impl AnimationNode for PlaceholderNode {
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
        if let Some(input) = evaluated_inputs.first() {
            output.copy_from(&input.frame);
        }
    }

    fn reset(&self, _: &mut ()) {}
}

#[cfg(test)]
mod tests {
    use concerto_ecs::resource::{Res, Resource};
    use concerto_foundation::assets::asset_store::AssetStore;

    use super::*;
    use crate::{blackboard::AnimationBlackboard, testing::Walks};

    #[derive(Resource)]
    struct Step(u32);

    #[derive(Default, Reflect)]
    #[reflect(AnimationNode)]
    struct Gain {
        amount: f32,
    }

    #[derive(Default)]
    struct GainState {
        updates: u32,
        resets: u32,
    }

    impl AnimationNode for Gain {
        type State = GainState;
        type CreateInput = ();
        type UpdateInput = Res<'static, Step>;
        type EvaluateInput = ();

        fn input_slots(&self) -> InputSlots {
            InputSlots::None
        }

        fn create_state(&self, _: &(), _context: &AnimationGraphContext<'_>) -> GainState {
            GainState::default()
        }

        fn update(
            &self,
            state: &mut GainState,
            _delta_time: f32,
            step: &Res<Step>,
            _context: &AnimationGraphContext<'_>,
        ) {
            state.updates += step.0;
        }

        fn evaluate(
            &self,
            state: &mut GainState,
            _: &(),
            _context: &AnimationGraphContext<'_>,
            _bone_ids: &[Uuid],
            _evaluated_inputs: &[EvaluatedFrame],
            _pool: &mut FramePool,
            output: &mut AnimationFrame,
        ) {
            if let Some(joint) = output.joint_mut(0) {
                joint.translation.x = self.amount * state.updates as f32;
            }
        }

        fn reset(&self, state: &mut GainState) {
            state.resets += 1;
        }

        fn is_finished(&self, state: &GainState) -> bool {
            state.updates >= 2
        }
    }

    fn gain_walks(register: impl FnOnce(&mut concerto_ecs::World)) -> Walks {
        Walks::with(AssetStore::new(), AssetStore::new(), register)
    }

    fn updates_after_two_steps(walks: &Walks) -> u32 {
        let blackboard = AnimationBlackboard::default();
        let node: Box<dyn ErasedAnimationNode> = Box::new(Gain { amount: 2.0 });
        let mut state = walks
            .create(&blackboard, |context| node.create_state(context))
            .unwrap();
        for _ in 0..2 {
            walks.update(&blackboard, |context| node.update(&mut state, 0.1, context));
        }
        node.state_as_any(&state)
            .downcast_ref::<GainState>()
            .unwrap()
            .updates
    }

    #[test]
    fn a_node_reads_its_update_input_from_the_world() {
        let walks = gain_walks(|world| {
            register_node_inputs::<Gain>(world);
            world.insert_resource(Step(3));
        });

        assert_eq!(updates_after_two_steps(&walks), 6);
    }
}
