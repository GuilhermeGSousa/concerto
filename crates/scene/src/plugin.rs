use concerto_app::{
    edit_time::AtEditTime,
    plugins::Plugin,
    schedule_groups::{Startup, Update},
    App, StartupScene,
};
use concerto_ecs::{CommandQueue, Res};
use concerto_foundation::assets::asset_server::AssetServer;
use concerto_foundation::transform::Transform;

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
