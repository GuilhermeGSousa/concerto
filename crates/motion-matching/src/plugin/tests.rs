use std::time::{Duration, Instant};

use concerto_animation::{
    animate_targets,
    clip::{Additive, AdditiveBase, AnimationClip},
    event::{AnimationEvent, AnimationSignal},
    graph::AnimationGraph,
    player::{AnimationHandleComponent, AnimationPlayer},
    plugin::{register_animation_types, AnimationAppExt},
    pose::RootMotion,
    root::AnimationRootBone,
    update_animation_players,
};
use concerto_app::{plugins::AssetManagerPlugin, schedule_groups::LateUpdate, App};
use concerto_ecs::{
    component::scene::SceneEntityRef,
    signal::{
        listener::{IntoListener, Listener},
        On,
    },
    system::{
        access::SystemAccess, executor::single_thread::SingleThreadedExecutor,
        meta::SystemMetadata, schedule::Schedules,
    },
    world::World,
    Entity, IntoSystem, ResMut, Resource, System,
};
use concerto_foundation::{
    assets::{
        asset_server::{handle_asset_load_events, AssetServer},
        asset_store::AssetStore,
        content::save_content_asset_into,
        derived::{DerivedData, DerivedDataCache, DerivedInputs},
        handle::AssetHandle,
        registry::AssetRegistry,
        AssetId, ContentAssetRoot,
    },
    time::Time,
    transform::Transform,
};
use concerto_mesh::skeleton::{Skeleton, SkeletonComponent};
use concerto_name::name;
use concerto_reflect::TypeRegistry;
use glam::{Quat, Vec3};

use crate::{
    channel::FeatureChannelData,
    database::MotionDatabase,
    node::MotionMatchingNode,
    test_support::{pace_database, skeleton, turn_left, unbaked, walk_forward, Pace},
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
fn a_float_param_on_the_blackboard_drives_the_nodes_play_rate() {
    let (mut app, database_handle, skeleton_handle, _) = stocked_app();
    let world = app.main_mut().world_mut();
    let graph_handle = add_graph(
        world,
        MotionMatchingNode {
            database: database_handle,
            play_rate_param: Some(name!("speed")),
            ..Default::default()
        },
    );
    let plain = spawn_player(
        world,
        graph_handle.clone(),
        skeleton_handle.clone(),
        AnimationPlayer::new(3),
    );
    let mut fast_player = AnimationPlayer::new(3);
    fast_player.set_float_param(name!("speed"), 2.0);
    let fast = spawn_player(world, graph_handle, skeleton_handle, fast_player);
    compile_schedules(world);
    world.insert(AnimationRootBone::default(), plain[0]);
    world.insert(AnimationRootBone::default(), fast[0]);
    world.run_schedule(LateUpdate);
    world.run_schedule(LateUpdate);

    std::thread::sleep(Duration::from_millis(5));
    world.get_resource_mut::<Time>().unwrap().update();
    world.run_schedule(LateUpdate);

    let moved = |bone: Entity| {
        world
            .get_component_for_entity::<AnimationRootBone>(bone)
            .unwrap()
            .root_motion
            .translation
    };
    let once = moved(plain[0]);
    let twice = moved(fast[0]);
    assert!(once.z < 0.0, "{once:?}");
    assert!(
        twice.distance(once * 2.0) < once.length() * 1e-2,
        "{once:?} vs {twice:?}"
    );
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
    assert!(matches!(&database.skeleton, AssetHandle::Strong(..)));
    assert_eq!(database.skeleton.id(), skeleton_id);
    assert!(database
        .clips()
        .all(|clip| matches!(clip, AssetHandle::Strong(..))));
    let ids: Vec<_> = database.clips().map(AssetHandle::id).collect();
    assert_eq!(ids, clip_ids);
}

struct Project {
    root: std::path::PathBuf,
    registry: AssetRegistry,
    skeleton_id: AssetId,
    clip_ids: [AssetId; 2],
}

impl Project {
    fn new() -> Self {
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
            save_content_asset_into(&turn_left(), &root, "content/turn.gasset", &mut registry)
                .unwrap();
        Self {
            root,
            registry,
            skeleton_id,
            clip_ids: [walk_id, turn_id],
        }
    }

    fn save(&mut self, weight: f32) -> (AssetId, MotionDatabase) {
        let mut database = unbaked(pace_database(
            1.0,
            AssetHandle::weak(self.skeleton_id),
            self.clip_ids.map(AssetHandle::weak),
        ));
        database.channels = vec![FeatureChannelData::live(Pace { weight })];
        let id = save_content_asset_into(
            &database,
            &self.root,
            "content/database.gasset",
            &mut self.registry,
        )
        .unwrap();
        (id, database)
    }

    fn cache(&self) -> DerivedDataCache {
        DerivedDataCache::in_project(&self.root)
    }

    fn blobs(&self) -> Vec<std::path::PathBuf> {
        let dir = self.root.join(format!(
            "derived-cache/MotionDatabase/v{}",
            MotionDatabase::DERIVED_VERSION
        ));
        let mut blobs: Vec<_> = std::fs::read_dir(dir)
            .map(|entries| entries.map(|entry| entry.unwrap().path()).collect())
            .unwrap_or_default();
        blobs.sort();
        blobs
    }

    fn serve(&self, world: &mut World) -> AssetServer {
        let server = world.get_resource::<AssetServer>().unwrap().clone();
        server.set_derived_cache(self.cache());
        server.publish_content_source(
            ContentAssetRoot::Directory(self.root.clone()),
            self.registry.clone(),
        );
        server
    }

    fn load(&self, id: AssetId) -> MotionDatabase {
        self.try_load(id).expect("the database loaded")
    }

    fn try_load(&self, id: AssetId) -> Option<MotionDatabase> {
        let mut app = motion_app();
        let world = app.main_mut().world_mut();
        let server = self.serve(world);
        let handle = server.load::<MotionDatabase>(id);
        compile_schedules(world);
        let started = Instant::now();
        while server.is_loading() {
            assert!(started.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(1));
            world.run_schedule(LateUpdate);
        }
        world.run_schedule(LateUpdate);
        world
            .get_resource::<AssetStore<MotionDatabase>>()
            .unwrap()
            .get(&handle)
            .cloned()
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).ok();
    }
}

