use std::any::Any;

use concerto_foundation::assets::handle::LiveLeaves;
use concerto_reflect::{
    EncodeError, FromType, Reflect, ReflectedValue, TypeData, ValueWarning, apply_value_with,
    to_value,
};
use uuid::Uuid;

use crate::{
    evaluation::AnimationGraphContext,
    pose::{EvaluatedPose, Pose, PosePool},
};

/// The runtime state of one node in one player, erased.
pub type NodeInstance = Box<dyn Any + Send + Sync>;

/// A kind of animation graph node: reflected data plus the per-player state it evaluates with.
pub trait AnimationNode: Reflect + Send + Sync + 'static {
    type Instance: Send + Sync + 'static;

    fn create_instance(&self, context: &AnimationGraphContext<'_>) -> Self::Instance;

    fn update(
        &self,
        instance: &mut Self::Instance,
        delta_time: f32,
        context: &AnimationGraphContext<'_>,
    );

    fn evaluate(
        &self,
        instance: &mut Self::Instance,
        context: &AnimationGraphContext<'_>,
        bone_ids: &[Uuid],
        evaluated_inputs: &[EvaluatedPose],
        pool: &mut PosePool,
        output: &mut Pose,
    );

    fn reset(&self, instance: &mut Self::Instance);

    fn is_finished(&self, _instance: &Self::Instance) -> bool {
        true
    }
}

/// An [`AnimationNode`] with its instance type erased; implemented for every node.
pub trait ErasedAnimationNode: Send + Sync {
    fn as_reflect(&self) -> &dyn Reflect;

    fn as_reflect_mut(&mut self) -> &mut dyn Reflect;

    fn create_instance(&self, context: &AnimationGraphContext<'_>) -> NodeInstance;

    fn update(
        &self,
        instance: &mut NodeInstance,
        delta_time: f32,
        context: &AnimationGraphContext<'_>,
    );

    fn evaluate(
        &self,
        instance: &mut NodeInstance,
        context: &AnimationGraphContext<'_>,
        bone_ids: &[Uuid],
        evaluated_inputs: &[EvaluatedPose],
        pool: &mut PosePool,
        output: &mut Pose,
    );

    fn reset(&self, instance: &mut NodeInstance);

    fn is_finished(&self, instance: &NodeInstance) -> bool;

    fn instance_as_any<'a>(&self, instance: &'a NodeInstance) -> &'a dyn Any;

    fn instance_as_any_mut<'a>(&self, instance: &'a mut NodeInstance) -> &'a mut dyn Any;
}

impl<T: AnimationNode> ErasedAnimationNode for T {
    fn as_reflect(&self) -> &dyn Reflect {
        self
    }

    fn as_reflect_mut(&mut self) -> &mut dyn Reflect {
        self
    }

    fn create_instance(&self, context: &AnimationGraphContext<'_>) -> NodeInstance {
        Box::new(AnimationNode::create_instance(self, context))
    }

    fn update(
        &self,
        instance: &mut NodeInstance,
        delta_time: f32,
        context: &AnimationGraphContext<'_>,
    ) {
        if let Some(instance) = instance.downcast_mut::<T::Instance>() {
            AnimationNode::update(self, instance, delta_time, context);
        }
    }

    fn evaluate(
        &self,
        instance: &mut NodeInstance,
        context: &AnimationGraphContext<'_>,
        bone_ids: &[Uuid],
        evaluated_inputs: &[EvaluatedPose],
        pool: &mut PosePool,
        output: &mut Pose,
    ) {
        if let Some(instance) = instance.downcast_mut::<T::Instance>() {
            AnimationNode::evaluate(
                self,
                instance,
                context,
                bone_ids,
                evaluated_inputs,
                pool,
                output,
            );
        }
    }

    fn reset(&self, instance: &mut NodeInstance) {
        if let Some(instance) = instance.downcast_mut::<T::Instance>() {
            AnimationNode::reset(self, instance);
        }
    }

    fn is_finished(&self, instance: &NodeInstance) -> bool {
        instance
            .downcast_ref::<T::Instance>()
            .is_none_or(|instance| AnimationNode::is_finished(self, instance))
    }

    fn instance_as_any<'a>(&self, instance: &'a NodeInstance) -> &'a dyn Any {
        &**instance
    }

    fn instance_as_any_mut<'a>(&self, instance: &'a mut NodeInstance) -> &'a mut dyn Any {
        &mut **instance
    }
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
        match evaluated_inputs.first() {
            Some(input) => output.copy_from(&input.pose),
            None => output.copy_from(&Pose::identity(output.len())),
        }
    }

    fn reset(&self, _: &mut ()) {}
}

