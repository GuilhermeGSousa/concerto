use concerto_name::Name;
use concerto_physics::physics_state::PhysicsState;
use concerto_physics::ragdoll::RagdollId;
use concerto_ragdoll::{
    BALANCE, BodyHit, BodyState, Outcome, RagdollBody, SHOT, ShotReaction, Wounds,
};
use glam::Vec3;

use crate::balance::Stage;

const SURFACE: f32 = 0.12;

struct Bullet {
    bone: &'static str,
    impulse: Vec3,
}

#[derive(Debug)]
struct Shot {
    balance: Option<Outcome>,
    shot: Option<Outcome>,
    state: BodyState,
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

fn shot(bullets: &[Bullet]) -> Shot {
    let mut stage = Stage::new();
    stage.insert(ShotReaction::default());
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
    let outcome = |name: &str| {
        stage
            .ended()
            .iter()
            .find(|ended| ended.behaviour == Name::from(name))
            .map(|ended| ended.outcome)
    };
    Shot {
        balance: outcome(BALANCE),
        shot: outcome(SHOT),
        state: stage.state(),
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
        (chest.balance, chest.shot, chest.state),
        (
            Some(Outcome::Succeeded),
            Some(Outcome::Succeeded),
            BodyState::Animated
        ),
        "a body that keeps its feet ends the reaction standing: {chest:?}"
    );
    assert_eq!(
        chest.wounded, 0.6,
        "the chest stays wounded after the reaction"
    );

    let heavy = shot(&[bullet("spine_03", 300.0)]);
    assert_eq!(
        (heavy.balance, heavy.shot),
        (Some(Outcome::GaveUp), Some(Outcome::GaveUp)),
        "a shot the body cannot take ends the reaction with balance's failure: {heavy:?}"
    );
    assert!(matches!(heavy.state, BodyState::Falling | BodyState::Down));

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
        both.lowest < 0.52 && both.balance == Some(Outcome::GaveUp),
        "shots to both thighs fold the knees and bring the pelvis under knee height: {both:?}"
    );
}
