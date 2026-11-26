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
fn animation_plugin_registers_the_built_in_nodes_and_assets() {
    use std::any::TypeId;

    use concerto_animation::mask::BoneMask;
    use concerto_animation::node::blend_space::BlendSpace2DNode;
    use concerto_animation::node::state_machine::AnimationStateMachine;
    use concerto_animation::node::{
        AdditiveNode, AimOffsetNode, AnimationClipNode, BlendNode, LayerNode, PlaceholderNode,
        ReflectAnimationNode, ResultNode, TwoBoneIkNode,
    };
    use concerto_animation::retarget::RetargetRig;

    use concerto_ecs::reflect::AppTypeRegistry;

    let mut app = App::new();
    app.insert_resource(AssetServer::new());
    app.register_plugin(AnimationPlugin);

    let server = app.get_resource::<AssetServer>().unwrap();
    assert!(server.is_registered::<BoneMask>());
    assert!(server.is_registered::<RetargetRig>());
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
    assert!(has_node_data(TypeId::of::<AdditiveNode>()));
    assert!(has_node_data(TypeId::of::<AimOffsetNode>()));
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

#[test]
fn a_drive_input_that_reads_what_the_graph_drive_writes_panics_when_the_drive_is_built() {
    use concerto_animation::node::DriveNodeInputs;
    use concerto_ecs::Query;

    let mut app = App::new();
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(TimePlugin)
        .register_plugin(AssetManagerPlugin)
        .register_plugin(AnimationPlugin);
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
