use concerto::{
    animation::{
        clip::{Additive, AdditiveBase, AnimationClip},
        graph::{AnimationGraph, AnimationNodeIndex},
        mask::{BoneMask, BoneMaskBranch},
        node::{
            AdditiveNode, AimOffsetNode, AimPoint, AnimationClipNode, AnimationPlayMode, IkGoal,
            LayerNode, LayerSpace, TipRotation, TwoBoneIkNode,
        },
        retarget::RetargetNode,
    },
    app::App,
    asset_id,
    ecs::Resource,
    foundation::assets::{AssetId, asset_server::AssetServer, handle::AssetHandle},
    motion_matching::{MotionDatabase, MotionMatchingNode},
    name::{Name, name},
    procedural_animation::{
        AimBone, AimNode, FootLockLeg, FootLockNode, FootPlacementLeg, FootPlacementNode,
        LookAtBone, LookAtNode, Muzzle, ProceduralMotionChannel, ProceduralMotionNode,
    },
};
use glam::{Quat, Vec3};

use crate::{
    character::{
        CharacterAssets, DATABASE, foot_lock_weight, foot_placement_ignore, foot_placement_weight,
    },
    grid::grid,
    sanatorium::sanatorium,
};

pub(crate) const UAL1_SCENE: AssetId = asset_id!("content/UAL1/scene.gasset");
pub(crate) const UAL1_RIG: AssetId = asset_id!("content/UAL1/rig.gasset");
pub(crate) const LAFAN1_RIG: AssetId = asset_id!("content/lafan1/model_skeleton/rig.gasset");

pub(crate) const PISTOL_IDLE: AssetId = asset_id!("content/UAL1/animation_Pistol_Idle_Loop.gasset");
pub(crate) const PISTOL_AIM_NEUTRAL: AssetId =
    asset_id!("content/UAL1/animation_Pistol_Aim_Neutral.gasset");
pub(crate) const PISTOL_AIM_UP: AssetId = asset_id!("content/UAL1/animation_Pistol_Aim_Up.gasset");
pub(crate) const PISTOL_AIM_DOWN: AssetId =
    asset_id!("content/UAL1/animation_Pistol_Aim_Down.gasset");
pub(crate) const PISTOL_SHOOT: AssetId = asset_id!("content/UAL1/animation_Pistol_Shoot.gasset");
pub(crate) const IDLE: AssetId = asset_id!("content/UAL1/animation_Idle_Loop.gasset");
pub(crate) const IDLE_TIRED: AssetId = asset_id!("content/UAL1/animation_Idle_Tired_Loop.gasset");

const FINGERS: [&str; 10] = [
    "thumb_01_l",
    "index_01_l",
    "middle_01_l",
    "ring_01_l",
    "pinky_01_l",
    "thumb_01_r",
    "index_01_r",
    "middle_01_r",
    "ring_01_r",
    "pinky_01_r",
];
const AIM_UP_PITCH: f32 = 1.49;
const AIM_DOWN_PITCH: f32 = -1.55;
const BARREL: Vec3 = Vec3::new(0.03, 0.982, -0.184);
const MUZZLE_DISTANCE: f32 = 0.15;
const OFF_HAND_TRANSLATION: Vec3 = Vec3::new(-0.067, -0.045, -0.058);
const OFF_HAND_ROTATION: Quat = Quat::from_xyzw(-0.097, -0.418, -0.140, 0.892);

/// Makes the player UAL1's character, playing LAFAN1's motion matching through a retarget node.
#[derive(Resource)]
pub(crate) struct RetargetedPlayer;

/// Builds the grid level with the player's character swapped for UAL1, driven by LAFAN1's motion matching through a retarget node.
pub fn retarget(app: &mut App) {
    grid(app);
    app.insert_resource(RetargetedPlayer);
}

/// Builds the sanatorium with the player's character swapped for UAL1, as `retarget` does on the grid.
pub fn retargeted_sanatorium(app: &mut App) {
    sanatorium(app);
    app.insert_resource(RetargetedPlayer);
}

/// The float param that lays the pistol over the upper body, keeps the off hand on it and lets a shot kick.
pub(crate) fn pistol_weight() -> Name {
    name!("pistol_weight")
}

/// The float param that turns the aim offset and the aim on.
pub(crate) fn aim_weight() -> Name {
    name!("aim_weight")
}

/// The world-space `Vec3` param the gun and the head turn toward.
pub(crate) fn aim_target() -> Name {
    name!("aim_target")
}

