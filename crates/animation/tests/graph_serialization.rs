use concerto_animation::clip::AnimationClip;
use concerto_animation::graph::AnimationGraph;
use concerto_animation::node::AnimationClipNode;
use concerto_animation::node::state_machine::AnimationStateMachine;
use concerto_foundation::assets::AssetId;
use concerto_foundation::assets::handle::AssetHandle;

#[test]
fn animation_graph_builds_nodes_through_the_generic_builders() {
    let clip = |name: &str| AssetHandle::<AnimationClip>::weak(AssetId::from_path(name));

    let mut graph = AnimationGraph::new();
    graph
        .result_node()
        .with_blend_space_2d_input("movement", |ctx| {
            ctx.animation_clip_input(clip("idle"), glam::Vec2::ZERO)
                .animation_clip_input(clip("walk"), glam::Vec2::new(0.0, 1.0))
                .animation_clip_input(clip("strafe"), glam::Vec2::new(1.0, 0.0));
        })
        .with_input(AnimationClipNode::new(clip("outer")), |_| {})
        .with_input(AnimationStateMachine::default(), |_| {});

    assert_eq!(graph.iter().count(), 7);
}
