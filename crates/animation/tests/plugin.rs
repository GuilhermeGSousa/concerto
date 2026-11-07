use std::{
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

use concerto_animation::{
    clip::{Additive, AdditiveBase, AnimationChanelOutput, AnimationChannel, AnimationClip},
    graph::AnimationGraph,
    node::{AimOffsetNode, AimPoint, AnimationClipNode, ErasedAnimationNode},
    player::{AnimationHandleComponent, AnimationPlayer},
    plugin::AnimationPlugin,
};
use concerto_app::{
    App,
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin},
};
use concerto_ecs::component::scene::SceneEntityRef;
use concerto_foundation::{
    assets::{
        AssetId, ContentAssetRoot, asset_server::AssetServer, asset_store::AssetStore,
        content::save_content_asset_into, derived::DerivedDataCache, handle::AssetHandle,
        registry::AssetRegistry,
    },
    transform::Transform,
};
use concerto_mesh::skeleton::{JointPose, Skeleton, SkeletonComponent};
use concerto_name::name;
use glam::Mat4;
use uuid::Uuid;

#[test]
fn animation_plugin_alone_provides_the_skeleton_store() {
    let mut app = App::new();
    app.insert_resource(AssetServer::new());
    app.register_plugin(AnimationPlugin);

    assert!(app.get_resource::<AssetStore<Skeleton>>().is_some());
    assert!(
        app.get_resource::<AssetServer>()
            .unwrap()
            .is_registered::<Skeleton>()
    );
}

#[test]
fn registering_the_skeleton_asset_twice_keeps_it_usable() {
    let mut app = App::new();
    app.insert_resource(AssetServer::new());
    app.register_plugin(AnimationPlugin);
    app.register_asset::<Skeleton>();

    assert!(app.get_resource::<AssetStore<Skeleton>>().is_some());
    assert!(
        app.get_resource::<AssetServer>()
            .unwrap()
            .is_registered::<Skeleton>()
    );
}

#[test]
fn animation_plugin_registers_the_built_in_nodes_and_handle_types() {
    use std::any::TypeId;

    use concerto_animation::clip::AnimationClip;
    use concerto_animation::graph::AnimationGraph;
    use concerto_animation::mask::BoneMask;
    use concerto_animation::node::blend_space::BlendSpace2DNode;
    use concerto_animation::node::state_machine::AnimationStateMachine;
    use concerto_animation::node::{
        AimOffsetNode, AimPoint, AnimationClipNode, BlendNode, LayerNode, PlaceholderNode,
        ReflectAnimationNode, ResultNode, TwoBoneIkNode,
    };
    use concerto_app::InspectedAssets;
    use concerto_ecs::reflect::AppTypeRegistry;
    use concerto_foundation::assets::{Asset, handle::AssetHandle};
    use concerto_mesh::ReflectSkeletonSource;

    let mut app = App::new();
    app.insert_resource(AssetServer::new());
    app.register_plugin(AnimationPlugin);

    assert!(
        app.get_resource::<InspectedAssets>()
            .unwrap()
            .iter()
            .any(|record| record.kind == BoneMask::name())
    );
    assert!(
        app.get_resource::<AssetServer>()
            .unwrap()
            .is_registered::<BoneMask>()
    );
    let registry = app.get_resource::<AppTypeRegistry>().unwrap().read();
    let has_node_data = |id: TypeId| registry.get_type_data::<ReflectAnimationNode>(id).is_some();

    assert!(has_node_data(TypeId::of::<ResultNode>()));
    assert!(has_node_data(TypeId::of::<BlendNode>()));
    assert!(has_node_data(TypeId::of::<AnimationClipNode>()));
    assert!(has_node_data(TypeId::of::<BlendSpace2DNode>()));
    assert!(has_node_data(TypeId::of::<AnimationStateMachine>()));
    assert!(has_node_data(TypeId::of::<PlaceholderNode>()));
    assert!(has_node_data(TypeId::of::<TwoBoneIkNode>()));
    assert!(has_node_data(TypeId::of::<LayerNode>()));
    assert!(has_node_data(TypeId::of::<AimOffsetNode>()));
    assert!(registry.get(TypeId::of::<AimPoint>()).is_some());
    assert!(
        registry
            .get(TypeId::of::<AssetHandle<AnimationClip>>())
            .is_some()
    );
    assert!(
        registry
            .get(TypeId::of::<AssetHandle<AnimationGraph>>())
            .is_some()
    );
    assert!(
        registry
            .get(TypeId::of::<AssetHandle<BoneMask>>())
            .is_some()
    );
    assert!(
        registry
            .get(TypeId::of::<AssetHandle<Skeleton>>())
            .is_some()
    );
    assert!(
        registry
            .get_type_data::<ReflectSkeletonSource>(TypeId::of::<BoneMask>())
            .is_some()
    );
}

