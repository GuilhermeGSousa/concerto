use concerto_animation::{
    clip::AnimationClip,
    graph::AnimationGraph,
    node::{AnimationClipNode, AnimationNode},
    player::AnimationHandleComponent,
    plugin::AnimationPlugin,
};
use concerto_app::{
    App,
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
    schedule_groups::{FixedUpdate, LateFixedUpdate, LateUpdate, Update},
};
use concerto_ecs::entity::Entity;
use concerto_ecs::resource::Resource;
use concerto_foundation::assets::{asset_server::AssetServer, handle::AssetHandle};
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
use concerto_ragdoll::{CharacterRagdoll, HoldNode, LimpNode, RagdollBody, RagdollPlugin};
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

fn at_rest(server: &AssetServer) -> AssetHandle<AnimationGraph> {
    let clip = server.add(AnimationClip::default());
    server.add(AnimationGraph::from_node(AnimationClipNode::new(clip)))
}

fn at_rest_over(server: &AssetServer, node: impl AnimationNode) -> AssetHandle<AnimationGraph> {
    let clip = server.add(AnimationClip::default());
    let mut graph = AnimationGraph::new();
    graph
        .result_node()
        .input(node)
        .input(AnimationClipNode::new(clip));
    server.add(graph)
}

fn play(app: &mut App, humanoid: &SpawnedHumanoid, graph: &AssetHandle<AnimationGraph>) {
    app.main_mut().world_mut().insert(
        AnimationHandleComponent::new(graph.clone()),
        humanoid.skeleton,
    );
}

#[test]
fn a_walking_character_gone_limp_keeps_its_speed_falls_to_rest_and_is_drawn_by_its_graphs_amount() {
    let mut app = App::new();
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(TimePlugin)
        .register_plugin(AssetManagerPlugin)
        .register_plugin(TransformPlugin)
        .register_plugin(PhysicsPlugin)
        .register_plugin(AnimationPlugin)
        .register_plugin(RagdollPlugin);
    app.finish_plugin_build();
    let server = app.get_resource::<AssetServer>().unwrap().clone();
    let reference = reference_humanoid();
    let skeleton = server.add(reference.skeleton);
    let mut asset = reference.ragdoll;
    asset.skeleton = skeleton.clone();
    let asset = server.add(asset);
    let plain = at_rest(&server);
    let limp = at_rest_over(&server, LimpNode::default());
    app.update();
    let world = app.main_mut().world_mut();
    world.spawn((
        Collider::cuboid(50.0, 0.5, 50.0),
        Transform::from_translation(Vec3::new(0.0, -0.5, 0.0)),
    ));
    let humanoid = spawn_humanoid(world, skeleton, Transform::default());
    world.insert(
        (
            CharacterRagdoll::new(asset),
            AnimationHandleComponent::new(plain.clone()),
        ),
        humanoid.skeleton,
    );

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

    play(&mut app, &humanoid, &limp);
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

    pose_at_rest(&mut app, &humanoid, root + Vec3::new(0.3, 0.0, 0.0));
    frame(&mut app);
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
        let off = simulated.translation.distance(bone.translation);
        assert!(
            off < 1e-3,
            "in the frame its character moves, the bone of part {part} ({:?}) is drawn {off} m from its part",
            body.bone_name(part)
        );
    }

    play(&mut app, &humanoid, &plain);
    for _ in 0..3 {
        pose_at_rest(&mut app, &humanoid, root);
        frame(&mut app);
    }
    let shown = position(&app, humanoid.bone("pelvis"));
    assert!(
        shown.distance(root + Vec3::new(0.0, 1.0, 0.0)) < 1e-4,
        "with no amount in the graph the bones show the animation, the pelvis was at {shown}"
    );
}

