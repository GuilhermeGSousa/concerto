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
    BodyHit, Dead, DyingNode, LimpNode, RagdollBody, Settled, messages::Death,
    testing::humanoid_skeleton,
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
            to.to(name!("dead"), AnimationFSMTrigger::OnSucceeded, 0.0);
        })
        .state(name!("dead"), dead, |_| {})
        .build(),
    )
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
    let mut stage = Stage::graphed(dying_graph);
    stage.run(5);
    stage.send(Death { seconds: 1.0 });
    stage.run(15);
    assert!(
        stage.in_state(name!("dying")) && !has::<Dead>(&stage),
        "half a second in the body is still dying"
    );
    stage.run(285);
    assert!(has::<Dead>(&stage) && stage.in_state(name!("dead")));
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
        stage.settles() == 2 && has::<Settled>(&stage) && has::<Dead>(&stage),
        "the woken body settles again and stays dead"
    );

    let mut stage = Stage::graphed(dying_graph);
    stage.run(5);
    stage.send(Death { seconds: 0.0 });
    stage.run(3);
    assert!(
        has::<Dead>(&stage) && stage.in_state(name!("dead")),
        "a zero duration kills at once"
    );
}