#[test]
fn an_aim_offset_graph_loaded_through_the_asset_server_finds_the_clips_of_its_points() {
    let mut app = App::new();
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(TimePlugin)
        .register_plugin(AssetManagerPlugin)
        .register_plugin(AnimationPlugin);
    app.finish_plugin_build();
    let server = app.get_resource::<AssetServer>().unwrap().clone();
    let clip = server.add(AnimationClip::default());
    let graph = server.add(AnimationGraph::from_node(
        AimOffsetNode::new(name!("aim_target"), "bone").with_point(AimPoint::new(
            AssetHandle::weak(clip.id()),
            0.0,
            0.3,
        )),
    ));
    app.update();

    let world = app.main().world();
    let graphs = world.get_resource::<AssetStore<AnimationGraph>>().unwrap();
    let loaded = graphs.get(&graph).expect("the graph loaded");
    let node = loaded
        .live_nodes()
        .find_map(|node: &dyn ErasedAnimationNode| {
            node.as_reflect().downcast_ref::<AimOffsetNode>()
        })
        .expect("the aim offset node is live");
    let point = &node.points[0];
    assert!(matches!(point.clip, AssetHandle::Strong(..)));
    assert_eq!(point.clip.id(), clip.id());
    let clips = world.get_resource::<AssetStore<AnimationClip>>().unwrap();
    assert!(clips.get(&point.clip).is_some());
}

fn bone_x_after_a_few_frames(mut app: App) -> f32 {
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(TimePlugin)
        .register_plugin(AssetManagerPlugin)
        .register_plugin(AnimationPlugin);
    app.finish_plugin_build();

    let bone_id = Uuid::from_u128(7);
    let server = app.get_resource::<AssetServer>().unwrap().clone();
    let mut clip = AnimationClip::default();
    clip.add_channel(
        bone_id,
        AnimationChannel::new(
            vec![0.0, 10.0],
            AnimationChanelOutput::from_translation(
                [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0]].into_iter(),
            ),
        ),
    );
    let clip = server.add(clip);
    let graph = server.add(AnimationGraph::from_node(AnimationClipNode::new(clip)));
    let skeleton = server.add(
        Skeleton::new(
            vec![Mat4::IDENTITY],
            vec![None],
            vec!["bone".into()],
            vec![JointPose::IDENTITY],
            vec![bone_id],
        )
        .unwrap(),
    );
    app.update();

    let world = app.main_mut().world_mut();
    let bone = world.spawn(Transform::IDENTITY);
    world.spawn((
        AnimationPlayer::new(1),
        AnimationHandleComponent::new(graph),
        SkeletonComponent::new(skeleton, vec![SceneEntityRef::Entity(bone)], None),
    ));
    for _ in 0..4 {
        thread::sleep(Duration::from_millis(5));
        app.update();
    }

    app.main()
        .world()
        .get_component_for_entity::<Transform>(bone)
        .unwrap()
        .translation
        .x
}

#[test]
fn an_animation_player_advances_in_a_preview_app_and_not_in_an_edit_app() {
    assert!(bone_x_after_a_few_frames(App::new()) > 0.0);
    assert!(bone_x_after_a_few_frames(App::for_previewing()) > 0.0);
    assert_eq!(bone_x_after_a_few_frames(App::for_editing()), 0.0);
}

const BONE: Uuid = Uuid::from_u128(7);

struct ClipProject {
    root: PathBuf,
    registry: AssetRegistry,
}

impl Drop for ClipProject {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

impl ClipProject {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("additive-clips-{}", AssetId::new().simple_hex()));
        std::fs::create_dir_all(root.join("content")).unwrap();
        Self {
            root,
            registry: AssetRegistry::new(),
        }
    }

    fn save(&mut self, name: &str, clip: &AnimationClip) -> AssetId {
        save_content_asset_into(
            clip,
            &self.root,
            &format!("content/{name}.gasset"),
            &mut self.registry,
        )
        .unwrap()
    }

    fn blobs(&self) -> usize {
        std::fs::read_dir(self.root.join("derived-cache/AnimationClip/v1"))
            .map(|entries| entries.count())
            .unwrap_or(0)
    }
}

fn ramp(to: f32) -> AnimationClip {
    let mut clip = AnimationClip::default();
    clip.add_channel(
        BONE,
        AnimationChannel::new(
            vec![0.0, 1.0],
            AnimationChanelOutput::from_translation([[0.0, 0.0, 0.0], [to, 0.0, 0.0]].into_iter()),
        ),
    );
    clip.set_frame_rate(10.0);
    clip
}