#[test]
fn a_database_loaded_through_the_server_derives_and_plays() {
    let mut project = Project::new();
    let (database_id, _) = project.save(1.0);

    let mut app = motion_app();
    let world = app.main_mut().world_mut();
    let server = project.serve(world);
    let _database = server.load::<MotionDatabase>(database_id);
    let bones = start_playing(
        world,
        AssetHandle::weak(database_id),
        AssetHandle::weak(project.skeleton_id),
    );
    world.insert(AnimationRootBone::default(), bones[0]);

    let started = Instant::now();
    while server.is_loading() {
        assert!(started.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(1));
        world.run_schedule(LateUpdate);
    }
    world.run_schedule(LateUpdate);
    world.run_schedule(LateUpdate);

    let loaded = world
        .get_resource::<AssetStore<MotionDatabase>>()
        .unwrap()
        .get(&AssetHandle::<MotionDatabase>::weak(database_id))
        .unwrap()
        .clone();
    assert!(loaded.is_baked());
    assert_eq!(project.blobs().len(), 1);
    let expected = loaded
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
}

#[test]
fn an_entry_naming_an_additive_clip_fails_the_databases_load_naming_the_clip() {
    let mut project = Project::new();
    let [walk_id, turn_id] = project.clip_ids;
    let mut turn = turn_left();
    turn.set_additive(Additive::LocalSpace(AdditiveBase::new(AssetHandle::weak(
        walk_id,
    ))));
    save_content_asset_into(
        &turn,
        &project.root,
        "content/turn.gasset",
        &mut project.registry,
    )
    .unwrap();
    let (database_id, _) = project.save(1.0);

    assert!(project.try_load(database_id).is_none());
    let mut app = motion_app();
    let server = project.serve(app.main_mut().world_mut());
    let error = server
        .cook(&project.root.join("cooked"))
        .expect_err("a database naming an additive clip can't be cooked");
    let message = format!("{error:#}");
    assert!(
        message.contains(&format!("{turn_id:?}"))
            && message.contains("content/turn.gasset")
            && message.contains("itself derived"),
        "{message}"
    );
    assert!(project.blobs().is_empty());
}

#[test]
fn derive_then_apply_gives_what_the_bake_gives() {
    let mut project = Project::new();
    let (_, database) = project.save(1.0);
    let inputs = pollster::block_on(DerivedInputs::read(
        &ContentAssetRoot::Directory(project.root.clone()),
        &project.registry,
        &database.derived_inputs(),
    ))
    .unwrap();
    let mut progress = Vec::new();

    let blob = database
        .derive(&inputs, &mut |finished, total| {
            progress.push((finished, total))
        })
        .unwrap();
    let mut derived = database.clone();
    derived.apply_derived(&blob).unwrap();

    let baked =
        crate::bake::bake(&database, &skeleton(), &[&walk_forward(), &turn_left()]).unwrap();
    assert_eq!(derived.baked, Some(baked));
    assert_eq!(progress, [(1, 2), (2, 2)]);
}

