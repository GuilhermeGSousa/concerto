use concerto_animation::{
    clip::{AnimationChanelOutput, AnimationChannel, AnimationClip},
    graph::AnimationGraph,
    node::{
        AnimationClipNode,
        state_machine::{AnimationFSMTrigger, AnimationStateMachine},
    },
};
use concerto_foundation::assets::asset_server::AssetServer;
use concerto_foundation::transform::Transform;
use concerto_mesh::Skeleton;
use concerto_name::{Name, name};
use concerto_physics::ragdoll::RagdollId;
use concerto_physics::{collider::Collider, physics_state::PhysicsState};
use concerto_ragdoll::{
    BalanceNode, BalanceSettings, BodySense, FallNode, LimpNode, RagdollBody,
    messages::{BodyDown, Toppled},
    testing::humanoid_skeleton,
};
use glam::{Quat, Vec3};

use crate::balance::{Stage, over_clip, standing_clip};

#[derive(Debug)]
struct Fallen {
    hands_down: [Option<usize>; 2],
    chest_down: Option<usize>,
    head_peak: f32,
    entries: Vec<Name>,
    limp: bool,
    resting: bool,
    downs: usize,
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

fn falling_graph(server: &AssetServer) -> AnimationGraph {
    let clip = server.add(standing_clip(&humanoid_skeleton()));
    let stand = server.add(over_clip(
        BalanceNode::default()
            .with_settings(BalanceSettings::default().with_settle_seconds(f32::MAX)),
        &clip,
    ));
    let fall = server.add(over_clip(
        FallNode::default().with_strength(name!("strength")),
        &clip,
    ));
    let down = server.add(over_clip(LimpNode::default(), &clip));
    AnimationGraph::from_node(
        AnimationStateMachine::from_initial_state(name!("stand"), stand, |to| {
            to.to(
                name!("fall"),
                AnimationFSMTrigger::on_message::<Toppled>(),
                0.0,
            );
        })
        .state(name!("fall"), fall, |to| {
            to.to(
                name!("down"),
                AnimationFSMTrigger::on_message::<BodyDown>(),
                0.0,
            );
        })
        .state(name!("down"), down, |_| {})
        .build(),
    )
}

struct Spread {
    bone: &'static str,
    angle: f32,
}

const SPREADS: [Spread; 4] = [
    Spread {
        bone: "upperarm_l",
        angle: -0.9,
    },
    Spread {
        bone: "upperarm_r",
        angle: 0.9,
    },
    Spread {
        bone: "thigh_l",
        angle: 0.35,
    },
    Spread {
        bone: "thigh_r",
        angle: -0.35,
    },
];

fn spread_clip(skeleton: &Skeleton) -> AnimationClip {
    let mut clip = standing_clip(skeleton);
    for spread in SPREADS {
        let index = skeleton.bone_index(spread.bone.into()).unwrap();
        let turn = Quat::from_rotation_z(spread.angle).to_array();
        clip.add_channel(
            skeleton.bone_ids[index],
            AnimationChannel::new(
                vec![0.0, 1.0],
                AnimationChanelOutput::from_rotation([turn, turn].into_iter()),
            ),
        );
    }
    clip
}

fn already_falling_graph(server: &AssetServer) -> AnimationGraph {
    let clip = server.add(spread_clip(&humanoid_skeleton()));
    let waiting = server.add(AnimationGraph::from_node(AnimationClipNode::new(
        clip.clone(),
    )));
    let fall = server.add(over_clip(FallNode::default(), &clip));
    let down = server.add(over_clip(LimpNode::default(), &clip));
    AnimationGraph::from_node(
        AnimationStateMachine::from_initial_state(name!("waiting"), waiting, |to| {
            to.to(
                name!("fall"),
                AnimationFSMTrigger::on_bool(name!("falling"), true),
                0.0,
            );
        })
        .state(name!("fall"), fall, |to| {
            to.to(
                name!("down"),
                AnimationFSMTrigger::on_message::<BodyDown>(),
                0.0,
            );
        })
        .state(name!("down"), down, |_| {})
        .build(),
    )
}

fn fallen(strength: f32, push: Vec3) -> Fallen {
    let mut stage = Stage::graphed(falling_graph);
    stage.set_param(name!("strength"), strength);
    stage.run(15);
    stage.push("spine_03", push);
    watch(stage)
}

fn fallen_on_its_limbs() -> Fallen {
    let mut stage = Stage::placed_graphed(
        Transform::from_translation_rotation(
            Vec3::new(0.0, 1.3, 0.0),
            Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
        ),
        already_falling_graph,
    );
    let world = stage.app.main_mut().world_mut();
    for side in [1.0, -1.0] {
        world.spawn((
            Collider::cuboid(0.9, 0.5, 1.5),
            Transform::from_translation(Vec3::new(side * 1.08, 0.5, 1.0)),
        ));
    }
    stage.set_param(name!("falling"), true);
    watch(stage)
}

fn watch(mut stage: Stage) -> Fallen {
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
        entries: stage.entries().to_vec(),
        limp: ["spine_03", "upperarm_l", "thigh_r"]
            .into_iter()
            .all(|bone| stage.motor(bone).torque_limit == 0.0),
        resting: world
            .get_component_for_entity::<BodySense>(stage.character)
            .unwrap()
            .at_rest,
        downs: stage.downs(),
    }
}

#[test]
fn a_falling_body_lands_on_its_hands_spares_its_head_and_ends_down_and_limp() {
    for direction in [Vec3::Z, Vec3::NEG_Z] {
        let limp = fallen(0.0, direction * 270.0);
        let braced = fallen(1.0, direction * 270.0);
        assert!(
            braced.head_peak < limp.head_peak * 0.5,
            "falling towards {direction} the head lands softer braced than limp: {braced:?} against {limp:?}"
        );
        for fall in [&limp, &braced] {
            assert!(
                fall.entries == [name!("stand"), name!("fall"), name!("down")]
                    && fall.limp
                    && fall.resting
                    && fall.downs == 1,
                "a fall towards {direction} says it is down once, then enters the down state once and stays there, limp and at rest: {fall:?}"
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
    let propped = fallen_on_its_limbs();
    assert!(
        propped.entries == [name!("waiting"), name!("fall"), name!("down")]
            && propped.limp
            && propped.downs == 1,
        "a fall that comes to rest on the hands and thighs across a trench, the trunk held off the ground, ends in the down state and limp: {propped:?}"
    );
}
