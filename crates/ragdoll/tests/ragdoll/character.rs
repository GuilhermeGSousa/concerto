use concerto_app::{
    App,
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
    schedule_groups::{FixedUpdate, LateFixedUpdate, LateUpdate, Update},
};
use concerto_ecs::entity::Entity;
use concerto_foundation::assets::asset_server::AssetServer;
use concerto_foundation::transform::{GlobalTransform, Transform};
use concerto_physics::body::BodyId;
use concerto_physics::collider::Collider;
use concerto_physics::physics_state::PhysicsState;
use concerto_physics::plugin::PhysicsPlugin;
use concerto_physics::query::{QueryFilter, RagdollQuery};
use concerto_physics::ragdoll::{BodyPose, RagdollId};
use concerto_physics::rigid_body::{AllowedDofs, RigidBody};
use concerto_ragdoll::testing::{SpawnedHumanoid, reference_humanoid, spawn_humanoid};
use concerto_ragdoll::{CharacterRagdoll, RagdollBody, RagdollPlugin};
use glam::{Quat, Vec3};

fn frame(app: &mut App) {
    app.serve(FixedUpdate);
    app.serve(LateFixedUpdate);
    app.serve(Update);
    app.serve(LateUpdate);
}

fn bone_poses(app: &App, bones: &[Entity]) -> Vec<BodyPose> {
    bones
        .iter()
        .map(|bone| {
            let transform = app
                .main()
                .world()
                .get_component_for_entity::<GlobalTransform>(*bone)
                .unwrap();
            BodyPose::new(transform.translation(), transform.rotation())
        })
        .collect()
}

fn position(app: &App, entity: Entity) -> Vec3 {
    app.main()
        .world()
        .get_component_for_entity::<GlobalTransform>(entity)
        .unwrap()
        .translation()
}

fn animate(app: &mut App, humanoid: &SpawnedHumanoid, step: usize) {
    let world = app.main_mut().world_mut();
    let swing = (step as f32 * 0.2).sin() * 0.8;
    world
        .get_component_for_entity_mut::<Transform>(humanoid.bone("upperarm_l"))
        .unwrap()
        .rotation = Quat::from_rotation_z(swing);
    world
        .get_component_for_entity_mut::<Transform>(humanoid.skeleton)
        .unwrap()
        .translation
        .x += 0.02;
}

#[test]
fn a_characters_ragdoll_follows_its_bones_names_the_bone_a_ray_hits_and_leaves_with_it() {
    let mut app = App::new();
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(TimePlugin)
        .register_plugin(AssetManagerPlugin)
        .register_plugin(TransformPlugin)
        .register_plugin(PhysicsPlugin)
        .register_plugin(RagdollPlugin);
    app.finish_plugin_build();
    let server = app.get_resource::<AssetServer>().unwrap().clone();
    let reference = reference_humanoid();
    let skeleton = server.add(reference.skeleton);
    let mut asset = reference.ragdoll;
    asset.skeleton = skeleton.clone();
    let asset = server.add(asset);
    app.update();

    let world = app.main_mut().world_mut();
    let floor = world.spawn((
        Collider::cuboid(20.0, 0.5, 20.0),
        Transform::from_translation(Vec3::new(0.0, -0.5, 0.0)),
    ));
    let capsule_start = Vec3::new(0.0, 0.86, 0.0);
    let capsule = world.spawn((
        RigidBody {
            allowed_dofs: AllowedDofs::TRANSLATION,
            ..RigidBody::default()
        },
        Collider::capsule(0.5, 0.35),
        Transform::from_translation(capsule_start),
    ));
    let humanoid = spawn_humanoid(world, skeleton, Transform::default());
    world.insert(
        CharacterRagdoll::new(asset).with_capsule(capsule),
        humanoid.skeleton,
    );
    frame(&mut app);
    frame(&mut app);
    let world = app.main().world();
    let ragdoll = *world
        .get_component_for_entity::<RagdollId>(humanoid.skeleton)
        .expect("the character's ragdoll is built once its assets have loaded");
    let body = world
        .get_component_for_entity::<RagdollBody>(humanoid.skeleton)
        .unwrap()
        .clone();

    let mut pose = Vec::new();
    for step in 0..60 {
        animate(&mut app, &humanoid, step);
        let drawn = bone_poses(&app, body.bones());
        frame(&mut app);
        app.get_resource::<PhysicsState>()
            .unwrap()
            .ragdoll_pose(ragdoll, &mut pose);
        for (part, (simulated, bone)) in pose.iter().zip(&drawn).enumerate() {
            let off = simulated.translation.distance(bone.translation);
            assert!(
                off < 2e-3 && simulated.rotation.dot(bone.rotation).abs() > 0.9999,
                "step {step}: part {part} ({:?}) is {off} m from the pose its bone was drawn in",
                body.bone_name(part)
            );
        }
    }
    frame(&mut app);

    let world = app.main().world();
    let state = app.get_resource::<PhysicsState>().unwrap();
    let capsule_body = *world.get_component_for_entity::<BodyId>(capsule).unwrap();
    let pushed = state
        .body_transform(capsule_body)
        .translation
        .distance(capsule_start);
    assert!(
        pushed < 0.02,
        "the character's parts should pass through its own capsule, they pushed it {pushed} m"
    );

    let forearm = (position(&app, humanoid.bone("lowerarm_l"))
        + position(&app, humanoid.bone("hand_l")))
        / 2.0;
    let shot = state
        .cast_ray(
            forearm + Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(0.0, 0.0, -2.0),
            QueryFilter::default().with_ragdolls(RagdollQuery::Only),
        )
        .expect("a ray at the forearm should hit a part");
    assert_eq!(shot.entity, Some(humanoid.skeleton));
    let part = shot.part.expect("the hit names the part");
    assert_eq!(body.bone(part), Some(humanoid.bone("lowerarm_l")));
    assert_eq!(body.bone_name(part).as_deref(), Some("lowerarm_l"));

    let foot = position(&app, humanoid.bone("foot_l"));
    let ground = state
        .cast_ray(foot, Vec3::new(0.0, -1.0, 0.0), QueryFilter::default())
        .expect("a ray down from the foot should reach the floor");
    assert_eq!(
        ground.entity,
        Some(floor),
        "a ray that did not ask for ragdolls ignores the character's own parts"
    );

    app.main_mut().world_mut().despawn(humanoid.skeleton);
    let state = app.get_resource::<PhysicsState>().unwrap();
    assert_eq!(state.ragdolls().count(), 0);
    assert!(
        state
            .cast_ray(
                forearm + Vec3::new(0.0, 0.0, 1.0),
                Vec3::new(0.0, 0.0, -2.0),
                QueryFilter::default().with_ragdolls(RagdollQuery::Only),
            )
            .is_none(),
        "despawning the character should leave no bodies behind"
    );
}
