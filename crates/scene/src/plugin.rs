use concerto_app::{
    edit_time::AtEditTime,
    plugins::Plugin,
    schedule_groups::{LateUpdate, Startup, Update},
    App, StartupScene,
};
use concerto_ecs::{CommandQueue, IntoSystemConfig, Res};
use concerto_foundation::assets::asset_server::{handle_asset_load_events, AssetServer};

use crate::link::{
    request_link_spawns, spawn_links, wait_for_reloaded_scenes, LinkParent, SceneLink,
    SceneLinkSpawned,
};
use crate::scene::Scene;

pub struct ScenePlugin;

impl Plugin for ScenePlugin {
    fn build(&self, app: &mut App) {
        app.register_placeable_asset::<Scene>()
            .register_type::<LinkParent>()
            .register_event::<SceneLinkSpawned>()
            .add_system(Update, request_link_spawns.at_edit_time())
            .add_system(
                LateUpdate,
                wait_for_reloaded_scenes
                    .after(handle_asset_load_events)
                    .at_edit_time(),
            )
            .add_listener(spawn_links.at_edit_time());
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
    cmd.spawn(SceneLink::new(asset_server.load::<Scene>(startup.0)));
}
