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
use concerto_physics::layers::{CollisionLayers, Layers};
use concerto_physics::physics_state::PhysicsState;
use concerto_physics::plugin::PhysicsPlugin;
use concerto_physics::query::QueryFilter;
use concerto_physics::ragdoll::{BodyPose, RagdollId};
use concerto_physics::rigid_body::{AllowedDofs, RigidBody};
use concerto_ragdoll::testing::{
    SpawnedHumanoid, humanoid_skeleton, reference_humanoid, spawn_humanoid,
};
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
        CollisionLayers::character(),
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
        "ragdoll parts should pass through a capsule on the character layer, they pushed it {pushed} m"
    );

    let forearm = (position(&app, humanoid.bone("lowerarm_l"))
        + position(&app, humanoid.bone("hand_l")))
        / 2.0;
    let shot = state
        .cast_ray(
            forearm + Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(0.0, 0.0, -2.0),
            QueryFilter::default().with_layers(Layers::RAGDOLL),
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
                QueryFilter::default().with_layers(Layers::RAGDOLL),
            )
            .is_none(),
        "despawning the character should leave no bodies behind"
    );
}

fn pose_at_rest(app: &mut App, humanoid: &SpawnedHumanoid, root: Vec3) {
    let rest = humanoid_skeleton();
    let world = app.main_mut().world_mut();
    for (bone, local) in humanoid.bones.iter().zip(rest.rest_pose.iter()) {
        let transform = world
            .get_component_for_entity_mut::<Transform>(*bone)
            .unwrap();
        transform.translation = local.translation;
        transform.rotation = local.rotation;
    }
    world
        .get_component_for_entity_mut::<Transform>(humanoid.skeleton)
        .unwrap()
        .translation = root;
}

fn character(app: &mut App, humanoid: &SpawnedHumanoid) -> CharacterRagdoll {
    app.main()
        .world()
        .get_component_for_entity::<CharacterRagdoll>(humanoid.skeleton)
        .unwrap()
        .clone()
}

fn edit(app: &mut App, humanoid: &SpawnedHumanoid, change: impl FnOnce(&mut CharacterRagdoll)) {
    change(
        app.main_mut()
            .world_mut()
            .get_component_for_entity_mut::<CharacterRagdoll>(humanoid.skeleton)
            .unwrap(),
    );
}

#[test]
fn a_walking_character_gone_limp_keeps_its_speed_falls_to_rest_and_is_drawn_by_weight() {
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
    world.spawn((
        Collider::cuboid(50.0, 0.5, 50.0),
        Transform::from_translation(Vec3::new(0.0, -0.5, 0.0)),
    ));
    let humanoid = spawn_humanoid(world, skeleton, Transform::default());
    world.insert(CharacterRagdoll::new(asset), humanoid.skeleton);

    let mut root = Vec3::new(0.0, 0.02, 0.0);
    for _ in 0..20 {
        root.x += 0.05;
        pose_at_rest(&mut app, &humanoid, root);
        frame(&mut app);
    }
    let world = app.main().world();
    let ragdoll = *world
        .get_component_for_entity::<RagdollId>(humanoid.skeleton)
        .unwrap();
    let body = world
        .get_component_for_entity::<RagdollBody>(humanoid.skeleton)
        .unwrap()
        .clone();
    let pelvis = body.resolved().part_of("pelvis").unwrap();
    let mut pose = Vec::new();
    let state = app.get_resource::<PhysicsState>().unwrap();
    state.ragdoll_pose(ragdoll, &mut pose);
    let let_go_at = pose[pelvis].translation.x;

    edit(&mut app, &humanoid, CharacterRagdoll::go_limp);
    for _ in 0..10 {
        pose_at_rest(&mut app, &humanoid, root);
        frame(&mut app);
    }
    let state = app.get_resource::<PhysicsState>().unwrap();
    state.ragdoll_pose(ragdoll, &mut pose);
    let carried = pose[pelvis].translation.x - let_go_at;
    assert!(
        carried > 0.3,
        "a body let go at 1.5 m/s keeps travelling, it moved {carried} m in a third of a second"
    );

    for _ in 0..360 {
        pose_at_rest(&mut app, &humanoid, root);
        frame(&mut app);
    }
    let state = app.get_resource::<PhysicsState>().unwrap();
    state.ragdoll_pose(ragdoll, &mut pose);
    let drawn = bone_poses(&app, body.bones());
    let stepped = app
        .main()
        .world()
        .get_component_for_entity::<RagdollBody>(humanoid.skeleton)
        .unwrap()
        .interpolated_pose(
            app.get_resource::<concerto_foundation::time::Time>()
                .unwrap()
                .fixed_alpha(),
        );
    for (part, (simulated, bone)) in stepped.iter().zip(&drawn).enumerate() {
        let speed = state
            .linear_velocity(state.ragdoll_bodies(ragdoll)[part])
            .length();
        assert!(
            speed < 0.1,
            "part {part} should be at rest, moved at {speed} m/s"
        );
        assert!(
            simulated.translation.y < 0.6,
            "part {part} should lie on the floor, was at y = {}",
            simulated.translation.y
        );
        let off = simulated.translation.distance(bone.translation);
        assert!(
            off < 1e-3 && simulated.rotation.dot(bone.rotation).abs() > 0.9999,
            "the bone of part {part} ({:?}) is drawn {off} m from its part",
            body.bone_name(part)
        );
    }
    let finger =
        position(&app, humanoid.bone("index_l")).distance(position(&app, humanoid.bone("hand_l")));
    assert!(
        (finger - 0.08).abs() < 1e-3,
        "a bone with no part keeps its animated pose under its parent, the finger was {finger} m from the hand"
    );

    let fallen = pose[pelvis].translation;
    let animated = root + Vec3::new(0.0, 1.0, 0.0);
    edit(&mut app, &humanoid, |character| character.weight = 0.0);
    pose_at_rest(&mut app, &humanoid, root);
    frame(&mut app);
    let shown = position(&app, humanoid.bone("pelvis"));
    assert!(
        shown.distance(animated) < 1e-4,
        "at weight 0 the bones show the animation, the pelvis was at {shown}"
    );

    edit(&mut app, &humanoid, |character| {
        character.weight = 0.5;
        character.part_weights = CharacterRagdoll::default()
            .with_part_weight("Head", 1.0)
            .part_weights;
    });
    pose_at_rest(&mut app, &humanoid, root);
    frame(&mut app);
    let shown = position(&app, humanoid.bone("pelvis"));
    assert!(
        shown.distance((animated + fallen) / 2.0) < 1e-3,
        "at weight 0.5 the pelvis is halfway between {animated} and {fallen}, it was at {shown}"
    );
    let head = body.resolved().part_of("Head").unwrap();
    let shown = position(&app, humanoid.bone("Head"));
    assert!(
        shown.distance(pose[head].translation) < 1e-3,
        "a part with its own weight of 1 is drawn on its body"
    );
    assert_eq!(character(&mut app, &humanoid).weight_of("Head"), 1.0);
}

