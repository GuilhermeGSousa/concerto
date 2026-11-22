use concerto_foundation::transform::GlobalTransform;
use concerto_ragdoll::{
    BodyHit, BodyState, CharacterRagdoll, Die, Dying, RagdollBody, RagdollMode, Settled,
};
use glam::Vec3;

use crate::balance::Stage;

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
    let die = Die {
        character: stage.character,
        seconds: 1.0,
    };
    stage.app.main_mut().world_mut().trigger(die);
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
    let die = Die {
        character: stage.character,
        seconds: 0.0,
    };
    stage.app.main_mut().world_mut().trigger(die);
    stage.run(1);
    assert_eq!(
        (mode(&stage), stage.state()),
        (RagdollMode::Limp, BodyState::Dead),
        "a zero duration kills at once"
    );
}
