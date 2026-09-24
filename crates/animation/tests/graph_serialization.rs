use concerto_animation::clip::AnimationClip;
use concerto_animation::graph::AnimationGraph;
use concerto_animation::node::state_machine::{AnimationFSMTrigger, AnimationStateMachine};
use concerto_animation::node::{AnimationClipNode, AnimationNodeKind};
use concerto_foundation::assets::AssetId;
use concerto_foundation::assets::asset_server::AssetServer;
use concerto_foundation::assets::asset_store::AssetStore;
use concerto_foundation::assets::handle::AssetHandle;

#[test]
fn animation_graph_round_trips_through_bincode() {
    let clip = |name: &str| AssetHandle::<AnimationClip>::weak(AssetId::from_path(name));

    let mut graph = AnimationGraph::new();
    graph
        .result_node()
        .with_blend_space_2d_input("movement", |ctx| {
            ctx.animation_clip_input(clip("idle"), glam::Vec2::ZERO)
                .animation_clip_input(clip("walk"), glam::Vec2::new(0.0, 1.0))
                .animation_clip_input(clip("strafe"), glam::Vec2::new(1.0, 0.0));
        });

    let bytes = bincode::serialize(&graph).expect("serialize");
    let restored: AnimationGraph = bincode::deserialize(&bytes).expect("deserialize");

    let count = |g: &AnimationGraph| g.iter().count();
    assert_eq!(count(&restored), count(&graph), "node count survives");

    // Blackboard param on the blend-space node survives.
    let has_movement_input = restored.iter().any(|idx| {
        matches!(
            restored.get_node(idx),
            Some(AnimationNodeKind::BlendSpace2D(def)) if def.input().param == "movement"
        )
    });
    assert!(
        has_movement_input,
        "BlendSpace2D input param survived the round trip"
    );
}

/// Spec §A7: a nested `StateMachine` with `BoolEquals` / `Vec2NonZero` /
/// `OnAnimationEnd` transitions must survive bincode with its trigger payloads
/// intact, and clip `AssetId`s must round-trip through the serialized graph.
#[test]
fn state_machine_graph_round_trips_trigger_payloads_and_clip_ids() {
    let clip = |name: &str| AssetHandle::<AnimationClip>::weak(AssetId::from_path(name));

    // Each FSM state points at a sub-graph asset. A unit test has no AssetServer
    // to resolve strong handles against, so register a store and mint real ones.
    let mut server = AssetServer::new();
    let graphs = AssetStore::<AnimationGraph>::new();
    server.register_asset(&graphs);

    let state_graph = |pose: AssetHandle<AnimationClip>| {
        let mut graph = AnimationGraph::new();
        graph.result_node().with_input(
            AnimationNodeKind::Clip(AnimationClipNode::new(pose)),
            |_| {},
        );
        graph
    };

    let state_machine = AnimationStateMachine::from_initial_state(
        "idle",
        server.add(state_graph(clip("idle"))),
        |transition| {
            transition.to("walk", AnimationFSMTrigger::on_bool("go", true), 0.2);
        },
    )
    .state(
        "walk",
        server.add(state_graph(clip("walk"))),
        |transition| {
            transition.to("run", AnimationFSMTrigger::on_non_zero_vec("move"), 0.15);
        },
    )
    .state("run", server.add(state_graph(clip("run"))), |transition| {
        transition.to("idle", AnimationFSMTrigger::OnAnimationEnd, 0.1);
    })
    .build();

    // The outer graph feeds the state machine plus a bare clip into its result;
    // the clip proves an AssetHandle round-trips through the serialized graph.
    let mut graph = AnimationGraph::new();
    graph
        .result_node()
        .with_input(AnimationNodeKind::StateMachine(state_machine), |_| {})
        .with_input(
            AnimationNodeKind::Clip(AnimationClipNode::new(clip("outer_pose"))),
            |_| {},
        );

    let bytes = bincode::serialize(&graph).expect("serialize");
    let restored: AnimationGraph = bincode::deserialize(&bytes).expect("deserialize");

    let count = |g: &AnimationGraph| g.iter().count();
    assert_eq!(count(&restored), count(&graph), "node count survives");
    assert_eq!(count(&restored), 3, "result + state machine + clip");

    let restored_machine = restored
        .iter()
        .find_map(|idx| match restored.get_node(idx) {
            Some(AnimationNodeKind::StateMachine(machine)) => Some(machine),
            _ => None,
        })
        .expect("state machine node survived");
    let triggers: Vec<&AnimationFSMTrigger> = restored_machine.triggers().collect();

    let bool_equals = triggers
        .iter()
        .find_map(|trigger| match trigger {
            AnimationFSMTrigger::BoolEquals { param, value } => Some((param.as_str(), *value)),
            _ => None,
        })
        .expect("BoolEquals trigger survived");
    assert_eq!(
        bool_equals,
        ("go", true),
        "BoolEquals payload survived the round trip"
    );

    let vec2_non_zero = triggers
        .iter()
        .find_map(|trigger| match trigger {
            AnimationFSMTrigger::Vec2NonZero { param } => Some(param.as_str()),
            _ => None,
        })
        .expect("Vec2NonZero trigger survived");
    assert_eq!(
        vec2_non_zero, "move",
        "Vec2NonZero payload survived the round trip"
    );

    assert!(
        triggers
            .iter()
            .any(|trigger| matches!(trigger, AnimationFSMTrigger::OnAnimationEnd)),
        "OnAnimationEnd trigger survived the round trip"
    );

    let restored_clip_id = restored
        .iter()
        .find_map(|idx| match restored.get_node(idx) {
            Some(AnimationNodeKind::Clip(node)) => Some(node.clip().id()),
            _ => None,
        })
        .expect("clip node survived");
    assert_eq!(
        restored_clip_id,
        AssetId::from_path("outer_pose"),
        "clip AssetId survived the round trip"
    );
}
