use concerto_app::{
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
    App,
};
use concerto_foundation::assets::AssetId;
use concerto_scene::{plugin::ScenePlugin, spawner::SceneSpawnerComponent};

fn spawners_after_startup(mut app: App, declare: bool) -> usize {
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(AssetManagerPlugin)
        .register_plugin(TimePlugin)
        .register_plugin(TransformPlugin)
        .register_plugin(ScenePlugin);
    if declare {
        app.set_startup_scene(AssetId::new());
    }
    app.finish_plugin_build();
    let world = app.main_mut().world_mut();
    world
        .query::<&SceneSpawnerComponent, ()>()
        .iter(world)
        .count()
}

#[test]
fn a_game_app_spawns_its_declared_startup_scene() {
    assert_eq!(spawners_after_startup(App::new(), true), 1);
}

#[test]
fn an_edit_app_does_not_spawn_the_startup_scene() {
    assert_eq!(spawners_after_startup(App::for_editing(), true), 0);
}

#[test]
fn without_a_declaration_nothing_is_spawned() {
    assert_eq!(spawners_after_startup(App::new(), false), 0);
}

#[test]
fn a_preview_app_does_not_spawn_the_startup_scene() {
    assert_eq!(spawners_after_startup(App::for_previewing(), true), 0);
}