#[test]
fn a_blob_that_does_not_fit_the_database_is_refused() {
    let mut project = Project::new();
    let (_, database) = project.save(1.0);
    let baked =
        crate::bake::bake(&database, &skeleton(), &[&walk_forward(), &turn_left()]).unwrap();
    let blob = bincode::serialize(&baked).unwrap();
    let mut fewer = database.clone();
    fewer.entries.pop();

    assert!(fewer.apply_derived(&blob).is_err());
    assert!(!fewer.is_baked());
}

fn key_of(project: &Project, database: &MotionDatabase) -> u64 {
    let inputs = pollster::block_on(DerivedInputs::read(
        &ContentAssetRoot::Directory(project.root.clone()),
        &project.registry,
        &database.derived_inputs(),
    ))
    .unwrap();
    database.derived_key(&inputs).unwrap()
}

#[test]
fn the_key_follows_every_field_and_input_but_the_weights() {
    let mut project = Project::new();
    let (_, database) = project.save(1.0);
    let key = key_of(&project, &database);

    let mut weighted = database.clone();
    weighted.channels = vec![FeatureChannelData::live(Pace { weight: 5.0 })];
    assert_eq!(key_of(&project, &weighted), key);

    let mut slower = database.clone();
    slower.speed = 0.5;
    assert_ne!(key_of(&project, &slower), key);

    let mut changed_clip = walk_forward();
    changed_clip.add_channel(
        uuid::Uuid::from_u128(3),
        concerto_animation::clip::AnimationChannel::new(
            vec![0.0, 1.0],
            concerto_animation::clip::AnimationChanelOutput::Translation(vec![Vec3::ZERO, Vec3::Y]),
        ),
    );
    save_content_asset_into(
        &changed_clip,
        &project.root,
        "content/walk.gasset",
        &mut project.registry,
    )
    .unwrap();
    assert_ne!(key_of(&project, &database), key);
}

#[test]
fn a_weight_only_change_reuses_the_cached_blob_and_the_search_sees_the_new_weight() {
    let mut project = Project::new();
    let (database_id, _) = project.save(1.0);
    let first = project.load(database_id);
    let [blob] = project.blobs().try_into().unwrap();
    let mut other = first.clone();
    other.speed = 2.0;
    let marked = crate::bake::bake(&other, &skeleton(), &[&walk_forward(), &turn_left()]).unwrap();
    std::fs::write(&blob, bincode::serialize(&marked).unwrap()).unwrap();

    project.save(4.0);
    let second = project.load(database_id);

    assert_eq!(project.blobs(), [blob]);
    assert_eq!(second.baked, Some(marked.clone()));
    let unweighted = MotionDatabase {
        baked: Some(marked),
        ..first
    }
    .scales()
    .unwrap();
    let after = second.scales().unwrap();
    assert!((after[0] * 4.0 - unweighted[0]).abs() < unweighted[0] * 1e-5);
}

#[test]
fn a_non_positive_weight_loads_the_same_derived_database_on_a_miss_and_a_hit() {
    let mut project = Project::new();
    let (database_id, database) = project.save(0.0);
    let baked =
        crate::bake::bake(&database, &skeleton(), &[&walk_forward(), &turn_left()]).unwrap();

    let missed = project.load(database_id);
    let hit = project.load(database_id);

    assert_eq!(project.blobs().len(), 1);
    for loaded in [&missed, &hit] {
        assert_eq!(loaded.baked, Some(baked.clone()));
        assert!(loaded.scales().is_none());
        let query = vec![0.0; loaded.feature_dims().unwrap()];
        assert_eq!(crate::search::search(loaded, &query, None, 0, 0), None);
    }
}

#[test]
fn a_node_does_nothing_for_a_database_not_derived() {
    let (mut app, database_handle, skeleton_handle, database) = stocked_app();
    let world = app.main_mut().world_mut();
    world
        .get_resource_mut::<AssetStore<MotionDatabase>>()
        .unwrap()
        .insert(database_handle.id(), unbaked(database));
    let bones = start_playing(world, database_handle, skeleton_handle);
    world.insert(AnimationRootBone::default(), bones[0]);
    world.run_schedule(LateUpdate);
    world.run_schedule(LateUpdate);

    let root = world
        .get_component_for_entity::<AnimationRootBone>(bones[0])
        .unwrap()
        .root_motion;
    assert_eq!(root, RootMotion::IDENTITY);
    let transform = world
        .get_component_for_entity::<Transform>(bones[0])
        .unwrap();
    assert_eq!(transform.rotation, Quat::IDENTITY);
    assert_eq!(transform.translation, Vec3::ZERO);
}

#[derive(Resource, Default)]
struct Heard(usize);

fn heard(world: &World) -> usize {
    world.get_resource::<Heard>().unwrap().0
}

fn hear() -> Listener<AnimationSignal<AnimationEvent>> {
    (|_: On<AnimationSignal<AnimationEvent>>, mut heard: ResMut<Heard>| {
        heard.0 += 1;
    })
    .into_listener()
}

