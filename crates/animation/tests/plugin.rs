use concerto_animation::plugin::AnimationPlugin;
use concerto_app::App;
use concerto_foundation::assets::{asset_server::AssetServer, asset_store::AssetStore};
use concerto_mesh::skeleton::Skeleton;

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