/// The float param that turns the head toward the aim target.
pub(crate) fn look_weight() -> Name {
    name!("look_weight")
}

/// The float param that weighs the tired overlay and deepens the breathing.
pub(crate) fn fatigue() -> Name {
    name!("fatigue")
}

/// The float param that scales the aim arm's sway.
pub(crate) fn sway() -> Name {
    name!("sway")
}

/// The float param that scales the gun hand's tremor.
pub(crate) fn tremor() -> Name {
    name!("tremor")
}

/// The float param that scales the playback rate of the locomotion.
pub(crate) fn pace() -> Name {
    name!("pace")
}

struct BodyClips {
    pistol_idle: AssetHandle<AnimationClip>,
    aim_level: AssetHandle<AnimationClip>,
    aim_up: AssetHandle<AnimationClip>,
    aim_down: AssetHandle<AnimationClip>,
    shoot: AssetHandle<AnimationClip>,
    tired: AssetHandle<AnimationClip>,
    idle: AssetHandle<AnimationClip>,
}

impl BodyClips {
    fn load(asset_server: &AssetServer) -> Self {
        let mut level = AnimationClip::default();
        level.set_additive(Additive::LocalSpace(AdditiveBase::new(
            asset_server.load(PISTOL_AIM_NEUTRAL),
        )));
        Self {
            pistol_idle: asset_server.load(PISTOL_IDLE),
            aim_level: asset_server.add(level),
            aim_up: asset_server.load(PISTOL_AIM_UP),
            aim_down: asset_server.load(PISTOL_AIM_DOWN),
            shoot: asset_server.load(PISTOL_SHOOT),
            tired: asset_server.load(IDLE_TIRED),
            idle: asset_server.load(IDLE),
        }
    }
}

struct BodyMasks {
    upper_body: AssetHandle<BoneMask>,
    chest: AssetHandle<BoneMask>,
    aim_arm: AssetHandle<BoneMask>,
    gun_hand: AssetHandle<BoneMask>,
    fingers: AssetHandle<BoneMask>,
}

impl BodyMasks {
    fn add(asset_server: &AssetServer) -> Self {
        let mask = |bone: &str, depth: u32| {
            asset_server
                .add(BoneMask::new().with_branch(BoneMaskBranch::new(bone).with_depth(depth)))
        };
        Self {
            upper_body: mask("spine_01", 2),
            chest: mask("spine_02", 1),
            aim_arm: mask("upperarm_r", 0),
            gun_hand: mask("hand_r", 0),
            fingers: asset_server.add(FINGERS.iter().fold(BoneMask::new(), |mask, finger| {
                mask.with_branch(BoneMaskBranch::new(*finger))
            })),
        }
    }
}

fn motion_matching_graph(database: AssetHandle<MotionDatabase>) -> AnimationGraph {
    let mut graph = AnimationGraph::new();
    graph
        .result_node()
        .add_input(MotionMatchingNode::new(database).with_play_rate_param(pace()));
    graph
}

fn foot_placement() -> FootPlacementNode {
    FootPlacementNode::new("pelvis")
        .with_leg(FootPlacementLeg::new("foot_l").with_toe("ball_l"))
        .with_leg(FootPlacementLeg::new("foot_r").with_toe("ball_r"))
        .with_weight_param(foot_placement_weight())
        .with_ignore_param(foot_placement_ignore())
}

fn foot_lock() -> FootLockNode {
    FootLockNode::new()
        .with_leg(FootLockLeg::new("foot_l").with_toe("ball_l"))
        .with_leg(FootLockLeg::new("foot_r").with_toe("ball_r"))
        .with_weight_param(foot_lock_weight())
}

fn off_hand() -> TwoBoneIkNode {
    TwoBoneIkNode {
        tip: "hand_l".into(),
        target: IkGoal::Bone {
            bone: "hand_r".into(),
            translation: OFF_HAND_TRANSLATION,
            rotation: OFF_HAND_ROTATION.normalize(),
        },
        tip_rotation: TipRotation::FromTarget,
        weight_param: Some(pistol_weight().into()),
        ..Default::default()
    }
}

