pub use concerto_animation as animation;
pub use concerto_app as app;
pub use concerto_color as color;
pub use concerto_director as director;
pub use concerto_ecs as ecs;
pub use concerto_foundation as foundation;
pub use concerto_foundation::asset_id;
pub use concerto_gameplay as gameplay;
use concerto_gameplay::GameplayPlugin;
pub use concerto_mesh as mesh;
pub use concerto_motion_matching as motion_matching;
pub use concerto_physics as physics;
pub use concerto_reflect as reflect;
pub use concerto_render as render;
pub use concerto_scene as scene;
pub use concerto_skybox as skybox;
pub use concerto_ui as ui;
pub use concerto_window as window;
pub use concerto_world_grid as world_grid;

use concerto_animation::plugin::AnimationPlugin;
use concerto_app::{
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
    App, Plugin,
};
use concerto_director::CameraDirectorPlugin;
use concerto_motion_matching::MotionMatchingPlugin;
use concerto_physics::plugin::PhysicsPlugin;
use concerto_render::{
    assets::material::StandardMaterial, plugin::RenderPlugin,
    shadow_pipeline::ShadowPipelinePlugin, MaterialPlugin,
};
use concerto_scene::plugin::ScenePlugin;
use concerto_skybox::plugin::SkyboxPlugin;
use concerto_ui::plugin::UIPlugin;
use concerto_window::plugin::WindowPlugin;
use concerto_world_grid::plugin::WorldGridPlugin;

/// Sets up logging, opens a window, and runs the `App` that `build` makes with `registry`, from [`content_registry!`], installed.
pub fn run(build: impl FnOnce(&mut app::App) + 'static, registry: &'static str) {
    init_logging();
    concerto_winit::run(move |app| {
        build(app);
        concerto_app::plugins::install_content_registry(app, registry);
    });
}

/// The asset registry the package's `build.rs` wrote through `concerto_asset_build::track_assets`.
#[macro_export]
macro_rules! content_registry {
    () => {
        include_str!(concat!(env!("OUT_DIR"), "/concerto_registry.toml"))
    };
}

#[doc(hidden)]
pub fn init_logging() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = env_logger::try_init();
    }
    #[cfg(target_arch = "wasm32")]
    {
        console_error_panic_hook::set_once();
        let _ = console_log::init_with_level(log::Level::Debug);
    }
}

/// Registers all standard engine plugins in the conventional order.
#[derive(Default)]
pub struct DefaultPlugins;

impl Plugin for DefaultPlugins {
    fn build(&self, app: &mut App) {
        app.register_plugin(MainSchedulePlugin)
            .register_plugin(AssetManagerPlugin)
            .register_plugin(TimePlugin)
            .register_plugin(WindowPlugin);

        app.register_plugin(TransformPlugin)
            .register_plugin(CameraDirectorPlugin)
            .register_plugin(RenderPlugin)
            .register_plugin(SkyboxPlugin)
            .register_plugin(ShadowPipelinePlugin)
            .register_plugin(MaterialPlugin::<StandardMaterial>::default());

        app.register_plugin(PhysicsPlugin)
            .register_plugin(MotionMatchingPlugin)
            .register_plugin(AnimationPlugin)
            .register_plugin(ScenePlugin)
            .register_plugin(WorldGridPlugin)
            .register_plugin(GameplayPlugin)
            .register_plugin(UIPlugin);
    }
}

/// Builds an `App` with a GPU and no window, ready to step; `None` when there is no GPU adapter.
#[cfg(not(target_arch = "wasm32"))]
pub fn headless(build: impl FnOnce(&mut App)) -> Option<App> {
    use concerto_app::plugins::PluginsState;
    use concerto_render::{draw_target::DrawTarget, gpu::Gpu};
    use concerto_window::plugin::Window;

    let gpu = pollster::block_on(Gpu::request(&Gpu::instance(), None))?;
    let mut app = App::new();
    app.insert_resource(gpu)
        .insert_resource(DrawTarget::None)
        .insert_resource(Window::new((1280, 720), 1.0));
    build(&mut app);
    while app.plugin_state() != PluginsState::Ready {}
    app.finish_plugin_build();
    Some(app)
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn setting_up_logging_twice_does_not_panic() {
        super::init_logging();
        super::init_logging();
    }
}
