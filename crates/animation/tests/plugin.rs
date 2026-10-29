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
