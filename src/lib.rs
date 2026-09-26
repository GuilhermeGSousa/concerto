pub use concerto_animation as animation;
pub use concerto_app as app;
pub use concerto_audio as audio;
pub use concerto_color as color;
pub use concerto_director as director;
pub use concerto_ecs as ecs;
pub use concerto_foundation as foundation;
pub use concerto_foundation::asset_id;
pub use concerto_gameplay as gameplay;
use concerto_gameplay::GameplayPlugin;
pub use concerto_mesh as mesh;
pub use concerto_physics as physics;
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
use concerto_audio::AudioPlugin;
use concerto_director::CameraDirectorPlugin;
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

/// Registers all standard engine plugins in the conventional order.
#[derive(Default)]
pub struct DefaultPlugins {
    headless: bool,
}

impl DefaultPlugins {
    pub fn headless() -> Self {
        Self { headless: true }
    }
}

impl Plugin for DefaultPlugins {
    fn build(&self, app: &mut App) {
        app.register_plugin(MainSchedulePlugin)
            .register_plugin(AssetManagerPlugin)
            .register_plugin(TimePlugin);

        if !self.headless {
            app.register_plugin(WindowPlugin);
        }
        // CameraPlugin goes before RenderPlugin so it demotes stray window
        // cameras before `camera_added` gives them render resources.
        app.register_plugin(TransformPlugin)
            .register_plugin(CameraDirectorPlugin)
            .register_plugin(RenderPlugin)
            .register_plugin(SkyboxPlugin)
            .register_plugin(ShadowPipelinePlugin)
            .register_plugin(MaterialPlugin::<StandardMaterial>::default());

        app.register_plugin(PhysicsPlugin)
            .register_plugin(AnimationPlugin)
            .register_plugin(ScenePlugin)
            .register_plugin(WorldGridPlugin)
            .register_plugin(GameplayPlugin);

        if !self.headless {
            app.register_plugin(UIPlugin).register_plugin(AudioPlugin);
        }
    }
}
