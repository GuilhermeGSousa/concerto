use concerto_animation::graph::AnimationGraph;
use concerto_animation::node::ErasedAnimationNode;
use concerto_animation::plugin::register_animation_types;
use concerto_name::name;
use concerto_procedural_animation::{FootPlacementLeg, FootPlacementNode};
use concerto_reflect::TypeRegistry;

fn registry() -> TypeRegistry {
    let mut registry = TypeRegistry::default();
    register_animation_types(&mut registry);
    registry.register::<FootPlacementNode>();
    registry
}

fn round_trip(graph: &AnimationGraph) -> AnimationGraph {
    let bytes = bincode::serialize(graph).expect("serialize");
    bincode::deserialize(&bytes).expect("deserialize")
}

fn live_placement(graph: &AnimationGraph) -> Option<&FootPlacementNode> {
    graph
        .live_nodes()
        .find_map(|node: &dyn ErasedAnimationNode| {
            node.as_reflect().downcast_ref::<FootPlacementNode>()
        })
}

#[test]
fn a_foot_placement_node_keeps_every_field_through_a_round_trip() {
    let node = FootPlacementNode {
        pelvis: "Hips".into(),
        legs: vec![
            FootPlacementLeg {
                foot: "LeftFoot".into(),
                toe: Some("LeftToe".into()),
            },
            FootPlacementLeg {
                foot: "RightFoot".into(),
                toe: None,
            },
        ],
        max_step_up: 0.4,
        max_step_down: 0.6,
        max_pelvis_drop: 0.3,
        max_foot_angle: 0.4,
        foot_halflife: 0.07,
        pelvis_halflife: 0.12,
        rotation_halflife: 0.08,
        softness: 0.02,
        weight: 0.8,
        weight_param: Some(name!("foot_placement_weight")),
        weight_halflife: 0.25,
        ignore_param: Some(name!("foot_placement_ignore")),
    };
    let defaults = FootPlacementNode::default();
    assert_ne!(node.pelvis, defaults.pelvis);
    assert_ne!(node.max_step_up, defaults.max_step_up);
    assert_ne!(node.max_step_down, defaults.max_step_down);
    assert_ne!(node.max_pelvis_drop, defaults.max_pelvis_drop);
    assert_ne!(node.max_foot_angle, defaults.max_foot_angle);
    assert_ne!(node.foot_halflife, defaults.foot_halflife);
    assert_ne!(node.pelvis_halflife, defaults.pelvis_halflife);
    assert_ne!(node.rotation_halflife, defaults.rotation_halflife);
    assert_ne!(node.softness, defaults.softness);
    assert_ne!(node.weight, defaults.weight);
    assert_ne!(node.weight_halflife, defaults.weight_halflife);
    let mut graph = AnimationGraph::from_node(node.clone());
    graph.make_live(&registry(), None).unwrap();

    let mut restored = round_trip(&graph);
    restored.make_live(&registry(), None).unwrap();

    assert!(
        restored.load_warnings().is_empty(),
        "{:?}",
        restored.load_warnings()
    );
    assert_eq!(live_placement(&restored), Some(&node));
}
