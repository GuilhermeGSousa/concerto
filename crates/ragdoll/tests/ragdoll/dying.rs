use concerto_animation::{
    graph::AnimationGraph,
    node::{
        AnimationClipNode,
        state_machine::{AnimationFSMTrigger, AnimationStateMachine},
    },
};
use concerto_foundation::assets::asset_server::AssetServer;
use concerto_foundation::transform::GlobalTransform;
use concerto_name::name;
use concerto_ragdoll::{
    BodyDead, BodyHit, BodyState, CharacterRagdoll, Dead, Death, Die, Dying, DyingNode, LimpNode,
    RagdollBody, RagdollMode, Settled, testing::humanoid_skeleton,
};
use glam::Vec3;

use crate::balance::{Stage, over_clip, standing_clip};

fn dying_graph(server: &AssetServer) -> AnimationGraph {
    let clip = server.add(standing_clip(&humanoid_skeleton()));
    let alive = server.add(AnimationGraph::from_node(AnimationClipNode::new(
        clip.clone(),
    )));
    let dying = server.add(over_clip(DyingNode::default(), &clip));
    let dead = server.add(over_clip(LimpNode::default(), &clip));
    AnimationGraph::from_node(
        AnimationStateMachine::from_initial_state(name!("alive"), alive, |to| {
            to.to(
                name!("dying"),
                AnimationFSMTrigger::on_message::<Death>(),
                0.0,
            );
        })
        .state(name!("dying"), dying, |to| {
            to.to(
                name!("dead"),
                AnimationFSMTrigger::on_message::<BodyDead>(),
                0.0,
            );
        })
        .state(name!("dead"), dead, |to| {
            to.to(
                name!("dying"),
                AnimationFSMTrigger::on_message::<Death>(),
                0.0,
            );
        })
        .build(),
    )
}

fn mode(stage: &Stage) -> RagdollMode {
    stage
        .app
        .main()
        .world()
        .get_component_for_entity::<CharacterRagdoll>(stage.character)
        .unwrap()
        .mode
}

fn has<C: concerto_ecs::Component>(stage: &Stage) -> bool {
    stage
        .app
        .main()
        .world()
        .get_component_for_entity::<C>(stage.character)
        .is_some()
}

fn bones(stage: &Stage) -> Vec<Vec3> {
    let world = stage.app.main().world();
    stage
        .humanoid
        .bones
        .iter()
        .map(|bone| {
            world
                .get_component_for_entity::<GlobalTransform>(*bone)
                .unwrap()
                .translation()
        })
        .collect()
}

fn die(stage: &mut Stage, seconds: f32) {
    let character = stage.character;
    stage
        .app
        .main_mut()
        .world_mut()
        .trigger(Die { character, seconds });
}

fn hit(stage: &mut Stage, bone: &str, impulse: Vec3) {
    let part = stage
        .app
        .main()
        .world()
        .get_component_for_entity::<RagdollBody>(stage.character)
        .unwrap()
        .resolved()
        .part_of(bone)
        .unwrap();
    let point = stage.part(bone);
    stage
        .app
        .main_mut()
        .world_mut()
        .trigger(BodyHit::new(stage.character, part, point, impulse));
}

#[test]
fn a_dying_body_drains_settles_into_a_static_pose_and_wakes_only_when_hit_hard() {
    let mut stage = Stage::new();
    die(&mut stage, 1.0);
    stage.run(15);
    assert!(
        has::<Dying>(&stage) && stage.state() != BodyState::Dead,
        "half a second in the body is still dying"
    );
    stage.run(285);
    assert_eq!(stage.state(), BodyState::Dead);
    assert_eq!(mode(&stage), RagdollMode::Limp);
    assert!(!has::<Dying>(&stage));
    assert!(
        stage.settles() == 1 && has::<Settled>(&stage),
        "a dead body at rest settles once"
    );
    assert!(
        stage.part("pelvis").y < 0.5,
        "the body lies on the ground, its pelvis is at {}",
        stage.part("pelvis").y
    );

    let lying = bones(&stage);
    stage.push("pelvis", Vec3::new(0.0, 200.0, 0.0));
    hit(&mut stage, "pelvis", Vec3::new(0.0, 20.0, 0.0));
    stage.run(30);
    assert_eq!(
        bones(&stage),
        lying,
        "a settled body is static: a push and a light hit leave every bone where it lay"
    );
    assert_eq!(stage.settles(), 1);

    let rested = stage.part("pelvis").y;
    hit(&mut stage, "pelvis", Vec3::new(0.0, 300.0, 0.0));
    stage.run(5);
    assert!(
        !has::<Settled>(&stage) && stage.part("pelvis").y > rested + 0.05,
        "a hard hit wakes the body and throws it"
    );
    stage.run(300);
    assert!(
        stage.settles() == 2 && has::<Settled>(&stage) && stage.state() == BodyState::Dead,
        "the woken body settles again and stays dead"
    );

    let mut stage = Stage::new();
    die(&mut stage, 0.0);
    stage.run(1);
    assert_eq!(
        (mode(&stage), stage.state()),
        (RagdollMode::Limp, BodyState::Dead),
        "a zero duration kills at once"
    );
    let mut stage = Stage::graphed(dying_graph);
    stage.run(5);
    die(&mut stage, 1.0);
    stage.run(15);
    assert!(
        stage.in_state(name!("dying")) && !has::<Dead>(&stage),
        "in the graph mode, half a second in the body is still dying"
    );
    stage.run(285);
    assert!(
        stage.deaths() == 1 && has::<Dead>(&stage) && stage.in_state(name!("dead")),
        "a drained body raises BodyDead once and enters dead"
    );
    assert!(
        stage.settles() == 1 && has::<Settled>(&stage),
        "a dead body at rest settles once in the graph mode"
    );
    assert!(
        stage.part("pelvis").y < 0.5,
        "the graph's dying body lies on the ground, its pelvis is at {}",
        stage.part("pelvis").y
    );

    let lying = bones(&stage);
    die(&mut stage, 1.0);
    stage.run(30);
    assert!(
        stage.deaths() == 1
            && stage.entries() == [name!("alive"), name!("dying"), name!("dead")]
            && bones(&stage) == lying,
        "a Die sent to a settled body neither kills it again nor drains it again"
    );

    let rested = stage.part("pelvis").y;
    hit(&mut stage, "pelvis", Vec3::new(0.0, 300.0, 0.0));
    stage.run(5);
    assert!(
        !has::<Settled>(&stage) && stage.part("pelvis").y > rested + 0.05,
        "a hard hit wakes the graph's dead body and throws it"
    );
    stage.run(300);
    assert!(
        stage.settles() == 2 && has::<Settled>(&stage) && has::<Dead>(&stage),
        "the woken body settles again and stays dead"
    );
    assert_eq!(
        stage.entries(),
        [name!("alive"), name!("dying"), name!("dead")],
        "the machine enters the dead state once and stays there"
    );

    let mut stage = Stage::graphed(dying_graph);
    stage.run(5);
    die(&mut stage, 0.0);
    stage.run(3);
    assert!(
        has::<Dead>(&stage) && stage.entries() == [name!("alive"), name!("dying"), name!("dead")],
        "a zero duration kills a graph's body at once: {:?}",
        stage.entries()
    );
}
