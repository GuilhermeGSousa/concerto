use concerto_animation::TransitionBlender;
use concerto_animation::clip::AnimationClip;
use concerto_animation::evaluation::AnimationGraphContext;
use concerto_animation::graph::AnimationGraph;
use concerto_animation::node::blend_space::BlendSpace2DNode;
use concerto_animation::node::state_machine::{AnimationFSMTrigger, AnimationStateMachine};
use concerto_animation::node::{
    AnimationClipNode, AnimationNode, ErasedAnimationNode, ReflectAnimationNode,
};
use concerto_animation::plugin::register_animation_types;
use concerto_animation::pose::{EvaluatedPose, Pose, PosePool};
use concerto_foundation::assets::AssetId;
use concerto_foundation::assets::asset_server::AssetServer;
use concerto_foundation::assets::asset_store::AssetStore;
use concerto_foundation::assets::handle::AssetHandle;
use concerto_reflect::{Reflect, TypeRegistry};
use uuid::Uuid;

#[derive(Default, Reflect)]
#[reflect(AnimationNode)]
struct Gain {
    amount: f32,
}

impl AnimationNode for Gain {
    type Instance = ();

    fn create_instance(&self, _context: &AnimationGraphContext<'_>) {}

    fn update(&self, _: &mut (), _: f32, _: &AnimationGraphContext<'_>) {}

    fn evaluate(
        &self,
        _: &mut (),
        _: &AnimationGraphContext<'_>,
        _: &[Uuid],
        _: &[EvaluatedPose],
        _: &mut PosePool,
        _: &mut Pose,
    ) {
    }

    fn reset(&self, _: &mut ()) {}
}

fn clip(name: &str) -> AssetHandle<AnimationClip> {
    AssetHandle::weak(AssetId::from_path(name))
}

fn registry(with_gain: bool) -> TypeRegistry {
    let mut registry = TypeRegistry::default();
    register_animation_types(&mut registry);
    if with_gain {
        registry.register::<Gain>();
    }
    registry
}

fn live_type_names(graph: &AnimationGraph) -> Vec<&'static str> {
    graph
        .live_nodes()
        .map(|node| node.as_reflect().type_name())
        .collect()
}

fn live_node<T: AnimationNode>(graph: &AnimationGraph) -> Option<&T> {
    graph
        .live_nodes()
        .find_map(|node: &dyn ErasedAnimationNode| node.as_reflect().downcast_ref::<T>())
}

fn state_machine() -> AnimationStateMachine {
    let state = |name: &str| AssetHandle::weak(AssetId::from_path(name));
    AnimationStateMachine::from_initial_state("idle", state("idle_graph"), |transition| {
        transition.to("walk", AnimationFSMTrigger::on_bool("go", true), 0.2);
    })
    .state("walk", state("walk_graph"), |transition| {
        transition.to("run", AnimationFSMTrigger::on_non_zero_vec("move"), 0.15);
    })
    .state("run", state("run_graph"), |transition| {
        transition.to("idle", AnimationFSMTrigger::OnAnimationEnd, 0.1);
    })
    .blender(TransitionBlender::Inertialization)
    .build()
}

fn mixed_graph() -> AnimationGraph {
    let mut graph = AnimationGraph::new();
    graph
        .result_node()
        .with_input(Gain { amount: 2.5 }, |_| {})
        .with_input(AnimationClipNode::new(clip("outer_pose")), |_| {})
        .with_input(state_machine(), |_| {})
        .with_blend_space_2d_input("movement", |ctx| {
            ctx.animation_clip_input(clip("idle"), glam::Vec2::ZERO)
                .animation_clip_input(clip("walk"), glam::Vec2::new(0.0, 1.0));
        });
    graph
}

fn round_trip(graph: &AnimationGraph) -> AnimationGraph {
    let bytes = bincode::serialize(graph).expect("serialize");
    bincode::deserialize(&bytes).expect("deserialize")
}

#[test]
fn a_registered_custom_node_survives_a_round_trip_next_to_built_ins() {
    let registry = registry(true);
    let mut restored = round_trip(&mixed_graph());
    assert!(!restored.is_live());
    restored.make_live(&registry, None).unwrap();

    assert!(restored.is_live());
    assert!(restored.load_warnings().is_empty());
    assert_eq!(restored.iter().count(), 7);
    assert_eq!(live_node::<Gain>(&restored).unwrap().amount, 2.5);
    assert_eq!(
        live_node::<AnimationClipNode>(&restored)
            .unwrap()
            .clip()
            .id(),
        AssetId::from_path("outer_pose"),
        "clip AssetId survived the round trip"
    );
}

#[test]
fn a_blend_space_keeps_its_input_param_and_points() {
    let registry = registry(true);
    let mut restored = round_trip(&mixed_graph());
    restored.make_live(&registry, None).unwrap();

    let blend_space = live_node::<BlendSpace2DNode>(&restored).expect("blend space survived");
    assert_eq!(blend_space.input().param, "movement");
    assert_eq!(
        blend_space.points(),
        &[glam::Vec2::ZERO, glam::Vec2::new(0.0, 1.0)]
    );
}

#[test]
fn a_state_machine_keeps_trigger_payloads_blender_and_blend_times() {
    let registry = registry(true);
    let mut restored = round_trip(&mixed_graph());
    restored.make_live(&registry, None).unwrap();

    let machine = live_node::<AnimationStateMachine>(&restored).expect("state machine survived");
    let triggers: Vec<&AnimationFSMTrigger> = machine.triggers().collect();

    let bool_equals = triggers
        .iter()
        .find_map(|trigger| match trigger {
            AnimationFSMTrigger::BoolEquals { param, value } => Some((param.as_str(), *value)),
            _ => None,
        })
        .expect("BoolEquals trigger survived");
    assert_eq!(bool_equals, ("go", true));

    let vec2_non_zero = triggers
        .iter()
        .find_map(|trigger| match trigger {
            AnimationFSMTrigger::Vec2NonZero { param } => Some(param.as_str()),
            _ => None,
        })
        .expect("Vec2NonZero trigger survived");
    assert_eq!(vec2_non_zero, "move");
    assert!(
        triggers
            .iter()
            .any(|trigger| matches!(trigger, AnimationFSMTrigger::OnAnimationEnd))
    );

    assert_eq!(machine.blender(), TransitionBlender::Inertialization);
    assert_eq!(
        machine.blend_times().collect::<Vec<_>>(),
        vec![0.2, 0.15, 0.1]
    );
}

#[test]
fn a_graph_built_in_code_and_one_loaded_from_its_bytes_match() {
    let registry = registry(true);
    let mut built = mixed_graph();
    built.make_live(&registry, None).unwrap();
    let mut loaded = round_trip(&built);
    loaded.make_live(&registry, None).unwrap();

    assert_eq!(
        built.stored_nodes().collect::<Vec<_>>(),
        loaded.stored_nodes().collect::<Vec<_>>()
    );
    assert_eq!(live_type_names(&built), live_type_names(&loaded));
    assert_eq!(built.iter().collect::<Vec<_>>().len(), 7);
    assert!(
        built
            .stored_nodes()
            .any(|(type_name, _)| type_name == std::any::type_name::<Gain>())
    );
}

#[test]
fn an_unregistered_node_type_loads_with_a_warning_and_is_kept_on_save() {
    let bytes = bincode::serialize(&mixed_graph()).unwrap();
    let mut loaded: AnimationGraph = bincode::deserialize(&bytes).unwrap();
    loaded.make_live(&registry(false), None).unwrap();

    assert!(loaded.is_live());
    let warnings = loaded.load_warnings();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].path.contains("Gain"), "{:?}", warnings[0]);
    assert!(live_node::<Gain>(&loaded).is_none());
    assert!(live_node::<AnimationClipNode>(&loaded).is_some());
    assert!(live_node::<AnimationStateMachine>(&loaded).is_some());
    assert!(
        live_type_names(&loaded)
            .iter()
            .any(|name| name.ends_with("PlaceholderNode"))
    );

    assert_eq!(bincode::serialize(&loaded).unwrap(), bytes);
}

#[test]
fn a_clip_handle_in_a_node_comes_back_strong() {
    let mut server = AssetServer::new();
    server.register_asset(&AssetStore::<AnimationClip>::new());
    server.register_asset(&AssetStore::<AnimationGraph>::new());

    let mut graph = AnimationGraph::from_node(AnimationClipNode::new(clip("outer_pose")));
    graph.make_live(&registry(false), Some(&server)).unwrap();

    assert!(graph.load_warnings().is_empty());
    let handle = live_node::<AnimationClipNode>(&graph).unwrap().clip();
    assert!(matches!(handle, AssetHandle::Strong(..)));
    assert_eq!(handle.id(), AssetId::from_path("outer_pose"));
}
