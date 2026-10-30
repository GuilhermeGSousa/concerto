use std::{thread, time::Duration};

use concerto_animation::{
    clip::{AnimationChanelOutput, AnimationChannel, AnimationClip},
    graph::AnimationGraph,
    node::AnimationClipNode,
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
    assets::{asset_server::AssetServer, asset_store::AssetStore},
    transform::Transform,
};
use concerto_mesh::skeleton::{JointPose, Skeleton, SkeletonComponent};
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
    use concerto_animation::node::blend_space::BlendSpace2DNode;
    use concerto_animation::node::state_machine::AnimationStateMachine;
    use concerto_animation::node::{
        AnimationClipNode, BlendNode, PlaceholderNode, ReflectAnimationNode, ResultNode,
    };
    use concerto_ecs::reflect::AppTypeRegistry;
    use concerto_foundation::assets::handle::AssetHandle;

    let mut app = App::new();
    app.insert_resource(AssetServer::new());
    app.register_plugin(AnimationPlugin);

    let registry = app.get_resource::<AppTypeRegistry>().unwrap().read();
    let has_node_data = |id: TypeId| registry.get_type_data::<ReflectAnimationNode>(id).is_some();

    assert!(has_node_data(TypeId::of::<ResultNode>()));
    assert!(has_node_data(TypeId::of::<BlendNode>()));
    assert!(has_node_data(TypeId::of::<AnimationClipNode>()));
    assert!(has_node_data(TypeId::of::<BlendSpace2DNode>()));
    assert!(has_node_data(TypeId::of::<AnimationStateMachine>()));
    assert!(has_node_data(TypeId::of::<PlaceholderNode>()));
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
            vec!["bone".to_string()],
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