#[test]
fn a_limp_characters_capsule_is_carried_under_its_body_and_moves_again_when_let_go() {
    use concerto_animation::plugin::AnimationPlugin;
    use concerto_motion_matching::plugin::MotionMatchingPlugin;
    use concerto_motion_matching::simulation::{CharacterSimulation, DesiredMotion};
    use concerto_physics::carried::Carried;
    use concerto_physics::collider::ColliderOffset;
    use concerto_ragdoll::RagdollMode;

    let mut app = App::new();
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(TimePlugin)
        .register_plugin(AssetManagerPlugin)
        .register_plugin(TransformPlugin)
        .register_plugin(PhysicsPlugin)
        .register_plugin(MotionMatchingPlugin)
        .register_plugin(AnimationPlugin)
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
    world.spawn((
        Collider::cuboid(50.0, 0.5, 50.0),
        Transform::from_translation(Vec3::new(0.0, -0.5, 0.0)),
    ));
    let shape = Collider::capsule(0.5, 0.3);
    let offset = ColliderOffset::bottom_origin(&shape);
    let capsule = world.spawn((
        RigidBody {
            allowed_dofs: AllowedDofs::TRANSLATION,
            ..RigidBody::default()
        },
        shape,
        offset,
        CollisionLayers::character(),
        Transform::from_translation(Vec3::new(0.0, 0.02, 0.0)),
        CharacterSimulation::default(),
        DesiredMotion {
            velocity: Vec3::new(1.5, 0.0, 0.0),
            rotation: Quat::IDENTITY,
        },
    ));
    let humanoid = spawn_humanoid(world, skeleton, Transform::default());
    world.insert(
        CharacterRagdoll::new(asset).with_capsule(capsule),
        humanoid.skeleton,
    );
    let stands = |app: &App| {
        let world = app.main().world();
        let body = *world.get_component_for_entity::<BodyId>(capsule).unwrap();
        app.get_resource::<PhysicsState>()
            .unwrap()
            .body_transform(body)
            .translation
    };
    let carried = |app: &App| {
        app.main()
            .world()
            .get_component_for_entity::<Carried>(capsule)
            .is_some()
    };
    let walk = |app: &mut App, frames: usize| {
        for _ in 0..frames {
            frame(app);
            let root = stands(app);
            pose_at_rest(app, &humanoid, root);
        }
    };

    walk(&mut app, 60);
    let walked = stands(&app);
    assert!(
        walked.x > 1.0,
        "the capsule walks under its input, it got to {walked}"
    );
    assert!(!carried(&app));

    edit(&mut app, &humanoid, CharacterRagdoll::go_limp);
    walk(&mut app, 300);
    let ragdoll = *app
        .main()
        .world()
        .get_component_for_entity::<RagdollId>(humanoid.skeleton)
        .unwrap();
    let mut pose = Vec::new();
    app.get_resource::<PhysicsState>()
        .unwrap()
        .ragdoll_pose(ragdoll, &mut pose);
    let pelvis = pose[0].translation;
    let fallen = stands(&app);
    assert!(carried(&app), "the capsule of a limp character is carried");
    assert!(
        fallen.with_y(0.0).distance(pelvis.with_y(0.0)) < 0.05
            && (fallen.y - walked.y).abs() < 0.05,
        "the capsule stands on the ground under the fallen pelvis at {pelvis}, it was at {fallen}"
    );
    walk(&mut app, 30);
    let held = stands(&app);
    assert!(
        held.distance(fallen) < 0.01,
        "held input does not move a carried capsule, it went from {fallen} to {held}"
    );

    edit(&mut app, &humanoid, |character| {
        character.mode = RagdollMode::FollowsAnimation;
        character.weight = 0.0;
    });
    walk(&mut app, 60);
    let resumed = stands(&app);
    assert!(
        !carried(&app),
        "a capsule is let go when no part has weight"
    );
    assert!(
        resumed.x - held.x > 1.0 && (resumed.z - held.z).abs() < 0.2,
        "the capsule walks again from where the body lay at {held}, it got to {resumed}"
    );
}