#[test]
fn a_limp_characters_capsule_is_carried_under_its_body_and_moves_again_when_let_go() {
    use concerto_animation::plugin::AnimationPlugin;
    use concerto_motion_matching::plugin::MotionMatchingPlugin;
    use concerto_motion_matching::simulation::{CharacterSimulation, DesiredMotion};
    use concerto_physics::carried::Carried;
    use concerto_physics::collider::ColliderOffset;

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
    let plain = at_rest(&server);
    let limp = at_rest_over(&server, LimpNode::default());
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
        (
            CharacterRagdoll::new(asset).with_capsule(capsule),
            AnimationHandleComponent::new(plain.clone()),
        ),
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

    play(&mut app, &humanoid, &limp);
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

    play(&mut app, &humanoid, &plain);
    walk(&mut app, 60);
    let resumed = stands(&app);
    assert!(
        !carried(&app),
        "a capsule is let go when no part has an amount"
    );
    assert!(
        resumed.x - held.x > 1.0 && (resumed.z - held.z).abs() < 0.2,
        "the capsule walks again from where the body lay at {held}, it got to {resumed}"
    );
}

#[derive(Resource, Default)]
struct Lost(u32);

fn count_lost() -> concerto_ecs::signal::listener::Listener<concerto_ragdoll::PoseLost> {
    use concerto_ecs::signal::listener::IntoListener;
    (|_: concerto_ecs::signal::On<concerto_ragdoll::PoseLost>,
      mut lost: concerto_ecs::resource::ResMut<Lost>| {
        lost.0 += 1;
    })
    .into_listener()
}

fn pose_walking(app: &mut App, humanoid: &SpawnedHumanoid, root: Vec3, time: f32) {
    pose_at_rest(app, humanoid, root);
    let world = app.main_mut().world_mut();
    let swing = (time * 4.0).sin() * 0.5;
    for (bone, turn) in [
        ("upperarm_l", Quat::from_rotation_z(swing)),
        ("upperarm_r", Quat::from_rotation_z(swing)),
        ("spine_02", Quat::from_rotation_y(swing * 0.4)),
        ("lowerarm_l", Quat::from_rotation_y(-0.4 - swing * 0.4)),
    ] {
        world
            .get_component_for_entity_mut::<Transform>(humanoid.bone(bone))
            .unwrap()
            .rotation = turn;
    }
}

fn joint_errors(app: &App, humanoid: &SpawnedHumanoid, body: &RagdollBody) -> Vec<f32> {
    let ragdoll = *app
        .main()
        .world()
        .get_component_for_entity::<RagdollId>(humanoid.skeleton)
        .unwrap();
    let mut pose = Vec::new();
    app.get_resource::<PhysicsState>()
        .unwrap()
        .ragdoll_pose(ragdoll, &mut pose);
    let animated = app
        .main()
        .world()
        .get_component_for_entity::<RagdollBody>(humanoid.skeleton)
        .unwrap()
        .animated_pose()
        .to_vec();
    body.resolved()
        .settings()
        .parts
        .iter()
        .enumerate()
        .map(|(part, settings)| match settings.parent {
            Some(parent) => {
                let wanted = animated[parent].rotation.inverse() * animated[part].rotation;
                let held = pose[parent].rotation.inverse() * pose[part].rotation;
                wanted.angle_between(held)
            }
            None => 0.0,
        })
        .collect()
}