#[test]
fn a_crossed_clip_event_is_heard_once_and_not_again_while_the_database_is_missing() {
    let (mut app, database_handle, skeleton_handle, database) = stocked_app();
    let world = app.main_mut().world_mut();
    let mut walk = walk_forward();
    walk.add_event(
        1.0 / 60.0,
        AnimationEvent {
            name: name!("step"),
        },
    );
    world
        .get_resource_mut::<AssetStore<AnimationClip>>()
        .unwrap()
        .insert(database.entries[0].clip.id(), walk);
    world.insert_resource(Heard::default());
    world.register_component::<Listener<AnimationSignal<AnimationEvent>>>();
    let graph_handle = add_graph(
        world,
        MotionMatchingNode {
            database: database_handle,
            ..Default::default()
        },
    );
    let bones = (0..3)
        .map(|_| SceneEntityRef::Entity(world.spawn(Transform::IDENTITY)))
        .collect();
    world.spawn((
        AnimationPlayer::new(3),
        AnimationHandleComponent::new(graph_handle),
        SkeletonComponent {
            skeleton: skeleton_handle,
            bones,
            root: None,
        },
        hear(),
    ));
    compile_schedules(world);
    world.run_schedule(LateUpdate);
    world.run_schedule(LateUpdate);
    assert_eq!(heard(world), 0);

    std::thread::sleep(Duration::from_millis(5));
    {
        let time = world.get_resource_mut::<Time>().unwrap();
        time.set_time_scale(20.0);
        time.update();
    }
    world.run_schedule(LateUpdate);
    assert_eq!(heard(world), 1);

    world.insert_resource(AssetStore::<MotionDatabase>::new());
    world.run_schedule(LateUpdate);
    assert_eq!(heard(world), 1);
}

fn motion_app() -> App {
    let mut app = App::new();
    app.register_plugin(AssetManagerPlugin);
    app.insert_resource(Time::new());
    app.register_plugin(concerto_animation::plugin::AnimationPlugin);
    app.register_derived_asset::<MotionDatabase>();
    app.register_type::<Pace>();
    app.register_animation_node::<MotionMatchingNode>();
    app
}

fn playing_app() -> (App, Vec<Entity>, MotionDatabase) {
    let (mut app, database_handle, skeleton_handle, database) = stocked_app();
    let world = app.main_mut().world_mut();
    let entities = start_playing(world, database_handle, skeleton_handle);
    (app, entities, database)
}

fn stocked_app() -> (
    App,
    AssetHandle<MotionDatabase>,
    AssetHandle<Skeleton>,
    MotionDatabase,
) {
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
    (app, database_handle, skeleton_handle, database)
}

fn start_playing(
    world: &mut World,
    database_handle: AssetHandle<MotionDatabase>,
    skeleton_handle: AssetHandle<Skeleton>,
) -> Vec<Entity> {
    let graph_handle = add_graph(
        world,
        MotionMatchingNode {
            database: database_handle,
            ..Default::default()
        },
    );
    let entities = spawn_player(
        world,
        graph_handle,
        skeleton_handle,
        AnimationPlayer::new(3),
    );
    compile_schedules(world);
    entities
}

fn add_graph(world: &mut World, node: MotionMatchingNode) -> AssetHandle<AnimationGraph> {
    let mut type_registry = TypeRegistry::default();
    register_animation_types(&mut type_registry);
    type_registry.register::<MotionMatchingNode>();
    type_registry.register::<AssetHandle<MotionDatabase>>();
    let mut graph = AnimationGraph::from_node(node);
    graph.make_live(&type_registry, None).unwrap();
    let graph_handle = AssetHandle::<AnimationGraph>::weak(AssetId::new());
    world
        .get_resource_mut::<AssetStore<AnimationGraph>>()
        .unwrap()
        .insert(graph_handle.id(), graph);
    graph_handle
}

fn spawn_player(
    world: &mut World,
    graph_handle: AssetHandle<AnimationGraph>,
    skeleton_handle: AssetHandle<Skeleton>,
    player: AnimationPlayer,
) -> Vec<Entity> {
    let entities: Vec<_> = (0..3).map(|_| world.spawn(Transform::IDENTITY)).collect();
    let bones = entities
        .iter()
        .map(|entity| SceneEntityRef::Entity(*entity))
        .collect();
    world.spawn((
        player,
        AnimationHandleComponent::new(graph_handle),
        SkeletonComponent {
            skeleton: skeleton_handle,
            bones,
            root: None,
        },
    ));
    entities
}

fn compile_schedules(world: &mut World) {
    let schedules = world.remove_resource::<Schedules>().unwrap();
    let compiled = schedules.compile::<SingleThreadedExecutor>(world);
    world.insert_resource(compiled);
}
