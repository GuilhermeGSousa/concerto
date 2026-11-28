use std::any::Any;

use concerto_ecs::{
    World,
    system::input::{Inputs, ReadOnlySystemInput, SystemInputData},
};
use concerto_foundation::assets::handle::{AssetHandle, LiveLeaves};
use concerto_name::{Name, name};
use concerto_reflect::{
    EncodeError, FromType, Reflect, ReflectedValue, TypeData, TypeRegistry, ValueWarning,
};
use log::warn;
use uuid::Uuid;

use crate::{
    evaluation::{AnimationGraphContext, DriveContext},
    graph::{AnimationGraph, AnimationGraphState},
    pose::{AnimationFrame, EvaluatedFrame, FramePool, RootMotion, WeightedMotion},
};

/// The runtime state of one node in one player, erased.
pub type NodeState = Box<dyn Any + Send + Sync>;

/// A kind of animation graph node: reflected data plus the per-player state it evaluates with.
pub trait AnimationNode: Reflect + Send + Sync + 'static {
    /// The state the node keeps per player, created by [`Self::create_state`].
    type State: Send + Sync + 'static;
    /// What the node reads to create its state.
    type CreateInput: ReadOnlySystemInput + 'static;
    /// What the node reads to advance.
    type UpdateInput: ReadOnlySystemInput + 'static;
    /// What the node reads to write its pose.
    type EvaluateInput: ReadOnlySystemInput + 'static;
    /// What the node reads to say what it asks of physics each step.
    type DriveInput: ReadOnlySystemInput + 'static;

    /// Creates the node's state for one player, when the player starts the graph.
    fn create_state(
        &self,
        input: &SystemInputData<'_, '_, Self::CreateInput>,
        context: &AnimationGraphContext<'_>,
    ) -> Self::State;

    /// Advances the node's state by `delta_time` seconds, before the frame is evaluated.
    ///
    /// `context` holds the messages the game sent the player this frame, and takes the messages
    /// the node raises.
    fn update(
        &self,
        state: &mut Self::State,
        delta_time: f32,
        input: &SystemInputData<'_, '_, Self::UpdateInput>,
        context: &mut AnimationGraphContext<'_>,
    );

    /// Writes the node's pose and the clip events it crossed into `output`.
    ///
    /// `evaluated_inputs` holds the frames of the node's inputs in slot order, and `bone_ids` the
    /// skeleton's bone ids in pose order. `output` starts at the skeleton's rest pose; a node that
    /// writes nothing leaves the rest pose. `pool` hands out scratch frames, which go back to it.
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

    /// How the root moved this frame, from the node's state after `update` and its inputs' root motion.
    ///
    /// The default passes the first input's root motion through.
    fn root_motion(
        &self,
        _state: &mut Self::State,
        _input: &SystemInputData<'_, '_, Self::UpdateInput>,
        _context: &AnimationGraphContext<'_>,
        _bone_ids: &[Uuid],
        inputs: &[WeightedMotion],
    ) -> RootMotion {
        inputs
            .first()
            .map_or(RootMotion::IDENTITY, |input| input.motion)
    }

    /// What the node asks of physics this step, from its state and its inputs' drive frames.
    ///
    /// Only attributes are read and written; joints are unused. `inputs` holds the drive frames of
    /// the node's inputs in slot order, each with its node's weight. The default blends the
    /// attributes of the inputs with a positive weight by those weights, as
    /// [`Self::drive_shares`] shares them out. Override it, with `drive_shares`, in a node that
    /// computes its own blend, like a blend space or a layer.
    ///
    /// A node that decides something in the physics step raises it with
    /// [`DriveContext::raise`]; the graph acts on it at the player's next update, as on a message
    /// the game sent.
    fn drive(
        &self,
        _state: &mut Self::State,
        _input: &SystemInputData<'_, '_, Self::DriveInput>,
        _context: &mut DriveContext<'_>,
        _bone_ids: &[Uuid],
        inputs: &[EvaluatedFrame],
        _pool: &mut FramePool,
        output: &mut AnimationFrame,
    ) {
        let mut weighted = inputs.iter().filter(|input| input.weight > 0.0);
        let Some(first) = weighted.next() else {
            return;
        };
        let attributes = output.attributes_mut();
        attributes.copy_from(first.frame.attributes());
        let mut total_weight = first.weight;
        for input in weighted {
            total_weight += input.weight;
            attributes.blend(input.frame.attributes(), input.weight / total_weight);
        }
    }

    /// How much of each input's drive frame [`Self::drive`] carries into the node's, written over `shares`.
    ///
    /// `shares` comes in holding the weight of each input's node, in slot order, and leaves
    /// holding each input's share of this node's drive frame, from 0 to 1. Before any node
    /// drives, the walk multiplies the shares down the graph into each node's
    /// [`DriveContext::weight`]; `context` carries this node's. The default normalizes the
    /// positive weights to sum to 1 and zeroes the rest, as the default [`Self::drive`] blends
    /// them. A node that overrides `drive` to compute its own blend overrides this too.
    fn drive_shares(&self, _state: &Self::State, _context: &DriveContext<'_>, shares: &mut [f32]) {
        let total: f32 = shares.iter().filter(|weight| **weight > 0.0).sum();
        for share in shares {
            *share = if *share > 0.0 { *share / total } else { 0.0 };
        }
    }

    /// Puts the state back to where [`Self::create_state`] left it, as when a state machine
    /// enters the node's state again.
    fn reset(&self, state: &mut Self::State);

    /// Checks what the node holds against the registered types, once, when the graph holding it
    /// is made live; returns what could not be applied.
    fn make_live(
        &mut self,
        _registry: &TypeRegistry,
        _leaves: &mut LiveLeaves,
    ) -> Vec<ValueWarning> {
        Vec::new()
    }

    /// Visits the graph assets this node plays inside itself.
    fn inner_graphs(&self, _visit: &mut dyn FnMut(&AssetHandle<AnimationGraph>)) {}

    /// Visits the graph states this node's state holds for the graphs it plays inside itself.
    ///
    /// When the node is a state machine, each comes with the name of the state that plays it.
    fn inner_states(
        &self,
        _state: &Self::State,
        _visit: &mut dyn FnMut(Option<Name>, &AnimationGraphState),
    ) {
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

/// What an editor or a tool asks of a node beyond running it: its input pins and a line saying what it does now; every method has a default, so a node with nothing to add writes `impl EditorAnimationNode for X {}`.
pub trait EditorAnimationNode: AnimationNode {
    /// The inputs this node takes, in the order `evaluate` receives them.
    fn input_slots(&self) -> InputSlots {
        InputSlots::None
    }

    /// Called when an editor gives the node an input at `index`.
    fn input_added(&mut self, _index: usize) {}

    /// Called when an editor takes the node's input at `index` away.
    fn input_removed(&mut self, _index: usize) {}

    /// One line saying what the node does now, shown by tools beside it; none by default.
    fn describe(&self, _state: &Self::State) -> Option<String> {
        None
    }
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

/// The graph walk that drives physics each step; its input set holds every registered node's `DriveInput`.
pub struct DriveNodeInputs;

/// Registers `N`'s four inputs with the graph walks of `world`, before its schedules compile.
pub fn register_node_inputs<N: AnimationNode>(world: &mut World) {
    world.register_input::<CreateNodeInputs, N::CreateInput>();
    world.register_input::<UpdateNodeInputs, N::UpdateInput>();
    world.register_input::<EvaluateNodeInputs, N::EvaluateInput>();
    world.register_input::<DriveNodeInputs, N::DriveInput>();
}

/// An [`EditorAnimationNode`] with its state type erased; implemented for every one.
pub trait ErasedAnimationNode: Send + Sync {
    /// The node as a reflected value.
    fn as_reflect(&self) -> &dyn Reflect;

    /// The node as a reflected value, to change.
    fn as_reflect_mut(&mut self) -> &mut dyn Reflect;

    /// Creates the node's state, or `None` when the walk's inputs lack what the node reads.
    fn create_state(&self, context: &AnimationGraphContext<'_>) -> Option<NodeState>;

    /// Advances the state by `delta_time` seconds; see [`AnimationNode::update`].
    fn update(
        &self,
        state: &mut NodeState,
        delta_time: f32,
        context: &mut AnimationGraphContext<'_>,
    );

    /// Writes the node's pose into `output`; see [`AnimationNode::evaluate`].
    fn evaluate(
        &self,
        state: &mut NodeState,
        context: &AnimationGraphContext<'_>,
        bone_ids: &[Uuid],
        evaluated_inputs: &[EvaluatedFrame],
        pool: &mut FramePool,
        output: &mut AnimationFrame,
    );

    /// The node's root motion this frame; see [`AnimationNode::root_motion`].
    fn root_motion(
        &self,
        state: &mut NodeState,
        context: &AnimationGraphContext<'_>,
        bone_ids: &[Uuid],
        inputs: &[WeightedMotion],
    ) -> RootMotion;

    /// What the node asks of physics this step; see [`AnimationNode::drive`].
    fn drive(
        &self,
        state: &mut NodeState,
        context: &mut DriveContext<'_>,
        bone_ids: &[Uuid],
        inputs: &[EvaluatedFrame],
        pool: &mut FramePool,
        output: &mut AnimationFrame,
    );

    /// Each input's share of the node's drive frame; see [`AnimationNode::drive_shares`].
    fn drive_shares(&self, state: &NodeState, context: &DriveContext<'_>, shares: &mut [f32]);

    /// Resets the state; see [`AnimationNode::reset`].
    fn reset(&self, state: &mut NodeState);

    /// Checks what the node holds against the registered types; see [`AnimationNode::make_live`].
    fn make_live(&mut self, registry: &TypeRegistry, leaves: &mut LiveLeaves) -> Vec<ValueWarning>;

    /// Visits the graph assets this node plays inside itself.
    fn inner_graphs(&self, visit: &mut dyn FnMut(&AssetHandle<AnimationGraph>));

    /// The inputs this node takes.
    fn input_slots(&self) -> InputSlots;

    /// Tells the node an editor gave it an input at `index`.
    fn input_added(&mut self, index: usize);

    /// Tells the node an editor took its input at `index` away.
    fn input_removed(&mut self, index: usize);

    /// Visits the graph states this node's state holds, each with the state machine state that
    /// plays it.
    fn inner_states(
        &self,
        state: &NodeState,
        visit: &mut dyn FnMut(Option<Name>, &AnimationGraphState),
    );

    /// One line saying what the node does now; see [`EditorAnimationNode::describe`].
    fn describe(&self, state: &NodeState) -> Option<String>;

    /// Visits, mutably, the graph states this node's state holds.
    #[cfg(any(test, feature = "test-support"))]
    fn inner_states_mut(
        &self,
        state: &mut NodeState,
        visit: &mut dyn FnMut(&mut AnimationGraphState),
    );

    /// The state as [`Any`], to downcast to the node's state type.
    fn state_as_any<'a>(&self, state: &'a NodeState) -> &'a dyn Any;

    /// The state as [`Any`], to downcast and change.
    fn state_as_any_mut<'a>(&self, state: &'a mut NodeState) -> &'a mut dyn Any;
}

impl<T: EditorAnimationNode> ErasedAnimationNode for T {
    fn as_reflect(&self) -> &dyn Reflect {
        self
    }

    fn as_reflect_mut(&mut self) -> &mut dyn Reflect {
        self
    }

    fn create_state(&self, context: &AnimationGraphContext<'_>) -> Option<NodeState> {
        let input = fetched::<T, T::CreateInput>(context.inputs)?;
        Some(Box::new(AnimationNode::create_state(self, input, context)))
    }

    fn update(
        &self,
        state: &mut NodeState,
        delta_time: f32,
        context: &mut AnimationGraphContext<'_>,
    ) {
        if let Some(state) = state.downcast_mut::<T::State>()
            && let Some(input) = fetched::<T, T::UpdateInput>(context.inputs)
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
            && let Some(input) = fetched::<T, T::EvaluateInput>(context.inputs)
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

    fn root_motion(
        &self,
        state: &mut NodeState,
        context: &AnimationGraphContext<'_>,
        bone_ids: &[Uuid],
        inputs: &[WeightedMotion],
    ) -> RootMotion {
        if let Some(state) = state.downcast_mut::<T::State>()
            && let Some(input) = fetched::<T, T::UpdateInput>(context.inputs)
        {
            AnimationNode::root_motion(self, state, input, context, bone_ids, inputs)
        } else {
            RootMotion::IDENTITY
        }
    }

    fn drive(
        &self,
        state: &mut NodeState,
        context: &mut DriveContext<'_>,
        bone_ids: &[Uuid],
        inputs: &[EvaluatedFrame],
        pool: &mut FramePool,
        output: &mut AnimationFrame,
    ) {
        if let Some(state) = state.downcast_mut::<T::State>()
            && let Some(input) = fetched::<T, T::DriveInput>(context.graph.inputs)
        {
            AnimationNode::drive(self, state, input, context, bone_ids, inputs, pool, output);
        }
    }

    fn drive_shares(&self, state: &NodeState, context: &DriveContext<'_>, shares: &mut [f32]) {
        match state.downcast_ref::<T::State>() {
            Some(state) => AnimationNode::drive_shares(self, state, context, shares),
            None => shares.fill(0.0),
        }
    }

    fn reset(&self, state: &mut NodeState) {
        if let Some(state) = state.downcast_mut::<T::State>() {
            AnimationNode::reset(self, state);
        }
    }

    fn make_live(&mut self, registry: &TypeRegistry, leaves: &mut LiveLeaves) -> Vec<ValueWarning> {
        AnimationNode::make_live(self, registry, leaves)
    }

    fn inner_graphs(&self, visit: &mut dyn FnMut(&AssetHandle<AnimationGraph>)) {
        AnimationNode::inner_graphs(self, visit);
    }

    fn input_slots(&self) -> InputSlots {
        EditorAnimationNode::input_slots(self)
    }

    fn input_added(&mut self, index: usize) {
        EditorAnimationNode::input_added(self, index);
    }

    fn input_removed(&mut self, index: usize) {
        EditorAnimationNode::input_removed(self, index);
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

    fn describe(&self, state: &NodeState) -> Option<String> {
        state
            .downcast_ref::<T::State>()
            .and_then(|state| EditorAnimationNode::describe(self, state))
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
    inputs: &'a Inputs<'a, 'a>,
) -> Option<&'a SystemInputData<'a, 'a, P>> {
    inputs
        .get::<P>()
        .inspect_err(|error| warn!("{} skipped: {error}", std::any::type_name::<T>()))
        .ok()
}

/// How a graph builds a node from stored data and stores it back.
#[derive(Clone, Copy)]
pub struct ReflectAnimationNode {
    /// Builds the node from its stored fields, with the fields that did not apply.
    pub from_value:
        fn(&ReflectedValue, &mut LiveLeaves) -> (Box<dyn ErasedAnimationNode>, Vec<ValueWarning>),
    /// Stores the node's fields; `None` when the node is not of the registered type.
    pub to_value: fn(&dyn ErasedAnimationNode) -> Option<Result<ReflectedValue, EncodeError>>,
}

impl TypeData for ReflectAnimationNode {}

impl<T: EditorAnimationNode + Default> FromType<T> for ReflectAnimationNode {
    fn from_type() -> Self {
        Self {
            from_value: from_value::<T>,
            to_value: to_value_of::<T>,
        }
    }
}

fn from_value<T: EditorAnimationNode + Default>(
    stored: &ReflectedValue,
    leaves: &mut LiveLeaves,
) -> (Box<dyn ErasedAnimationNode>, Vec<ValueWarning>) {
    let mut node = T::default();
    let warnings = stored.apply_with(&mut node, leaves);
    (Box::new(node), warnings)
}

fn to_value_of<T: AnimationNode>(
    node: &dyn ErasedAnimationNode,
) -> Option<Result<ReflectedValue, EncodeError>> {
    let node = node.as_reflect().downcast_ref::<T>()?;
    Some(ReflectedValue::encode(node))
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
    type DriveInput = ();

    fn create_state(&self, _: &(), _context: &AnimationGraphContext<'_>) {}

    fn update(&self, _: &mut (), _: f32, _: &(), _: &mut AnimationGraphContext<'_>) {}

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

impl EditorAnimationNode for PlaceholderNode {
    fn input_slots(&self) -> InputSlots {
        InputSlots::Fixed(vec![name!("pose")])
    }
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
        type DriveInput = ();

        fn create_state(&self, _: &(), _context: &AnimationGraphContext<'_>) -> GainState {
            GainState::default()
        }

        fn update(
            &self,
            state: &mut GainState,
            _delta_time: f32,
            step: &Res<Step>,
            _context: &mut AnimationGraphContext<'_>,
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
    }

    impl EditorAnimationNode for Gain {}

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
