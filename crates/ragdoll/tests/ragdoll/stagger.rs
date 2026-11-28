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
    BALANCE, Balance, BalanceNode, BalanceSettings, BodyState, LimpNode, Outcome, STAGGER, Stagger,
    StaggerNode,
    messages::{self, Steadied, Toppled},
    testing::humanoid_skeleton,
};
use glam::Vec3;

use crate::balance::{Stage, over_clip, standing_clip};

#[derive(Debug)]
struct Stumble {
    steps: u32,
    reach: f32,
}

fn stumble(
    stage: &mut Stage,
    ended: impl Fn(&Stage) -> bool,
    stepping: impl Fn(&Stage) -> bool,
) -> Stumble {
    let mut steps = 0;
    let mut was_stepping = false;
    let mut reach: f32 = 0.0;
    let held = ["hand_l", "hand_r"].map(|hand| stage.part(hand) - stage.part("spine_03"));
    for _ in 0..300 {
        if ended(stage) {
            break;
        }
        let chest = stage.part("spine_03");
        for (hand, held) in ["hand_l", "hand_r"].into_iter().zip(held) {
            reach = reach.max((stage.part(hand) - chest - held).length());
        }
        let now_stepping = stepping(stage);
        if now_stepping && !was_stepping {
            steps += 1;
        }
        was_stepping = now_stepping;
        stage.run(1);
    }
    Stumble { steps, reach }
}

#[derive(Debug)]
struct Stumbled {
    balance: Option<Outcome>,
    stagger: Option<Outcome>,
    stumble: Stumble,
    state: BodyState,
}

fn stumbled(stagger: Option<Stagger>, impulse: Vec3) -> Stumbled {
    let mut stage = Stage::new();
    stage.insert(Balance::default());
    stage.run(15);
    stage.push("spine_03", impulse);
    if let Some(stagger) = stagger {
        stage.insert(stagger);
    }
    let stumble = stumble(
        &mut stage,
        |stage| {
            stage
                .ended()
                .iter()
                .any(|ended| ended.behaviour == Name::from(BALANCE))
        },
        |stage| stage.balance().is_some_and(|balance| balance.stepping()),
    );
    stage.run(1);
    let outcome = |name: &str| {
        stage
            .ended()
            .iter()
            .find(|ended| ended.behaviour == Name::from(name))
            .map(|ended| ended.outcome)
    };
    Stumbled {
        balance: outcome(BALANCE),
        stagger: outcome(STAGGER),
        stumble,
        state: stage.state(),
    }
}

fn staggering_graph(server: &AssetServer) -> AnimationGraph {
    let clip = server.add(standing_clip(&humanoid_skeleton()));
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
                AnimationFSMTrigger::on_message::<messages::Stagger>(),
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

#[derive(Debug)]
struct Graphed {
    entries: Vec<Name>,
    stumble: Stumble,
    pelvis: f32,
}

fn stumbled_in_graph(stagger: Option<messages::Stagger>, impulse: Vec3) -> Graphed {
    let mut stage = match stagger {
        Some(_) => Stage::graphed(staggering_graph),
        None => Stage::balancing(BalanceNode::default()),
    };
    stage.run(15);
    if let Some(stagger) = stagger {
        stage.send(stagger);
        stage.run(1);
    }
    stage.push("spine_03", impulse);
    let stumble = stumble(
        &mut stage,
        |stage| stage.in_state(name!("steady")) || stage.in_state(name!("fallen")),
        Stage::node_stepping,
    );
    stage.run(120);
    Graphed {
        entries: stage.entries().to_vec(),
        stumble,
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
        assert_eq!(alone.balance, Some(Outcome::Succeeded), "{alone:?}");

        let mild = stumbled(Some(Stagger::new(direction, 0.3)), push);
        assert_eq!(
            (mild.balance, mild.stagger, mild.state),
            (
                Some(Outcome::Succeeded),
                Some(Outcome::Succeeded),
                BodyState::Animated
            ),
            "a mild stagger towards {direction} ends standing, with balance's outcome: {mild:?}"
        );
        assert!(
            mild.stumble.reach > alone.stumble.reach + 0.05,
            "the hands travel further from where the chest held them in a stagger towards {direction}: {mild:?} against {alone:?}"
        );
        steps_alone += alone.stumble.steps;
        steps_staggering += mild.stumble.steps;

        let severe = stumbled(Some(Stagger::new(direction, 1.0)), push);
        assert_eq!(
            (severe.balance, severe.stagger, severe.state),
            (
                Some(Outcome::GaveUp),
                Some(Outcome::GaveUp),
                BodyState::Falling
            ),
            "a severe stagger towards {direction} cannot be recovered: {severe:?}"
        );
    }
    assert!(
        steps_staggering > steps_alone,
        "over the four directions a mild stagger takes more steps than balance alone: {steps_staggering} against {steps_alone}"
    );

    let mut steps_alone = 0;
    let mut steps_staggering = 0;
    for direction in [Vec3::Z, Vec3::NEG_Z, Vec3::X, Vec3::NEG_X] {
        let push = direction * 70.0;
        let alone = stumbled_in_graph(None, push);
        assert_eq!(
            alone.entries,
            [name!("balance"), name!("steady")],
            "{alone:?}"
        );

        let mild = stumbled_in_graph(
            Some(messages::Stagger {
                direction,
                severity: 0.3,
            }),
            push,
        );
        assert_eq!(
            mild.entries,
            [name!("stand"), name!("stagger"), name!("steady")],
            "a mild stagger from the graph towards {direction} ends steady, entered once and kept: {mild:?}"
        );
        assert!(
            mild.stumble.reach > alone.stumble.reach + 0.05,
            "the hands travel further from where the chest held them in a stagger from the graph towards {direction}: {mild:?} against {alone:?}"
        );
        steps_alone += alone.stumble.steps;
        steps_staggering += mild.stumble.steps;

        let severe = stumbled_in_graph(
            Some(messages::Stagger {
                direction,
                severity: 1.0,
            }),
            push,
        );
        assert_eq!(
            severe.entries,
            [name!("stand"), name!("stagger"), name!("fallen")],
            "a severe stagger from the graph towards {direction} cannot be recovered, and falls once and stays fallen: {severe:?}"
        );
        assert!(
            severe.pelvis < 0.4,
            "a severe stagger from the graph towards {direction} ends on the ground: {severe:?}"
        );
    }
    assert!(
        steps_staggering > steps_alone,
        "over the four directions a mild stagger from the graph takes more steps than balance alone: {steps_staggering} against {steps_alone}"
    );
}