#[test]
fn a_powered_body_tracks_its_animation_gives_to_a_push_hangs_when_weak_and_says_when_it_is_lost() {
    use concerto_ragdoll::{BRACED, FLINCH, PartStrength, StrengthProfile};

    let mut app = App::new();
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(TimePlugin)
        .register_plugin(AssetManagerPlugin)
        .register_plugin(TransformPlugin)
        .register_plugin(PhysicsPlugin)
        .register_plugin(AnimationPlugin)
        .register_plugin(RagdollPlugin);
    app.finish_plugin_build();
    app.insert_resource(Lost::default());
    app.main_mut()
        .world_mut()
        .register_component::<concerto_ecs::signal::listener::Listener<concerto_ragdoll::PoseLost>>(
        );
    let server = app.get_resource::<AssetServer>().unwrap().clone();
    let reference = reference_humanoid();
    let skeleton = server.add(reference.skeleton);
    let mut asset = reference.ragdoll;
    asset.skeleton = skeleton.clone();
    let mut weak = asset.profile(FLINCH).unwrap().clone();
    weak.name = "weak".into();
    for bone in [
        "upperarm_l",
        "lowerarm_l",
        "hand_l",
        "upperarm_r",
        "lowerarm_r",
        "hand_r",
    ] {
        weak = weak.with_part(bone, PartStrength::powered(300.0, 0.5));
    }
    let weak = StrengthProfile {
        parts: weak.parts.into_iter().rev().collect(),
        ..weak
    };
    let asset = server.add(asset.with_profile(weak));
    let flinch = at_rest_over(&server, HoldNode::new(FLINCH));
    let weak = at_rest_over(&server, HoldNode::new("weak"));
    let braced = at_rest_over(&server, HoldNode::new(BRACED));
    app.update();
    let world = app.main_mut().world_mut();
    world.spawn((
        Collider::cuboid(50.0, 0.5, 50.0),
        Transform::from_translation(Vec3::new(0.0, -0.5, 0.0)),
    ));
    let humanoid = spawn_humanoid(world, skeleton, Transform::default());
    world.insert(
        (
            CharacterRagdoll::new(asset),
            AnimationHandleComponent::new(flinch),
            count_lost(),
        ),
        humanoid.skeleton,
    );

    let step = 0.033;
    let mut root = Vec3::new(0.0, 0.02, 0.0);
    let mut time = 0.0;
    pose_walking(&mut app, &humanoid, root, time);
    frame(&mut app);
    frame(&mut app);
    let body = app
        .main()
        .world()
        .get_component_for_entity::<RagdollBody>(humanoid.skeleton)
        .unwrap()
        .clone();
    let mut worst = 0.0f32;
    for index in 0..120 {
        time += step;
        root.x += step;
        pose_walking(&mut app, &humanoid, root, time);
        frame(&mut app);
        if index >= 30 {
            let errors = joint_errors(&app, &humanoid, &body);
            worst = errors.into_iter().fold(worst, f32::max);
        }
    }
    assert!(
        worst < 0.15,
        "a powered upper body tracks a swinging clip within 0.15 rad a joint, the worst was {worst}"
    );

    let chest = body.resolved().part_of("spine_03").unwrap();
    let ragdoll = *app
        .main()
        .world()
        .get_component_for_entity::<RagdollId>(humanoid.skeleton)
        .unwrap();
    for _ in 0..30 {
        pose_at_rest(&mut app, &humanoid, root);
        frame(&mut app);
    }
    let torso = |app: &App| {
        let errors = joint_errors(app, &humanoid, &body);
        ["spine_01", "spine_02", "spine_03"]
            .into_iter()
            .map(|bone| errors[body.resolved().part_of(bone).unwrap()])
            .fold(0.0, f32::max)
    };
    let standing = torso(&app);
    let state = app.get_resource_mut::<PhysicsState>().unwrap();
    let chest_body = state.ragdoll_bodies(ragdoll)[chest];
    state.add_impulse(chest_body, Vec3::new(0.0, 0.0, 80.0));
    let mut bent = 0.0f32;
    for _ in 0..30 {
        pose_at_rest(&mut app, &humanoid, root);
        frame(&mut app);
        bent = bent.max(torso(&app));
    }
    let settled = torso(&app);
    assert!(
        bent > standing + 0.05,
        "a push on the chest bends the torso, it bent {bent} rad from {standing}"
    );
    assert!(
        settled < standing + 0.02,
        "the torso is back on its pose a second after the push, it was {settled} rad off against {standing} before"
    );
    assert_eq!(app.get_resource::<Lost>().unwrap().0, 0);

    play(&mut app, &humanoid, &weak);
    for _ in 0..90 {
        pose_at_rest(&mut app, &humanoid, root);
        frame(&mut app);
    }
    let mut pose = Vec::new();
    app.get_resource::<PhysicsState>()
        .unwrap()
        .ragdoll_pose(ragdoll, &mut pose);
    let shoulder = pose[body.resolved().part_of("upperarm_l").unwrap()].translation;
    let hand = pose[body.resolved().part_of("hand_l").unwrap()].translation;
    assert!(
        shoulder.y - hand.y > 0.3,
        "arms with half a newton metre of muscle hang, the hand was at {hand} under a shoulder at {shoulder}"
    );

    play(&mut app, &humanoid, &braced);
    for _ in 0..15 {
        pose_at_rest(&mut app, &humanoid, root);
        frame(&mut app);
    }
    let state = app.get_resource_mut::<PhysicsState>().unwrap();
    let chest_body = state.ragdoll_bodies(ragdoll)[chest];
    state.add_impulse(chest_body, Vec3::new(0.0, 0.0, 60.0));
    for _ in 0..120 {
        pose_at_rest(&mut app, &humanoid, root);
        frame(&mut app);
    }
    assert_eq!(
        app.get_resource::<Lost>().unwrap().0,
        1,
        "a body on its own feet that is pushed over says once that it lost its pose"
    );
}

