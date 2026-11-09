use concerto::{
    animation::{graph::AnimationGraph, retarget::RetargetNode},
    app::App,
    asset_id,
    ecs::Resource,
    foundation::assets::{AssetId, asset_server::AssetServer, handle::AssetHandle},
    motion_matching::{MotionDatabase, MotionMatchingNode},
    procedural_animation::{FootLockLeg, FootLockNode, FootPlacementLeg, FootPlacementNode},
};

use crate::{
    character::{
        CharacterAssets, DATABASE, foot_lock_weight, foot_placement_ignore, foot_placement_weight,
    },
    grid::grid,
};

pub(crate) const UAL1_SCENE: AssetId = asset_id!("content/UAL1/scene.gasset");
pub(crate) const UAL1_RIG: AssetId = asset_id!("content/UAL1/rig.gasset");
pub(crate) const LAFAN1_RIG: AssetId = asset_id!("content/lafan1/model_skeleton/rig.gasset");

/// Makes the player UAL1's character, playing LAFAN1's motion matching through a retarget node.
#[derive(Resource)]
pub(crate) struct RetargetedPlayer;

/// Builds the grid level with the player's character swapped for UAL1, driven by LAFAN1's motion matching through a retarget node.
pub fn retarget(app: &mut App) {
    grid(app);
    app.insert_resource(RetargetedPlayer);
}

fn motion_matching_graph(database: AssetHandle<MotionDatabase>) -> AnimationGraph {
    let mut graph = AnimationGraph::new();
    graph.result_node().with_input(
        MotionMatchingNode {
            database,
            ..Default::default()
        },
        |_| {},
    );
    graph
}

fn retargeted_graph(retarget: RetargetNode) -> AnimationGraph {
    let mut graph = AnimationGraph::new();
    graph.result_node().with_input(
        FootPlacementNode::new("pelvis")
            .with_leg(FootPlacementLeg::new("foot_l").with_toe("ball_l"))
            .with_leg(FootPlacementLeg::new("foot_r").with_toe("ball_r"))
            .with_weight_param(foot_placement_weight())
            .with_ignore_param(foot_placement_ignore()),
        |mut placement| {
            placement.with_input(
                FootLockNode::new()
                    .with_leg(FootLockLeg::new("foot_l").with_toe("ball_l"))
                    .with_leg(FootLockLeg::new("foot_r").with_toe("ball_r"))
                    .with_weight_param(foot_lock_weight()),
                |mut lock| {
                    lock.with_input(retarget, |_| {});
                },
            );
        },
    );
    graph
}

/// UAL1's character, playing LAFAN1's motion matching retargeted onto its skeleton, then locking and placing its feet.
pub(crate) fn retargeted_character(asset_server: &AssetServer) -> CharacterAssets {
    let motion = asset_server.add(motion_matching_graph(asset_server.load(DATABASE)));
    let source = asset_server.load(LAFAN1_RIG);
    let target = asset_server.load(UAL1_RIG);
    CharacterAssets {
        scene: asset_server.load(UAL1_SCENE),
        graph: asset_server.add(retargeted_graph(RetargetNode::new(motion, source, target))),
    }
}
