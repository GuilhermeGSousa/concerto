use concerto_animation::{
    graph::AnimationGraph,
    node::{
        AnimationClipNode,
        state_machine::{AnimationFSMTrigger, AnimationStateMachine},
    },
};
use concerto_foundation::assets::asset_server::AssetServer;
use concerto_name::{Name, name};
use concerto_ragdoll::{
    BalanceNode, BalanceSettings, LimpNode, StaggerNode,
    messages::{Stagger, Steadied, Toppled},
    testing::humanoid_skeleton,
};
use glam::Vec3;

use crate::balance::{Stage, over_clip, standing_clip};

#[derive(Debug)]
struct Stumbled {
    entries: Vec<Name>,
    steps: u32,
    reach: f32,
    animated: bool,
    pelvis: f32,
}

fn staggering_graph(server: &AssetServer) -> AnimationGraph {
    let skeleton = humanoid_skeleton();
    let clip = server.add(standing_clip(&skeleton));
    let stand = server.add(over_clip(
        BalanceNode::default()
            .with_settings(BalanceSettings::default().with_settle_seconds(f32::MAX)),
        &clip,
    ));
    let stagger = server.add(over_clip(StaggerNode::default(), &clip));
    let steady = server.add(AnimationGraph::from_node(AnimationClipNode::new(
        clip.clone(),
    )));
    let fallen = server.add(over_clip(LimpNode::default(), &clip));
    AnimationGraph::from_node(
        AnimationStateMachine::from_initial_state(name!("stand"), stand, |to| {
            to.to(
                name!("stagger"),
                AnimationFSMTrigger::on_message::<Stagger>(),
                0.0,
            );
        })
        .state(name!("stagger"), stagger, |to| {
            to.to(
                name!("steady"),
                AnimationFSMTrigger::on_message::<Steadied>(),
                0.0,
            )
            .to(
                name!("fallen"),
                AnimationFSMTrigger::on_message::<Toppled>(),
                0.0,
            );
        })
        .state(name!("steady"), steady, |_| {})
        .state(name!("fallen"), fallen, |_| {})
        .build(),
    )
}

fn stumbled(stagger: Option<Stagger>, impulse: Vec3) -> Stumbled {
    let mut stage = match stagger {
        Some(_) => Stage::graphed(staggering_graph),
        None => Stage::balancing(Default::default()),
    };
    stage.run(15);
    if let Some(stagger) = stagger {
        stage.send(stagger);
        stage.run(1);
    }
    stage.push("spine_03", impulse);
    let mut steps = 0;
    let mut stepping = false;
    let mut reach: f32 = 0.0;
    let held = ["hand_l", "hand_r"].map(|hand| stage.part(hand) - stage.part("spine_03"));
    for _ in 0..300 {
        if stage.outcome().is_some() {
            break;
        }
        let chest = stage.part("spine_03");
        for (hand, held) in ["hand_l", "hand_r"].into_iter().zip(held) {
            reach = reach.max((stage.part(hand) - chest - held).length());
        }
        if stage.stepping() && !stepping {
            steps += 1;
        }
        stepping = stage.stepping();
        stage.run(1);
    }
    stage.run(120);
    Stumbled {
        entries: stage.entries().to_vec(),
        steps,
        reach,
        animated: stage.animated(),
        pelvis: stage.part("pelvis").y,
    }
}

#[test]
fn a_stagger_stumbles_further_than_balance_with_arms_out_and_a_severe_one_falls() {
    let mut steps_alone = 0;
    let mut steps_staggering = 0;
    for direction in [Vec3::Z, Vec3::NEG_Z, Vec3::X, Vec3::NEG_X] {
        let push = direction * 70.0;
        let alone = stumbled(None, push);
        assert_eq!(
            alone.entries,
            [name!("balance"), name!("steady")],
            "{alone:?}"
        );

        let mild = stumbled(
            Some(Stagger {
                direction,
                severity: 0.3,
            }),
            push,
        );
        assert_eq!(
            mild.entries,
            [name!("stand"), name!("stagger"), name!("steady")],
            "a mild stagger towards {direction} ends steady, entered once and kept: {mild:?}"
        );
        assert!(
            mild.animated,
            "a mild stagger towards {direction} ends drawn from its animation: {mild:?}"
        );
        assert!(
            mild.reach > alone.reach + 0.05,
            "the hands travel further from where the chest held them in a stagger towards {direction}: {mild:?} against {alone:?}"
        );
        steps_alone += alone.steps;
        steps_staggering += mild.steps;

        let severe = stumbled(
            Some(Stagger {
                direction,
                severity: 1.0,
            }),
            push,
        );
        assert_eq!(
            severe.entries,
            [name!("stand"), name!("stagger"), name!("fallen")],
            "a severe stagger towards {direction} cannot be recovered, and falls once and stays fallen: {severe:?}"
        );
        assert!(
            severe.pelvis < 0.4,
            "a severe stagger towards {direction} ends on the ground: {severe:?}"
        );
    }
    assert!(
        steps_staggering > steps_alone,
        "over the four directions a mild stagger takes more steps than balance alone: {steps_staggering} against {steps_alone}"
    );
}
