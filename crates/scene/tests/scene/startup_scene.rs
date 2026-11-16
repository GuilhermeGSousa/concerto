use concerto_app::{
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
    App,
};
use concerto_foundation::assets::AssetId;
use concerto_scene::{link::SceneLink, plugin::ScenePlugin};

struct Startup {
    name: &'static str,
    app: fn() -> App,
    declare: bool,
    links: usize,
}

#[test]
fn only_a_game_app_spawns_the_startup_scene_it_declares() {
    let cases = [
        Startup {
            name: "a game app",
            app: App::new,
            declare: true,
            links: 1,
        },
        Startup {
            name: "an edit app",
            app: App::for_editing,
            declare: true,
            links: 0,
        },
        Startup {
            name: "a preview app",
            app: App::for_previewing,
            declare: true,
            links: 0,
        },
    ];
    for case in cases {
        let mut app = (case.app)();
        app.register_plugin(MainSchedulePlugin)
            .register_plugin(AssetManagerPlugin)
            .register_plugin(TimePlugin)
            .register_plugin(TransformPlugin)
            .register_plugin(ScenePlugin);
        if case.declare {
            app.set_startup_scene(AssetId::new());
        }
        app.finish_plugin_build();
        let world = app.main_mut().world_mut();
        assert_eq!(
            world.query::<&SceneLink, ()>().iter(world).count(),
            case.links,
            "{}",
            case.name
        );
    }
}
