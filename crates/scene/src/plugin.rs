use concerto_app::{
    edit_time::AtEditTime,
    plugins::Plugin,
    schedule_groups::{Startup, Update},
    App, StartupScene,
};
use concerto_ecs::{CommandQueue, Res};
use concerto_foundation::assets::asset_server::AssetServer;
use concerto_foundation::transform::Transform;
use concerto_mesh::{mesh::MeshComponent, SkeletonComponent};
use concerto_render::components::camera::Camera;
use concerto_render::components::camera_environment::CameraEnvironment;
use concerto_render::components::light::Light;
use concerto_render::components::material::MaterialComponent;
use concerto_render::components::render_entity::SyncWithRenderWorld;

use crate::scene::Scene;
use crate::spawner::{spawn_scene_components, SceneSpawnerComponent};

pub struct ScenePlugin;

impl Plugin for ScenePlugin {
    fn build(&self, app: &mut App) {
        // Mesh, Texture and StandardMaterial are already registered by
        // RenderPlugin / MaterialPlugin; re-registering here would swap their
        // populated AssetStore for an empty one and add a duplicate tracking
        // system, breaking in-flight loads.
        app.register_asset::<Scene>();

        app.register_scene_component::<Transform>();
        app.register_scene_component::<MeshComponent>();
        app.register_scene_component::<MaterialComponent>();
        app.register_scene_component::<Camera>();
        app.register_scene_component::<CameraEnvironment>();
        app.register_scene_component::<Light>();
        app.register_scene_component::<SyncWithRenderWorld>();
        app.register_scene_component::<SkeletonComponent>();

        app.add_system(Update, spawn_scene_components.at_edit_time());
    }

    fn finish(&self, app: &mut App) {
        if app.get_resource::<StartupScene>().is_some() {
            app.add_system(Startup, spawn_startup_scene);
        }
    }
}

fn spawn_startup_scene(
    startup: Res<StartupScene>,
    asset_server: Res<AssetServer>,
    mut cmd: CommandQueue,
) {
    cmd.spawn((
        SceneSpawnerComponent(asset_server.load::<Scene>(startup.0)),
        Transform::default(),
    ));
}