#[cfg(test)]
mod tests {
    use concerto_foundation::assets::asset_store::AssetStore;
    use concerto_reflect::TypeRegistry;

    use super::*;
    use crate::blackboard::AnimationBlackboard;

    #[derive(Default, Reflect)]
    #[reflect(AnimationNode)]
    struct Gain {
        amount: f32,
    }

    #[derive(Default)]
    struct GainInstance {
        updates: u32,
        resets: u32,
    }

    impl AnimationNode for Gain {
        type Instance = GainInstance;

        fn create_instance(&self, _context: &AnimationGraphContext<'_>) -> GainInstance {
            GainInstance::default()
        }

        fn update(
            &self,
            instance: &mut GainInstance,
            _delta_time: f32,
            _context: &AnimationGraphContext<'_>,
        ) {
            instance.updates += 1;
        }

        fn evaluate(
            &self,
            instance: &mut GainInstance,
            _context: &AnimationGraphContext<'_>,
            _bone_ids: &[Uuid],
            _evaluated_inputs: &[EvaluatedPose],
            _pool: &mut PosePool,
            output: &mut Pose,
        ) {
            if let Some(joint) = output.get_joint_pose_mut(0) {
                joint.translation.x = self.amount * instance.updates as f32;
            }
        }

        fn reset(&self, instance: &mut GainInstance) {
            instance.resets += 1;
        }

        fn is_finished(&self, instance: &GainInstance) -> bool {
            instance.updates >= 2
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

    #[test]
    fn erased_calls_reach_the_typed_methods() {
        let clips = AssetStore::new();
        let graphs = AssetStore::new();
        let blackboard = AnimationBlackboard::default();
        let context = AnimationGraphContext {
            animation_clips: &clips,
            animation_graphs: &graphs,
            blackboard: &blackboard,
            root: None,
        };
        let node: Box<dyn ErasedAnimationNode> = Box::new(Gain { amount: 2.0 });
        let mut instance = node.create_instance(&context);
        let mut pool = PosePool::new(1);
        let mut output = Pose::identity(1);

        assert!(!node.is_finished(&instance));
        node.update(&mut instance, 0.1, &context);
        node.update(&mut instance, 0.1, &context);
        node.evaluate(&mut instance, &context, &[], &[], &mut pool, &mut output);
        node.reset(&mut instance);

        assert!(node.is_finished(&instance));
        assert_eq!(output.joints()[0].translation.x, 4.0);
        let typed = node
            .instance_as_any(&instance)
            .downcast_ref::<GainInstance>()
            .unwrap();
        assert_eq!((typed.updates, typed.resets), (2, 1));
    }

    #[test]
    fn placeholder_passes_the_first_input_through() {
        let clips = AssetStore::new();
        let graphs = AssetStore::new();
        let blackboard = AnimationBlackboard::default();
        let context = AnimationGraphContext {
            animation_clips: &clips,
            animation_graphs: &graphs,
            blackboard: &blackboard,
            root: None,
        };
        let node: Box<dyn ErasedAnimationNode> = Box::new(PlaceholderNode {});
        let mut instance = node.create_instance(&context);
        let mut pool = PosePool::new(1);
        let mut input = Pose::identity(1);
        input.get_joint_pose_mut(0).unwrap().translation.x = 3.0;
        let inputs = [EvaluatedPose {
            pose: input,
            weight: 1.0,
        }];

        let mut output = Pose::identity(1);
        node.evaluate(
            &mut instance,
            &context,
            &[],
            &inputs,
            &mut pool,
            &mut output,
        );
        assert_eq!(output.joints()[0].translation.x, 3.0);

        let mut output = Pose::identity(1);
        node.evaluate(&mut instance, &context, &[], &[], &mut pool, &mut output);
        assert_eq!(output.joints()[0].translation.x, 0.0);
    }

    #[test]
    fn placeholder_without_input_resets_a_recycled_pose_to_identity() {
        let clips = AssetStore::new();
        let graphs = AssetStore::new();
        let blackboard = AnimationBlackboard::default();
        let context = AnimationGraphContext {
            animation_clips: &clips,
            animation_graphs: &graphs,
            blackboard: &blackboard,
            root: None,
        };
        let node: Box<dyn ErasedAnimationNode> = Box::new(PlaceholderNode {});
        let mut instance = node.create_instance(&context);
        let mut pool = PosePool::new(1);
        let mut recycled = Pose::identity(1);
        recycled.get_joint_pose_mut(0).unwrap().translation.x = 7.0;
        pool.release(recycled);

        let mut output = pool.acquire();
        node.evaluate(&mut instance, &context, &[], &[], &mut pool, &mut output);
        assert_eq!(output.joints()[0].translation.x, 0.0);
    }
}
