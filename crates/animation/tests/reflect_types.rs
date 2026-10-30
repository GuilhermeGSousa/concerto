use concerto_animation::node::AnimationClipNode;
use concerto_animation::node::AnimationPlayMode;
use concerto_animation::node::blend_space::{BlendInput, BlendSpace2DNode};
use concerto_animation::node::state_machine::{
    AnimationFSMTrigger, AnimationStateMachine, StateId,
};
use concerto_reflect::{Reflect, TypeRegistry};

fn reflected<T: Reflect + Default + concerto_reflect::GetTypeRegistration>() {
    let mut registry = TypeRegistry::default();
    registry.register::<T>();
    let value: Box<dyn Reflect> = Box::new(T::default());
    assert!(value.downcast_ref::<T>().is_some());
}

#[test]
fn graph_data_types_reflect_and_default() {
    reflected::<StateId>();
    reflected::<AnimationFSMTrigger>();
    reflected::<AnimationStateMachine>();
    reflected::<BlendInput>();
    reflected::<BlendSpace2DNode>();
    reflected::<AnimationClipNode>();
    reflected::<AnimationPlayMode>();
}