fn additive_ramp(to: f32, base: AssetId) -> AnimationClip {
    let mut clip = ramp(to);
    clip.set_additive(Additive::LocalSpace(AdditiveBase::new(AssetHandle::weak(
        base,
    ))));
    clip
}

struct ClipRuntime {
    app: App,
    server: AssetServer,
}

impl ClipRuntime {
    fn new(root: &Path, registry: &AssetRegistry, cache: DerivedDataCache) -> Self {
        let mut app = App::new();
        app.register_plugin(MainSchedulePlugin)
            .register_plugin(TimePlugin)
            .register_plugin(AssetManagerPlugin)
            .register_plugin(AnimationPlugin);
        app.finish_plugin_build();
        app.update();
        let server = app.get_resource::<AssetServer>().unwrap().clone();
        server.set_derived_cache(cache);
        server.publish_content_source(
            ContentAssetRoot::Directory(root.to_path_buf()),
            registry.clone(),
        );
        Self { app, server }
    }

    fn load<R>(&mut self, id: AssetId, read: impl FnOnce(&AnimationClip) -> R) -> Option<R> {
        let handle = self.server.load::<AnimationClip>(id);
        let started = Instant::now();
        while self.server.is_loading() {
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "load timed out"
            );
            thread::sleep(Duration::from_millis(1));
            self.app.update();
        }
        self.app.update();
        self.app
            .main()
            .world()
            .get_resource::<AssetStore<AnimationClip>>()
            .unwrap()
            .get(&handle)
            .map(read)
    }
}

#[derive(Debug, PartialEq)]
struct BuiltDelta {
    duration: f32,
    halfway: f32,
}

fn built_delta(clip: &AnimationClip) -> BuiltDelta {
    BuiltDelta {
        duration: clip.additive_duration(),
        halfway: clip.sample_additive(0.5, &BONE).translation.x,
    }
}

fn assert_built(delta: Option<BuiltDelta>) {
    let delta = delta.expect("the additive clip loaded");
    assert_eq!(delta.duration, 1.0);
    assert!((delta.halfway - 1.0).abs() < 1e-4, "{delta:?}");
}

#[test]
fn an_additive_clip_loaded_with_its_base_holds_a_built_delta() {
    let mut project = ClipProject::new();
    let base = project.save("base", &ramp(1.0));
    let clip = project.save("additive", &additive_ramp(3.0, base));
    let cache = DerivedDataCache::in_project(&project.root);

    let mut runtime = ClipRuntime::new(&project.root, &project.registry, cache);

    assert_built(runtime.load(clip, built_delta));
    assert_eq!(project.blobs(), 1);
}

#[test]
fn an_additive_clip_whose_base_is_missing_fails_its_load_naming_the_base() {
    let mut project = ClipProject::new();
    let missing = AssetId::new();
    let clip = project.save("additive", &additive_ramp(3.0, missing));

    let mut runtime = ClipRuntime::new(&project.root, &project.registry, DerivedDataCache::none());

    assert!(runtime.load(clip, built_delta).is_none());
    let error = runtime
        .server
        .cook(&project.root.join("cooked"))
        .expect_err("a clip with a missing base can't be cooked");
    let message = format!("{error:#}");
    assert!(
        message.contains(&format!("{missing:?}")) && message.contains("content/additive.gasset"),
        "{message}"
    );
}

#[test]
fn a_plain_clip_loads_with_no_derived_data_cache_and_no_inputs() {
    let mut project = ClipProject::new();
    let clip = project.save("plain", &ramp(1.0));

    let mut runtime = ClipRuntime::new(&project.root, &project.registry, DerivedDataCache::none());

    let loaded = runtime
        .load(clip, built_delta)
        .expect("the plain clip loaded");
    assert_eq!(loaded.duration, 0.0);
    assert_eq!(loaded.halfway, 0.0);
    assert!(!project.root.join("derived-cache").exists());
}

#[test]
fn a_cooked_additive_clip_applies_its_blob_without_its_base() {
    let mut project = ClipProject::new();
    let base = project.save("base", &ramp(1.0));
    let clip = project.save("additive", &additive_ramp(3.0, base));
    let out = project.root.join("cooked");
    let cooking = ClipRuntime::new(&project.root, &project.registry, DerivedDataCache::none());

    let report = cooking.server.cook(&out).unwrap();

    assert_eq!(report.cooked, ["content/additive.gasset"]);
    assert!(!out.join("content/base.gasset").exists());
    let mut runtime = ClipRuntime::new(&out, &project.registry, DerivedDataCache::none());
    assert_built(runtime.load(clip, built_delta));
}
