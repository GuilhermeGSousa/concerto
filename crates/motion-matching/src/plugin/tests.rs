use std::time::{Duration, Instant};

use concerto_animation::{
    animate_targets,
    clip::AnimationClip,
    plugin::{register_animation_types, AnimationAppExt},
    pose::RootMotion,
    root::AnimationRootBone,
    update_animation_players,
};
use concerto_app::{plugins::AssetManagerPlugin, schedule_groups::LateUpdate, App};
use concerto_ecs::{
    component::scene::SceneEntityRef,
    system::{
        access::SystemAccess, executor::single_thread::SingleThreadedExecutor,
        meta::SystemMetadata, schedule::Schedules,
    },
    world::World,
    Entity, IntoSystem, ResMut, System,
};
use concerto_foundation::{
    assets::{
        asset_server::{handle_asset_load_events, AssetServer},
        asset_store::AssetStore,
        content::save_content_asset_into,
        handle::AssetHandle,
        registry::AssetRegistry,
        AssetId, ContentAssetRoot,
    },
    time::Time,
    transform::Transform,
};
use concerto_mesh::skeleton::{Skeleton, SkeletonComponent};
use concerto_reflect::TypeRegistry;
use glam::{Quat, Vec3};

use crate::{
    database::MotionDatabase,
    node::MotionMatchingNode,
    test_support::{pace_database, skeleton, turn_left, walk_forward, Pace},
};

fn access_of<M>(world: &mut World, system: impl IntoSystem<(), M>) -> SystemAccess {
    let mut system = system.into_system();
    system.initialize(world);
    let mut access = SystemAccess::default();
    system.fill_access(&mut SystemMetadata::default(), &mut access);
    access
}

#[test]
fn only_the_update_walk_declares_the_motion_database_store() {
    let mut app = motion_app();
    let world = app.main_mut().world_mut();

    let writer = access_of(world, |_: ResMut<AssetStore<MotionDatabase>>| {});
    let update = access_of(world, update_animation_players);
    let evaluate = access_of(world, animate_targets);

    assert!(!SystemAccess::are_disjoint(&update, &writer));
    assert!(SystemAccess::are_disjoint(&evaluate, &writer));
}

#[test]
fn a_frame_with_a_missing_database_gives_the_root_bone_no_root_motion() {
    let (mut app, bones, _) = playing_app();
    let world = app.main_mut().world_mut();
    world.insert(AnimationRootBone::default(), bones[0]);
    world.run_schedule(LateUpdate);
    world.run_schedule(LateUpdate);

    std::thread::sleep(Duration::from_millis(5));
    world.get_resource_mut::<Time>().unwrap().update();
    world.run_schedule(LateUpdate);
    let moved = world
        .get_component_for_entity::<AnimationRootBone>(bones[0])
        .unwrap()
        .root_motion;
    assert!(moved.translation.z < 0.0, "{moved:?}");

    world.insert_resource(AssetStore::<MotionDatabase>::new());
    world.run_schedule(LateUpdate);

    let stopped = world
        .get_component_for_entity::<AnimationRootBone>(bones[0])
        .unwrap()
        .root_motion;
    assert_eq!(stopped, RootMotion::IDENTITY);
}

#[test]
fn the_graph_walks_then_animate_targets_pose_the_bones_from_the_database() {
    let (mut app, bones, database) = playing_app();
    let world = app.main_mut().world_mut();
    world.run_schedule(LateUpdate);
    world.run_schedule(LateUpdate);

    let expected = database
        .pose(0, &skeleton(), &[&walk_forward(), &turn_left()])
        .unwrap();
    let hips_transform = world
        .get_component_for_entity::<Transform>(bones[1])
        .unwrap();
    assert!(expected[1].translation.distance(Vec3::ZERO) > 0.5);
    assert!(
        hips_transform.translation.distance(expected[1].translation) < 1e-4,
        "{:?} vs {:?}",
        hips_transform.translation,
        expected[1].translation
    );
}

#[test]
fn a_database_frames_root_joint_reaches_the_root_bones_transform() {
    let (mut app, bones, database) = playing_app();
    let world = app.main_mut().world_mut();
    world.insert(AnimationRootBone::default(), bones[0]);
    world.run_schedule(LateUpdate);
    world.run_schedule(LateUpdate);

    let expected = database
        .pose(0, &skeleton(), &[&walk_forward(), &turn_left()])
        .unwrap();
    let root_transform = world
        .get_component_for_entity::<Transform>(bones[0])
        .unwrap();
    assert!(expected[0].rotation.angle_between(Quat::IDENTITY) > 1.0);
    assert!(
        root_transform.rotation.angle_between(expected[0].rotation) < 1e-3,
        "{:?} vs {:?}",
        root_transform.rotation,
        expected[0].rotation
    );
    assert!(
        root_transform.translation.distance(expected[0].translation) < 1e-4,
        "{:?} vs {:?}",
        root_transform.translation,
        expected[0].translation
    );
}

#[test]
fn on_load_makes_the_clips_and_skeleton_strong_with_the_same_ids() {
    let mut app = motion_app();
    let world = app.main_mut().world_mut();
    let server = world.get_resource::<AssetServer>().unwrap().clone();
    let skeleton_id = AssetId::new();
    let clip_ids = [AssetId::new(), AssetId::new()];
    let handle = server.add(pace_database(
        1.0,
        AssetHandle::weak(skeleton_id),
        clip_ids.map(AssetHandle::weak),
    ));
    handle_asset_load_events(world);

    let databases = world.get_resource::<AssetStore<MotionDatabase>>().unwrap();
    let database = databases.get(&handle).unwrap();
    assert!(matches!(database.skeleton(), AssetHandle::Strong(..)));
    assert_eq!(database.skeleton().id(), skeleton_id);
    assert!(database
        .clips()
        .iter()
        .all(|clip| matches!(clip, AssetHandle::Strong(..))));
    let ids: Vec<_> = database.clips().iter().map(AssetHandle::id).collect();
    assert_eq!(ids, clip_ids);
}