fn unsteadiness(masks: &BodyMasks) -> ProceduralMotionNode {
    ProceduralMotionNode::new()
        .with_channel(
            ProceduralMotionChannel::breathing(masks.chest.clone())
                .with_frequency(0.45)
                .with_amplitude(Vec3::new(4.0_f32.to_radians(), 0.0, 0.0))
                .with_magnitude_param(fatigue())
                .with_magnitude_halflife(0.3),
        )
        .with_channel(
            ProceduralMotionChannel::sway(masks.aim_arm.clone())
                .with_magnitude_param(sway())
                .with_magnitude_halflife(0.3),
        )
        .with_channel(
            ProceduralMotionChannel::tremor(masks.gun_hand.clone())
                .with_magnitude_param(tremor())
                .with_magnitude_halflife(0.3),
        )
}

fn look() -> LookAtNode {
    LookAtNode {
        chain: vec![
            LookAtBone {
                bone: "neck_01".into(),
                share: 1.0,
                halflife: 0.2,
                ..Default::default()
            },
            LookAtBone {
                bone: "Head".into(),
                share: 2.0,
                ..Default::default()
            },
        ],
        target_param: aim_target().into(),
        weight_param: Some(look_weight().into()),
        ..Default::default()
    }
}

fn recoil(shoot: AssetHandle<AnimationClip>) -> AdditiveNode {
    AdditiveNode::new(shoot)
        .with_play_mode(AnimationPlayMode::PlayOnce)
        .with_weight_param(pistol_weight())
}

fn aim() -> AimNode {
    let barrel = BARREL.normalize();
    AimNode::new(
        aim_target(),
        Muzzle::new("hand_r")
            .with_offset(barrel * MUZZLE_DISTANCE)
            .with_direction(barrel),
    )
    .with_bone(AimBone::new("spine_02").with_share(1.0))
    .with_bone(AimBone::new("spine_03").with_share(1.0))
    .with_bone(AimBone::new("upperarm_r").with_share(2.0))
    .with_weight_param(aim_weight())
    .with_reacquire_halflife(0.06)
}

fn aim_offset(clips: &BodyClips) -> AimOffsetNode {
    AimOffsetNode::new(aim_target(), "spine_03")
        .with_point(AimPoint::new(clips.aim_down.clone(), 0.0, AIM_DOWN_PITCH))
        .with_point(AimPoint::new(clips.aim_level.clone(), 0.0, 0.0))
        .with_point(AimPoint::new(clips.aim_up.clone(), 0.0, AIM_UP_PITCH))
        .with_weight_param(aim_weight())
}

struct BodyGraph {
    graph: AnimationGraph,
    recoil: AnimationNodeIndex,
}

fn retargeted_graph(retarget: RetargetNode, clips: BodyClips, masks: BodyMasks) -> BodyGraph {
    let mut graph = AnimationGraph::new();
    let recoil_node = graph
        .result_node()
        .input(foot_placement())
        .input(foot_lock())
        .input(off_hand())
        .input(unsteadiness(&masks))
        .input(look())
        .input(recoil(clips.shoot.clone()));
    let recoil = recoil_node.index();
    recoil_node
        .input(aim())
        .input(aim_offset(&clips))
        .add_layer_input_with(
            LayerNode::new(masks.upper_body.clone()).with_weight_param(pistol_weight()),
            |base| {
                base.input(AdditiveNode::new(clips.tired.clone()).with_weight_param(fatigue()))
                    .add_layer_input(
                        LayerNode::new(masks.fingers.clone()).with_space(LayerSpace::Local),
                        retarget,
                        AnimationClipNode::new(clips.idle.clone()),
                    );
            },
            |mut overlay| {
                overlay.add_input(AnimationClipNode::new(clips.pistol_idle.clone()));
            },
        );
    BodyGraph { graph, recoil }
}

/// UAL1's character: LAFAN1's motion matching retargeted onto its skeleton, its fingers, which LAFAN1's motion does not move, taken from UAL1's own idle, a tired overlay over both, the pistol layered over the upper body and aimed, the head turned to the aim point, the aim arm swaying, the off hand held on the gun, and the feet locked and placed last.
pub(crate) fn retargeted_character(asset_server: &AssetServer) -> CharacterAssets {
    let motion = asset_server.add(motion_matching_graph(asset_server.load(DATABASE)));
    let source = asset_server.load(LAFAN1_RIG);
    let target = asset_server.load(UAL1_RIG);
    let body = retargeted_graph(
        RetargetNode::new(motion, source, target),
        BodyClips::load(asset_server),
        BodyMasks::add(asset_server),
    );
    CharacterAssets {
        scene: asset_server.load(UAL1_SCENE),
        graph: asset_server.add(body.graph),
        recoil: Some(body.recoil),
    }
}
