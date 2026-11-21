use concerto_app::{
    App,
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
    schedule_groups::{FixedUpdate, LateFixedUpdate, LateUpdate, Update},
};
use concerto_ecs::resource::{ResMut, Resource};
use concerto_ecs::signal::On;
use concerto_ecs::{Component, IntoSystemConfig, Query};
use concerto_foundation::assets::asset_server::AssetServer;
use concerto_foundation::transform::Transform;
use concerto_physics::collider::Collider;
use concerto_physics::physics_state::PhysicsState;
use concerto_physics::plugin::PhysicsPlugin;
use concerto_physics::ragdoll::RagdollId;
use concerto_ragdoll::testing::{humanoid_skeleton, reference_humanoid, spawn_humanoid};
use concerto_ragdoll::{
    BehaviourSet, BodyDrive, BodySense, BodyState, BodyStateChanged, CharacterRagdoll, Limbs,
    PartCommand, PartStrength, PhysicalState, RagdollBody, RagdollPlugin, describe_body,
};
use glam::{Quat, Vec3};

#[derive(Component)]
struct RaiseArms;

#[derive(Component)]
struct LowerLeftArm;

fn muscle() -> PartStrength {
    PartStrength::powered(500.0, 120.0)
}

fn raise_arms(characters: Query<(&RaiseArms, &RagdollBody, &mut BodyDrive)>) {
    for (_, body, mut drive) in characters.iter() {
        let landmarks = body.resolved().landmarks();
        for raised in [
            Raised {
                limbs: landmarks.left,
                turn: 1.0,
            },
            Raised {
                limbs: landmarks.right,
                turn: -1.0,
            },
        ] {
            let Limbs {
                upper_arm: Some(upper),
                lower_arm: Some(lower),
                hand: Some(hand),
                ..
            } = raised.limbs
            else {
                continue;
            };
            drive.claim(
                "raise_arms",
                upper,
                1,
                1.0,
                PartCommand::held(muscle()).with_target(Quat::from_rotation_z(raised.turn)),
            );
            drive.claim("raise_arms", lower, 1, 1.0, PartCommand::held(muscle()));
            drive.claim("raise_arms", hand, 1, 1.0, PartCommand::held(muscle()));
        }
    }
}

struct Raised {
    limbs: Limbs,
    turn: f32,
}

fn lower_left_arm(characters: Query<(&LowerLeftArm, &RagdollBody, &mut BodyDrive)>) {
    for (_, body, mut drive) in characters.iter() {
        if let Some(arm) = body.resolved().landmarks().left.upper_arm {
            drive.claim(
                "lower_left_arm",
                arm,
                5,
                1.0,
                PartCommand::held(muscle()).with_target(Quat::from_rotation_z(-1.0)),
            );
        }
    }
}

#[derive(Resource, Default)]
struct Changes(Vec<BodyStateChanged>);

fn record(on: On<BodyStateChanged>, mut changes: ResMut<Changes>) {
    changes.0.push(*on.signal());
}

fn frame(app: &mut App) {
    app.serve(FixedUpdate);
    app.serve(LateFixedUpdate);
    app.serve(Update);
    app.serve(LateUpdate);
}

