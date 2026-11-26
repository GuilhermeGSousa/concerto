use concerto_animation::{
    graph::AnimationGraph,
    node::{
        AnimationClipNode, NodeOutcome,
        state_machine::{AnimationFSMTrigger, AnimationStateMachine},
    },
};
use concerto_foundation::assets::asset_server::AssetServer;
use concerto_name::name;
use concerto_physics::physics_state::PhysicsState;
use concerto_physics::ragdoll::RagdollId;
use concerto_ragdoll::{
    BalanceNode, BalanceSettings, BodyHit, LimpNode, RagdollBody, ShotNode, Wounds, messages::Shot,
    testing::humanoid_skeleton,
};
use glam::Vec3;

use crate::balance::{Stage, over_clip, standing_clip};

const SURFACE: f32 = 0.12;

struct Bullet {
    bone: &'static str,
    impulse: Vec3,
}

#[derive(Debug)]
struct Answered {
    outcome: Option<NodeOutcome>,
    animated: bool,
    chest: Vec3,
    hand_gap: f32,
    early: Vec3,
    lowest: f32,
    feet: [f32; 2],
    wounded: f32,
}

fn shoot(stage: &mut Stage, bullet: &Bullet) -> Vec3 {
    let world = stage.app.main().world();
    let ragdoll = *world
        .get_component_for_entity::<RagdollId>(stage.character)
        .unwrap();
    let part = world
        .get_component_for_entity::<RagdollBody>(stage.character)
        .unwrap()
        .resolved()
        .part_of(bullet.bone)
        .unwrap();
    let mut pose = Vec::new();
    stage
        .app
        .get_resource::<PhysicsState>()
        .unwrap()
        .ragdoll_pose(ragdoll, &mut pose);
    let point = pose[part].translation - bullet.impulse.normalize() * SURFACE;
    stage
        .app
        .main_mut()
        .world_mut()
        .trigger(BodyHit::new(stage.character, part, point, bullet.impulse).shot());
    point
}

fn shooting_graph(server: &AssetServer) -> AnimationGraph {
    let skeleton = humanoid_skeleton();
    let clip = server.add(standing_clip(&skeleton));
    let walk = server.add(over_clip(
        BalanceNode::default()
            .with_settings(BalanceSettings::default().with_settle_seconds(f32::MAX)),
        &clip,
    ));
    let shot = server.add(over_clip(ShotNode::default(), &clip));
    let steady = server.add(AnimationGraph::from_node(AnimationClipNode::new(
        clip.clone(),
    )));
    let fallen = server.add(over_clip(LimpNode::default(), &clip));
    AnimationGraph::from_node(
        AnimationStateMachine::from_initial_state(name!("walk"), walk, |to| {
            to.to(
                name!("shot"),
                AnimationFSMTrigger::on_message::<Shot>(),
                0.0,
            );
        })
        .state(name!("shot"), shot, |to| {
            to.to(name!("steady"), AnimationFSMTrigger::OnSucceeded, 0.0)
                .to(name!("fallen"), AnimationFSMTrigger::OnFailed, 0.0);
        })
        .state(name!("steady"), steady, |_| {})
        .state(name!("fallen"), fallen, |_| {})
        .build(),
    )
}

fn shot(bullets: &[Bullet]) -> Answered {
    let mut stage = Stage::graphed(shooting_graph);
    stage.run(10);
    let start = stage.part("pelvis");
    let chest_start = stage.part("spine_03");
    let mut wound = Vec3::ZERO;
    for bullet in bullets {
        wound = shoot(&mut stage, bullet) - stage.part(bullet.bone);
    }
    let mut lowest = f32::MAX;
    let mut feet = [0.0_f32; 2];
    let wounded = bullets.last().unwrap().bone;
    let wound_part = stage
        .app
        .main()
        .world()
        .get_component_for_entity::<RagdollBody>(stage.character)
        .unwrap()
        .resolved()
        .part_of(wounded)
        .unwrap();
    let mut chest = Vec3::ZERO;
    let mut hand_gap = f32::MAX;
    let mut early = Vec3::ZERO;
    for frame in 0..240 {
        stage.run(1);
        let pelvis = stage.part("pelvis");
        lowest = lowest.min(pelvis.y);
        if frame < 30 {
            for (foot, height) in ["foot_l", "foot_r"].into_iter().zip(&mut feet) {
                *height = height.max(stage.part(foot).y);
            }
        }
        let moved = stage.part("spine_03") - chest_start;
        if frame < 20 && moved.length() > chest.length() {
            chest = moved;
        }
        if frame == 20 {
            early = pelvis - start;
        }
        for hand in ["hand_l", "hand_r"] {
            hand_gap = hand_gap.min(stage.part(hand).distance(stage.part(wounded) + wound));
        }
    }
    stage.run(2);
    Answered {
        outcome: stage.outcome(),
        animated: stage.animated(),
        chest,
        hand_gap,
        early,
        lowest,
        feet,
        wounded: stage
            .app
            .main()
            .world()
            .get_component_for_entity::<Wounds>(stage.character)
            .map_or(1.0, |wounds| wounds.strength_of(wound_part)),
    }
}

fn bullet(bone: &'static str, impulse: f32) -> Bullet {
    Bullet {
        bone,
        impulse: Vec3::new(0.0, 0.0, -impulse),
    }
}

#[test]
fn a_shot_body_answers_by_where_it_was_struck() {
    let chest = shot(&[bullet("spine_03", 40.0)]);
    assert!(
        chest.chest.z < -0.05 && chest.hand_gap < 0.15,
        "a shot to the chest from the front throws it back and brings a hand to the wound: {chest:?}"
    );
    assert_eq!(
        chest.outcome,
        Some(NodeOutcome::Succeeded),
        "a body that keeps its feet ends the reaction: {chest:?}"
    );
    assert!(
        chest.animated,
        "a body that keeps its feet ends the reaction drawn from its animation: {chest:?}"
    );
    assert_eq!(
        chest.wounded, 0.6,
        "the chest stays wounded after the shot ends: {chest:?}"
    );

    let heavy = shot(&[bullet("spine_03", 300.0)]);
    assert_eq!(
        heavy.outcome,
        Some(NodeOutcome::Failed),
        "a shot the body cannot take ends the reaction with balance's failure: {heavy:?}"
    );
    assert_eq!(
        heavy.wounded, 0.6,
        "the chest stays wounded after a shot that brings the body down: {heavy:?}"
    );
    assert!(
        heavy.lowest < 0.3,
        "a shot the body cannot take puts it on the ground: {heavy:?}"
    );

    let left = shot(&[bullet("thigh_l", 20.0)]);
    let right = shot(&[bullet("thigh_r", 20.0)]);
    assert!(
        left.feet[0] > left.feet[1] + 0.05 && right.feet[1] > right.feet[0] + 0.05,
        "a shot thigh takes its foot off the ground: {left:?} and {right:?}"
    );
    assert!(
        (left.feet[0] - right.feet[1]).abs() < 0.03
            && (left.feet[1] - right.feet[0]).abs() < 0.03
            && (left.early.x + right.early.x).abs() < 0.05,
        "the same shot on the left and on the right mirror each other: {left:?} and {right:?}"
    );

    let both = shot(&[bullet("thigh_l", 20.0), bullet("thigh_r", 20.0)]);
    assert!(
        both.lowest < 0.52 && both.outcome == Some(NodeOutcome::Failed),
        "shots to both thighs fold the knees and bring the pelvis under knee height: {both:?}"
    );
}
