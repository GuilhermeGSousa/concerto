use std::any::Any;

use concerto_ecs::{
    World,
    system::input::{ReadOnlySystemInput, SystemInputData, register_input},
};
use concerto_foundation::assets::handle::LiveLeaves;
use concerto_reflect::{
    EncodeError, FromType, Reflect, ReflectedValue, TypeData, ValueWarning, apply_value_with,
    to_value,
};
use log::warn;
use uuid::Uuid;

use crate::{
    evaluation::AnimationGraphContext,
    pose::{EvaluatedPose, Pose, PosePool},
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
        evaluated_inputs: &[EvaluatedPose],
        pool: &mut PosePool,
        output: &mut Pose,
    );

    fn reset(&self, state: &mut Self::State);

    fn is_finished(&self, _state: &Self::State) -> bool {
        true
    }
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
        evaluated_inputs: &[EvaluatedPose],
        pool: &mut PosePool,
        output: &mut Pose,
    );

    fn reset(&self, state: &mut NodeState);

    fn is_finished(&self, state: &NodeState) -> bool;

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
        evaluated_inputs: &[EvaluatedPose],
        pool: &mut PosePool,
        output: &mut Pose,
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

    fn create_state(&self, _: &(), _context: &AnimationGraphContext<'_>) {}

    fn update(&self, _: &mut (), _: f32, _: &(), _: &AnimationGraphContext<'_>) {}

    fn evaluate(
        &self,
        _: &mut (),
        _: &(),
        _: &AnimationGraphContext<'_>,
        _: &[Uuid],
        evaluated_inputs: &[EvaluatedPose],
        _: &mut PosePool,
        output: &mut Pose,
    ) {
        if let Some(input) = evaluated_inputs.first() {
            output.copy_from(&input.pose);
        }
    }

    fn reset(&self, _: &mut ()) {}
}

#[cfg(test)]
mod tests {
    use concerto_ecs::resource::{Res, Resource};
    use concerto_foundation::assets::asset_store::AssetStore;
    use concerto_reflect::TypeRegistry;
    use glam::Vec3;

    use super::*;
    use crate::{blackboard::AnimationBlackboard, pose::JointPose, test_walks::Walks};

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
            _evaluated_inputs: &[EvaluatedPose],
            _pool: &mut PosePool,
            output: &mut Pose,
        ) {
            if let Some(joint) = output.get_joint_pose_mut(0) {
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

    fn registered() -> TypeRegistry {
        let mut registry = TypeRegistry::default();
        registry.register::<Gain>();
        registry
    }

    #[test]
    fn from_value_and_to_value_round_trip() {
        let registry = registered();
        let data = *registry
            .get_type_data::<ReflectAnimationNode>(std::any::TypeId::of::<Gain>())
            .unwrap();
        let stored = to_value(&Gain { amount: 2.5 }).unwrap();

        let mut leaves = LiveLeaves::from_parts(&registry, None);
        let (node, warnings) = (data.from_value)(&stored, &mut leaves);

        assert!(warnings.is_empty());
        assert_eq!(
            node.as_reflect().downcast_ref::<Gain>().unwrap().amount,
            2.5
        );
        assert_eq!((data.to_value)(&*node).unwrap().unwrap(), stored);
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
    fn erased_calls_reach_the_typed_methods_with_their_inputs() {
        let walks = gain_walks(|world| {
            register_node_inputs::<Gain>(world);
            world.insert_resource(Step(1));
        });
        let blackboard = AnimationBlackboard::default();
        let node: Box<dyn ErasedAnimationNode> = Box::new(Gain { amount: 2.0 });
        let mut state = walks
            .create(&blackboard, |context| node.create_state(context))
            .unwrap();
        let mut pool = PosePool::new(1);
        let mut output = Pose::identity(1);

        assert!(!node.is_finished(&state));
        walks.update(&blackboard, |context| node.update(&mut state, 0.1, context));
        walks.update(&blackboard, |context| node.update(&mut state, 0.1, context));
        walks.evaluate(&blackboard, None, |context| {
            node.evaluate(&mut state, context, &[], &[], &mut pool, &mut output)
        });
        node.reset(&mut state);

        assert!(node.is_finished(&state));
        assert_eq!(output.joints()[0].translation.x, 4.0);
        let typed = node
            .state_as_any(&state)
            .downcast_ref::<GainState>()
            .unwrap();
        assert_eq!((typed.updates, typed.resets), (2, 1));
    }

    #[test]
    fn a_node_reads_its_update_input_from_the_world() {
        let walks = gain_walks(|world| {
            register_node_inputs::<Gain>(world);
            world.insert_resource(Step(3));
        });

        assert_eq!(updates_after_two_steps(&walks), 6);
    }

    #[test]
    fn a_node_whose_inputs_were_not_registered_is_not_updated() {
        let walks = gain_walks(|world| world.insert_resource(Step(3)));

        assert_eq!(updates_after_two_steps(&walks), 0);
    }

    #[test]
    fn a_node_whose_input_resource_is_missing_is_not_updated() {
        let walks = gain_walks(register_node_inputs::<Gain>);

        assert_eq!(updates_after_two_steps(&walks), 0);
    }

    #[test]
    fn placeholder_passes_the_first_input_through() {
        let walks = gain_walks(|_| {});
        let blackboard = AnimationBlackboard::default();
        let node: Box<dyn ErasedAnimationNode> = Box::new(PlaceholderNode {});
        let mut state = walks
            .create(&blackboard, |context| node.create_state(context))
            .unwrap();
        let mut pool = PosePool::new(1);
        let mut input = Pose::identity(1);
        input.get_joint_pose_mut(0).unwrap().translation.x = 3.0;
        let inputs = [EvaluatedPose {
            pose: input,
            weight: 1.0,
        }];

        let mut output = Pose::identity(1);
        walks.evaluate(&blackboard, None, |context| {
            node.evaluate(&mut state, context, &[], &inputs, &mut pool, &mut output)
        });
        assert_eq!(output.joints()[0].translation.x, 3.0);

        let mut output = Pose::identity(1);
        walks.evaluate(&blackboard, None, |context| {
            node.evaluate(&mut state, context, &[], &[], &mut pool, &mut output)
        });
        assert_eq!(output.joints()[0].translation.x, 0.0);
    }

    #[test]
    fn placeholder_without_input_leaves_the_rest_pose() {
        let walks = gain_walks(|_| {});
        let blackboard = AnimationBlackboard::default();
        let node: Box<dyn ErasedAnimationNode> = Box::new(PlaceholderNode {});
        let mut state = walks
            .create(&blackboard, |context| node.create_state(context))
            .unwrap();
        let rest = [JointPose {
            translation: Vec3::X,
            ..JointPose::IDENTITY
        }];
        let mut pool = PosePool::new(1);
        pool.set_rest(&rest);

        let mut output = pool.acquire();
        walks.evaluate(&blackboard, None, |context| {
            node.evaluate(&mut state, context, &[], &[], &mut pool, &mut output)
        });
        assert_eq!(output.joints(), rest);
    }
}