#[test]
fn a_database_loaded_through_the_server_plays() {
    let root = std::env::temp_dir().join(format!(
        "motion-database-load-{}",
        AssetId::new().simple_hex()
    ));
    let mut registry = AssetRegistry::new();
    let skeleton_id =
        save_content_asset_into(&skeleton(), &root, "content/skeleton.gasset", &mut registry)
            .unwrap();
    let walk_id =
        save_content_asset_into(&walk_forward(), &root, "content/walk.gasset", &mut registry)
            .unwrap();
    let turn_id =
        save_content_asset_into(&turn_left(), &root, "content/turn.gasset", &mut registry).unwrap();
    let database = pace_database(
        1.0,
        AssetHandle::weak(skeleton_id),
        [AssetHandle::weak(walk_id), AssetHandle::weak(turn_id)],
    );
    let database_id =
        save_content_asset_into(&database, &root, "content/database.gasset", &mut registry)
            .unwrap();

    let mut app = motion_app();
    let world = app.main_mut().world_mut();
    let server = world.get_resource::<AssetServer>().unwrap().clone();
    server.publish_content_source(ContentAssetRoot::Directory(root.clone()), registry);
    let _database = server.load::<MotionDatabase>(database_id);
    let bones = start_playing(
        world,
        AssetHandle::weak(database_id),
        AssetHandle::weak(skeleton_id),
    );

    let started = Instant::now();
    while server.is_loading() {
        assert!(started.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(1));
        world.run_schedule(LateUpdate);
    }
    world.run_schedule(LateUpdate);
    world.run_schedule(LateUpdate);
    std::fs::remove_dir_all(&root).ok();

    let expected = database
        .pose(0, &skeleton(), &[&walk_forward(), &turn_left()])
        .unwrap();
    let hips_transform = world
        .get_component_for_entity::<Transform>(bones[1])
        .unwrap();
    assert!(
        hips_transform.translation.distance(expected[1].translation) < 1e-4,
        "{:?} vs {:?}",
        hips_transform.translation,
        expected[1].translation
    );
}

fn motion_app() -> App {
    let mut app = App::new();
    app.register_plugin(AssetManagerPlugin);
    app.insert_resource(Time::new());
    app.register_plugin(concerto_animation::plugin::AnimationPlugin);
    app.register_asset::<MotionDatabase>();
    app.register_type::<Pace>();
    app.register_animation_node::<MotionMatchingNode>();
    app
}

fn playing_app() -> (App, Vec<Entity>, MotionDatabase) {
    let mut app = motion_app();
    let world = app.main_mut().world_mut();
    let skeleton_handle = AssetHandle::<Skeleton>::weak(AssetId::new());
    let walk_handle = AssetHandle::<AnimationClip>::weak(AssetId::new());
    let turn_handle = AssetHandle::<AnimationClip>::weak(AssetId::new());
    let database = pace_database(
        1.0,
        skeleton_handle.clone(),
        [walk_handle.clone(), turn_handle.clone()],
    );
    let database_handle = AssetHandle::<MotionDatabase>::weak(AssetId::new());
    world
        .get_resource_mut::<AssetStore<Skeleton>>()
        .unwrap()
        .insert(skeleton_handle.id(), skeleton());
    {
        let clips = world
            .get_resource_mut::<AssetStore<AnimationClip>>()
            .unwrap();
        clips.insert(walk_handle.id(), walk_forward());
        clips.insert(turn_handle.id(), turn_left());
    }
    world
        .get_resource_mut::<AssetStore<MotionDatabase>>()
        .unwrap()
        .insert(database_handle.id(), database.clone());

    let entities = start_playing(world, database_handle, skeleton_handle);
    (app, entities, database)
}

fn start_playing(
    world: &mut World,
    database_handle: AssetHandle<MotionDatabase>,
    skeleton_handle: AssetHandle<Skeleton>,
) -> Vec<Entity> {
    let mut type_registry = TypeRegistry::default();
    register_animation_types(&mut type_registry);
    type_registry.register::<MotionMatchingNode>();
    type_registry.register::<AssetHandle<MotionDatabase>>();
    let mut graph = concerto_animation::graph::AnimationGraph::from_node(MotionMatchingNode {
        database: database_handle,
        ..Default::default()
    });
    graph.make_live(&type_registry, None).unwrap();
    let graph_handle =
        AssetHandle::<concerto_animation::graph::AnimationGraph>::weak(AssetId::new());
    world
        .get_resource_mut::<AssetStore<concerto_animation::graph::AnimationGraph>>()
        .unwrap()
        .insert(graph_handle.id(), graph);

    let player = concerto_animation::player::AnimationPlayer::new(3);
    let entities: Vec<_> = (0..3).map(|_| world.spawn(Transform::IDENTITY)).collect();
    let bones = entities
        .iter()
        .map(|entity| SceneEntityRef::Entity(*entity))
        .collect();
    world.spawn((
        player,
        concerto_animation::player::AnimationHandleComponent::new(graph_handle),
        SkeletonComponent {
            skeleton: skeleton_handle,
            bones,
            root: None,
        },
    ));

    let schedules = world.remove_resource::<Schedules>().unwrap();
    let compiled = schedules.compile::<SingleThreadedExecutor>(world);
    world.insert_resource(compiled);
    entities
}
