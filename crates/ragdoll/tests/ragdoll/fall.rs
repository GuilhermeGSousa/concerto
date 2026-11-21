use concerto_physics::physics_state::PhysicsState;
use concerto_physics::ragdoll::RagdollId;
use concerto_ragdoll::{
    Balance, BodySense, BodyState, CharacterRagdoll, FallReaction, RagdollBody, RagdollMode,
};
use glam::Vec3;

use crate::balance::Stage;

#[derive(Debug)]
struct Fallen {
    hands_down: [Option<usize>; 2],
    chest_down: Option<usize>,
    head_peak: f32,
    state: BodyState,
    mode: RagdollMode,
    resting: bool,
}

fn impulse(stage: &Stage, bone: &str) -> Option<f32> {
    let world = stage.app.main().world();
    let ragdoll = *world
        .get_component_for_entity::<RagdollId>(stage.character)
        .unwrap();
    let part = world
        .get_component_for_entity::<RagdollBody>(stage.character)
        .unwrap()
        .resolved()
        .part_of(bone)
        .unwrap();
    let state = stage.app.get_resource::<PhysicsState>().unwrap();
    state
        .contacts_of(state.ragdoll_bodies(ragdoll)[part])
        .iter()
        .filter(|contact| contact.other != Some(stage.character))
        .map(|contact| contact.impulse)
        .reduce(f32::max)
}

fn fallen(strength: f32, push: Vec3) -> Fallen {
    let mut stage = Stage::new();
    stage.insert(FallReaction { strength });
    stage.insert(Balance::default());
    stage.run(15);
    stage.push("spine_03", push);
    let mut hands_down = [None; 2];
    let mut chest_down = None;
    let mut head_peak: f32 = 0.0;
    for frame in 0..300 {
        stage.run(1);
        for (hand, down) in ["hand_l", "hand_r"].into_iter().zip(&mut hands_down) {
            if down.is_none() && impulse(&stage, hand).is_some() {
                *down = Some(frame);
            }
        }
        if chest_down.is_none() && impulse(&stage, "spine_03").is_some() {
            chest_down = Some(frame);
        }
        head_peak = head_peak.max(impulse(&stage, "Head").unwrap_or(0.0));
    }
    let world = stage.app.main().world();
    Fallen {
        hands_down,
        chest_down,
        head_peak,
        state: stage.state(),
        mode: world
            .get_component_for_entity::<CharacterRagdoll>(stage.character)
            .unwrap()
            .mode,
        resting: world
            .get_component_for_entity::<BodySense>(stage.character)
            .unwrap()
            .at_rest,
    }
}

#[test]
fn a_falling_body_lands_on_its_hands_spares_its_head_and_ends_down_and_limp() {
    for direction in [Vec3::Z, Vec3::NEG_Z] {
        let limp = fallen(0.0, direction * 300.0);
        let braced = fallen(1.0, direction * 300.0);
        assert!(
            braced.head_peak < limp.head_peak * 0.5,
            "falling towards {direction} the head lands softer braced than limp: {braced:?} against {limp:?}"
        );
        for fall in [&limp, &braced] {
            assert!(
                fall.state == BodyState::Down && fall.mode == RagdollMode::Limp && fall.resting,
                "a fall towards {direction} ends down, limp and at rest: {fall:?}"
            );
        }
        if direction == Vec3::Z {
            let chest = braced.chest_down.unwrap_or(usize::MAX);
            assert!(
                braced
                    .hands_down
                    .iter()
                    .all(|hand| hand.is_some_and(|hand| hand < chest)),
                "falling forward both hands touch the ground before the chest: {braced:?}"
            );
        }
    }
}