#[test]
fn a_body_senses_its_support_its_balance_its_fall_and_its_rest() {
    use concerto_ragdoll::{BRACED, BodySense, FLINCH};

    let mut app = App::new();
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(TimePlugin)
        .register_plugin(AssetManagerPlugin)
        .register_plugin(TransformPlugin)
        .register_plugin(PhysicsPlugin)
        .register_plugin(AnimationPlugin)
        .register_plugin(RagdollPlugin);
    app.finish_plugin_build();
    let server = app.get_resource::<AssetServer>().unwrap().clone();
    let reference = reference_humanoid();
    let skeleton = server.add(reference.skeleton);
    let mut asset = reference.ragdoll;
    asset.skeleton = skeleton.clone();
    let asset = server.add(asset);
    let flinch = at_rest_over(&server, HoldNode::new(FLINCH));
    let braced = at_rest_over(&server, HoldNode::new(BRACED));
    let limp = at_rest_over(&server, LimpNode::default());
    app.update();
    let world = app.main_mut().world_mut();
    world.spawn((
        Collider::cuboid(50.0, 0.5, 50.0),
        Transform::from_translation(Vec3::new(0.0, -0.5, 0.0)),
    ));
    let humanoid = spawn_humanoid(world, skeleton, Transform::default());
    world.insert(
        (
            CharacterRagdoll::new(asset),
            AnimationHandleComponent::new(flinch),
        ),
        humanoid.skeleton,
    );
    let root = Vec3::new(0.0, 0.05, 0.0);
    let stand = |app: &mut App, frames: usize| {
        for _ in 0..frames {
            pose_at_rest(app, &humanoid, root);
            frame(app);
        }
    };
    let sense = |app: &App| {
        app.main()
            .world()
            .get_component_for_entity::<BodySense>(humanoid.skeleton)
            .unwrap()
            .clone()
    };
    let push = |app: &mut App, impulse: Vec3| {
        let world = app.main().world();
        let ragdoll = *world
            .get_component_for_entity::<RagdollId>(humanoid.skeleton)
            .unwrap();
        let chest = world
            .get_component_for_entity::<RagdollBody>(humanoid.skeleton)
            .unwrap()
            .resolved()
            .landmarks()
            .chest
            .unwrap();
        let state = app.get_resource_mut::<PhysicsState>().unwrap();
        let body = state.ragdoll_bodies(ragdoll)[chest];
        state.add_impulse(body, impulse);
    };

    stand(&mut app, 12);
    let standing = sense(&app);
    assert_eq!(standing.supporting.len(), 2, "both feet are on the ground");
    assert!(standing.support.len() >= 4, "{:?}", standing.support);
    assert!(
        standing.balanced(),
        "off balance by {}",
        standing.off_balance
    );
    assert!((standing.centre.mass - 75.0).abs() < 1e-3);
    assert!(
        (standing.centre.position.y - 1.0).abs() < 0.15 && standing.centre.position.x.abs() < 0.05,
        "the centre of a standing body is near its pelvis, it was at {}",
        standing.centre.position
    );
    assert!(standing.ground.abs() < 0.02 && standing.lean < 0.1 && standing.airborne == 0.0);
    assert_eq!(standing.chest_impact, None);

    push(&mut app, Vec3::new(0.0, 0.0, 4.0));
    for _ in 0..4 {
        stand(&mut app, 1);
        let light = sense(&app);
        assert!(
            light.balanced(),
            "a light push leaves the capture point inside the feet: {}",
            light.off_balance
        );
    }

    push(&mut app, Vec3::new(0.0, 0.0, 150.0));
    stand(&mut app, 2);
    let shoved = sense(&app);
    assert!(
        shoved.off_balance.z > 0.05 && shoved.off_balance.x.abs() < shoved.off_balance.z,
        "a hard push sends the capture point out on the side of the push: {}",
        shoved.off_balance
    );
    assert!(!shoved.at_rest);

    stand(&mut app, 60);
    play(&mut app, &humanoid, &braced);
    stand(&mut app, 6);
    assert_eq!(
        sense(&app).supporting.len(),
        2,
        "feet that are simulated are on the ground by their contacts"
    );
    push(&mut app, Vec3::new(0.0, 0.0, 150.0));
    stand(&mut app, 300);
    let fallen = sense(&app);
    assert!(fallen.at_rest, "a fallen body comes to rest");
    assert!(fallen.centre.position.y < 0.5 && fallen.lean > 0.5);

    let ragdoll = *app
        .main()
        .world()
        .get_component_for_entity::<RagdollId>(humanoid.skeleton)
        .unwrap();
    play(&mut app, &humanoid, &limp);
    stand(&mut app, 1);
    let state = app.get_resource_mut::<PhysicsState>().unwrap();
    let mut pose = Vec::new();
    state.ragdoll_pose(ragdoll, &mut pose);
    for part in &mut pose {
        part.translation.y += 3.0;
    }
    state.set_ragdoll_pose(ragdoll, &pose);
    stand(&mut app, 3);
    let dropped = sense(&app);
    assert!(dropped.supporting.is_empty() && dropped.airborne > 0.05);
    let impact = dropped
        .chest_impact
        .expect("a falling chest is headed for the ground");
    assert!(
        impact.point.y.abs() < 0.3 && impact.seconds > 0.4 && impact.seconds < 1.0,
        "the chest, about three metres up, lands in under a second: {impact:?}"
    );
    assert!(dropped.head_impact.is_some());
}

#[test]
fn a_drive_input_that_reads_what_the_graph_drive_writes_panics_when_the_drive_is_built() {
    use concerto_animation::{node::DriveNodeInputs, player::AnimationPlayer};
    use concerto_ecs::Query;

    let mut app = App::new();
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(TimePlugin)
        .register_plugin(AssetManagerPlugin)
        .register_plugin(TransformPlugin)
        .register_plugin(PhysicsPlugin)
        .register_plugin(RagdollPlugin);
    app.register_input::<DriveNodeInputs, Query<'static, 'static, &'static AnimationPlayer>>();

    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        app.finish_plugin_build();
        app.update();
    }));
    let panic = built.expect_err("a drive input that reads the player conflicts with the drive");
    let message = panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|text| text.to_string()))
        .unwrap_or_default();
    assert!(
        message.contains("AnimationPlayer"),
        "the panic names the conflicting component: {message}"
    );
}
