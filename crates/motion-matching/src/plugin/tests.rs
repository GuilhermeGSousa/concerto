use std::time::{Duration, Instant};

use concerto_animation::{
    clip::AnimationClip,
    event::{AnimationEvent, AnimationSignal},
    graph::AnimationGraph,
    player::{AnimationHandleComponent, AnimationPlayer},
    plugin::{register_animation_types, AnimationAppExt},
    root::AnimationRootBone,
};
use concerto_app::{plugins::AssetManagerPlugin, schedule_groups::LateUpdate, App};
use concerto_ecs::{
    component::scene::SceneEntityRef,
    signal::{
        listener::{IntoListener, Listener},
        On,
    },
    system::{executor::single_thread::SingleThreadedExecutor, schedule::Schedules},
    world::World,
    Entity, ResMut, Resource,
};
use concerto_foundation::{
    assets::{
        asset_server::AssetServer,
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
use glam::Quat;

use crate::{
    channel::FeatureChannelData,
    database::MotionDatabase,
    node::MotionMatchingNode,
    test_support::{pace_database, skeleton, turn_left, unbaked, walk_forward, Pace},
};

#[test]
fn a_float_param_on_the_blackboard_drives_the_nodes_play_rate() {
    let Stocked {
        mut app,
        database_handle,
        skeleton_handle,
        ..
    } = stocked_app();
    let world = app.main_mut().world_mut();
    let graph_handle = add_graph(
        world,
        MotionMatchingNode::new(database_handle).with_play_rate_param(name!("speed")),
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

struct Saved {
    id: AssetId,
    database: MotionDatabase,
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

    fn save(&mut self, weight: f32) -> Saved {
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
        Saved { id, database }
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
    let database_id = project.save(1.0).id;

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
fn derive_then_apply_gives_what_the_bake_gives() {
    let mut project = Project::new();
    let database = project.save(1.0).database;
    let inputs = pollster::block_on(DerivedInputs::read(
        &ContentAssetRoot::Directory(project.root.clone()),
        &project.registry,
        &database.derived_inputs(),
    ))
    .unwrap();
    let blob = database.derive(&inputs, &mut |_, _| {}).unwrap();
    let mut derived = database.clone();
    derived.apply_derived(&blob).unwrap();

    let baked = database
        .bake(&skeleton(), &[&walk_forward(), &turn_left()])
        .unwrap();
    assert_eq!(derived.baked, Some(baked));
}

#[test]
fn a_weight_only_change_reuses_the_cached_blob_and_the_search_sees_the_new_weight() {
    let mut project = Project::new();
    let database_id = project.save(1.0).id;
    let first = project.load(database_id);
    let [blob] = project.blobs().try_into().unwrap();
    let mut other = first.clone();
    other.speed = 2.0;
    let marked = other
        .bake(&skeleton(), &[&walk_forward(), &turn_left()])
        .unwrap();
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
    let Stocked {
        mut app,
        database_handle,
        skeleton_handle,
        database,
    } = stocked_app();
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
    let graph_handle = add_graph(world, MotionMatchingNode::new(database_handle));
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

struct Stocked {
    app: App,
    database_handle: AssetHandle<MotionDatabase>,
    skeleton_handle: AssetHandle<Skeleton>,
    database: MotionDatabase,
}

fn stocked_app() -> Stocked {
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
    Stocked {
        app,
        database_handle,
        skeleton_handle,
        database,
    }
}

fn start_playing(
    world: &mut World,
    database_handle: AssetHandle<MotionDatabase>,
    skeleton_handle: AssetHandle<Skeleton>,
) -> Vec<Entity> {
    let graph_handle = add_graph(world, MotionMatchingNode::new(database_handle));
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
