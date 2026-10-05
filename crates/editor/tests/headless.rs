//! The editor's core must run without a window: no `WindowPlugin`, no
//! `UIPlugin`, and so no `Window`, `Input`, `ActionMap` or UI resources. A
//! panel system leaking into the core panics here on its first missing `Res`.
use concerto_app::{
    App,
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
};
use concerto_editor::{EditorPlugin, viewport::FlyCamera};
use concerto_mesh::Mesh;
use concerto_render::assets::texture::Texture;

/// Apps in one process share the global compute pool, and as many apps
/// updating at once as the pool has threads deadlock it. Run one at a time.
static ONE_APP_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn headless_editor() -> (App, std::sync::MutexGuard<'static, ()>) {
    let turn = ONE_APP_AT_A_TIME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut app = App::new();
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(AssetManagerPlugin)
        .register_plugin(TimePlugin)
        .register_plugin(TransformPlugin);
    // RenderPlugin needs a GPU adapter; register the asset types it would.
    app.register_asset::<Mesh>().register_asset::<Texture>();
    app.register_plugin(concerto_scene::plugin::ScenePlugin)
        .register_plugin(EditorPlugin {
            headless: true,
            viewport_size: [640, 360],
            ..Default::default()
        });
    app.finish_plugin_build();
    (app, turn)
}

#[test]
fn the_editor_core_runs_frames_without_a_window() {
    let (mut app, _turn) = headless_editor();
    for _ in 0..3 {
        app.update();
    }
}

#[test]
fn the_camera_follows_the_fly_camera_without_input() {
    let (mut app, _turn) = headless_editor();
    app.update();
    let target = glam::Vec3::new(3.0, 4.0, 5.0);
    app.get_resource_mut::<FlyCamera>().unwrap().position = target;
    app.update();
    let world = app.main_mut().world_mut();
    let cameras: Vec<_> = world
        .query::<(
            &concerto_render::components::camera::Camera,
            &concerto_foundation::transform::Transform,
            &concerto_foundation::transform::GlobalTransform,
        ), ()>()
        .iter(world)
        .map(|(camera, transform, global)| {
            (camera.aspect, transform.translation, global.translation())
        })
        .collect();
    assert_eq!(cameras.len(), 1, "the editor spawns one camera");
    let (aspect, translation, global) = cameras[0];
    assert_eq!(
        translation, target,
        "a tool moves the camera by writing FlyCamera"
    );
    assert_eq!(
        global, target,
        "the move propagates in the same frame, or the camera renders a frame behind"
    );
    assert!(
        (aspect - 640.0 / 360.0).abs() < 1e-5,
        "the camera's aspect follows the configured viewport size"
    );
}
