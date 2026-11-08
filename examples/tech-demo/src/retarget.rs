use concerto::{
    animation::{
        graph::AnimationGraph,
        retarget::{ReferencePose, RetargetChain, RetargetGoal, RetargetNode, RetargetRig},
    },
    app::App,
    asset_id,
    ecs::Resource,
    foundation::assets::{AssetId, asset_server::AssetServer, handle::AssetHandle},
    mesh::Skeleton,
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
pub(crate) const UAL1_SKELETON: AssetId = asset_id!("content/UAL1/skeleton_0.gasset");
pub(crate) const LAFAN1_SKELETON: AssetId =
    asset_id!("content/lafan1/model_skeleton/skeleton_0.gasset");

/// Makes the player UAL1's character, playing LAFAN1's motion matching through a retarget node.
#[derive(Resource)]
pub(crate) struct RetargetedPlayer;

/// Builds the grid level with the player's character swapped for UAL1, driven by LAFAN1's motion matching through a retarget node.
pub fn retarget(app: &mut App) {
    grid(app);
    app.insert_resource(RetargetedPlayer);
}

/// The rig of LAFAN1's skeleton, the one the motion database is built on.
pub(crate) fn lafan1_rig(skeleton: AssetHandle<Skeleton>) -> RetargetRig {
    RetargetRig::new()
        .with_skeleton(skeleton)
        .with_root("Hips")
        .with_reference(ReferencePose::Rest)
        .with_chain(RetargetChain::new("spine", "Spine", "Spine2"))
        .with_chain(RetargetChain::new("neck", "Neck", "Head"))
        .with_chain(RetargetChain::new("arm_l", "LeftShoulder", "LeftHand"))
        .with_chain(RetargetChain::new("arm_r", "RightShoulder", "RightHand"))
        .with_chain(RetargetChain::new("leg_l", "LeftUpLeg", "LeftToe"))
        .with_chain(RetargetChain::new("leg_r", "RightUpLeg", "RightToe"))
        .with_goal(RetargetGoal::new("foot_l", "LeftFoot"))
        .with_goal(RetargetGoal::new("foot_r", "RightFoot"))
}

/// The rig of UAL1's skeleton, whose `root` bone carries the ground motion.
pub(crate) fn ual1_rig(skeleton: AssetHandle<Skeleton>) -> RetargetRig {
    RetargetRig::new()
        .with_skeleton(skeleton)
        .with_root("pelvis")
        .with_motion_root("root")
        .with_reference(ReferencePose::Rest)
        .with_chain(RetargetChain::new("spine", "spine_01", "spine_03"))
        .with_chain(RetargetChain::new("neck", "neck_01", "Head"))
        .with_chain(RetargetChain::new("arm_l", "clavicle_l", "hand_l"))
        .with_chain(RetargetChain::new("arm_r", "clavicle_r", "hand_r"))
        .with_chain(RetargetChain::new("leg_l", "thigh_l", "ball_l"))
        .with_chain(RetargetChain::new("leg_r", "thigh_r", "ball_r"))
        .with_goal(RetargetGoal::new("foot_l", "foot_l"))
        .with_goal(RetargetGoal::new("foot_r", "foot_r"))
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
    let source = asset_server.add(lafan1_rig(asset_server.load(LAFAN1_SKELETON)));
    let target = asset_server.add(ual1_rig(asset_server.load(UAL1_SKELETON)));
    CharacterAssets {
        scene: asset_server.load(UAL1_SCENE),
        graph: asset_server.add(retargeted_graph(RetargetNode::new(motion, source, target))),
    }
}