#[test]
fn behaviours_share_a_body_by_priority_and_the_character_says_what_state_it_is_in() {
    let mut app = App::new();
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(TimePlugin)
        .register_plugin(AssetManagerPlugin)
        .register_plugin(TransformPlugin)
        .register_plugin(PhysicsPlugin)
        .register_plugin(RagdollPlugin);
    app.add_system(FixedUpdate, raise_arms.in_set(BehaviourSet))
        .add_system(FixedUpdate, lower_left_arm.in_set(BehaviourSet))
        .add_listener(record)
        .insert_resource(Changes::default());
    app.finish_plugin_build();
    let server = app.get_resource::<AssetServer>().unwrap().clone();
    let reference = reference_humanoid();
    let skeleton = server.add(reference.skeleton);
    let mut asset = reference.ragdoll;
    asset.skeleton = skeleton.clone();
    let asset = server.add(asset);
    app.update();
    let world = app.main_mut().world_mut();
    world.spawn((
        Collider::cuboid(50.0, 0.5, 50.0),
        Transform::from_translation(Vec3::new(0.0, -0.5, 0.0)),
    ));
    let humanoid = spawn_humanoid(world, skeleton, Transform::from_translation(Vec3::Y * 0.05));
    let character = humanoid.skeleton;
    world.insert(
        (
            CharacterRagdoll::new(asset),
            BodyDrive::default(),
            BodySense::default(),
            PhysicalState::default(),
        ),
        character,
    );
    let rest = humanoid_skeleton();
    let bones = humanoid.bones.clone();
    let run = |app: &mut App, frames: usize| {
        for _ in 0..frames {
            let world = app.main_mut().world_mut();
            for (bone, local) in bones.iter().zip(rest.rest_pose.iter()) {
                let transform = world
                    .get_component_for_entity_mut::<Transform>(*bone)
                    .unwrap();
                transform.translation = local.translation;
                transform.rotation = local.rotation;
            }
            frame(app);
        }
    };
    let heights = |app: &App| {
        let world = app.main().world();
        let ragdoll = *world
            .get_component_for_entity::<RagdollId>(character)
            .unwrap();
        let body = world
            .get_component_for_entity::<RagdollBody>(character)
            .unwrap();
        let landmarks = body.resolved().landmarks();
        let mut pose = Vec::new();
        app.get_resource::<PhysicsState>()
            .unwrap()
            .ragdoll_pose(ragdoll, &mut pose);
        let rise = |hand: Option<usize>, arm: Option<usize>| {
            pose[hand.unwrap()].translation.y - pose[arm.unwrap()].translation.y
        };
        Hands {
            left: rise(landmarks.left.hand, landmarks.left.upper_arm),
            right: rise(landmarks.right.hand, landmarks.right.upper_arm),
        }
    };
    let state = |app: &App| {
        app.main()
            .world()
            .get_component_for_entity::<PhysicalState>(character)
            .unwrap()
            .state
    };

    run(&mut app, 5);
    assert_eq!(state(&app), BodyState::Animated);
    app.main_mut()
        .world_mut()
        .insert((RaiseArms, LowerLeftArm), character);
    run(&mut app, 45);
    let hands = heights(&app);
    assert!(
        hands.right > 0.2 && hands.left < -0.2,
        "the right arm rises with the first behaviour and the left follows the second at higher priority: {hands:?}"
    );
    assert_eq!(state(&app), BodyState::Reacting);
    let world = app.main().world();
    let report = describe_body(
        world.get_component_for_entity::<PhysicalState>(character),
        world.get_component_for_entity::<BodyDrive>(character),
        world.get_component_for_entity::<BodySense>(character),
        world
            .get_component_for_entity::<RagdollBody>(character)
            .unwrap(),
    );
    for line in [
        "state: Reacting",
        "behaviour lower_left_arm · priority 5 · upperarm_l",
        "behaviour raise_arms · priority 1 · upperarm_l, lowerarm_l, hand_l, upperarm_r, lowerarm_r, hand_r",
        "support: foot_l, foot_r",
    ] {
        assert!(
            report.contains(line),
            "the report lacks `{line}`:\n{report}"
        );
    }
    let weight = world
        .get_component_for_entity::<CharacterRagdoll>(character)
        .unwrap()
        .weight;
    assert_eq!(weight, 1.0, "a driven body is drawn as it is simulated");

    app.main_mut()
        .world_mut()
        .remove_component::<LowerLeftArm>(character);
    run(&mut app, 45);
    let hands = heights(&app);
    assert!(
        hands.right > 0.2 && hands.left > 0.2,
        "stopping the second behaviour releases the left arm to the first: {hands:?}"
    );

    app.main_mut()
        .world_mut()
        .remove_component::<RaiseArms>(character);
    run(&mut app, 3);
    assert_eq!(state(&app), BodyState::Animated);
    let changes = &app.get_resource::<Changes>().unwrap().0;
    assert_eq!(
        changes,
        &[
            BodyStateChanged {
                character,
                from: BodyState::Animated,
                to: BodyState::Reacting,
            },
            BodyStateChanged {
                character,
                from: BodyState::Reacting,
                to: BodyState::Animated,
            },
        ]
    );
    let weight = app
        .main()
        .world()
        .get_component_for_entity::<CharacterRagdoll>(character)
        .unwrap()
        .weight;
    assert_eq!(weight, 0.0);
}

#[derive(Debug)]
struct Hands {
    left: f32,
    right: f32,
}
