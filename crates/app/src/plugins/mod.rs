use essential::assets::asset_server::{handle_asset_load_events, AssetServer};
use essential::assets::handle::AssetLifetimeEvent;
use essential::time::{FrameStats, Time};

use ecs::resource::{Res, ResMut};
use essential::transform::systems::{propagate_global_transforms, update_simple_entities};

use crate::schedule_groups::{LateUpdate, Update};
use crate::App;

/// Describes the current phase of plugin initialisation.
#[derive(PartialEq, Eq, Debug, Clone, Copy, PartialOrd, Ord)]
pub enum PluginsState {
    /// Plugins are still being built (waiting for async resources, etc.).
    Building,
    /// All plugins have reported `ready == true`; [`Plugin::finish`] can be called.
    Ready,
    /// [`Plugin::finish`] has been called on every plugin; the app is fully initialised.
    Finished,
}

/// Trait for modular pieces of engine functionality.
///
/// Implement `Plugin` to bundle related systems, resources, and configuration into a
/// reusable unit.  Register plugins with [`App::register_plugin`](crate::App::register_plugin).
///
/// # Lifecycle
/// 1. [`build`](Plugin::build) is called immediately on registration.
/// 2. [`ready`](Plugin::ready) is polled until all plugins return `true`.
/// 3. [`finish`](Plugin::finish) is called once to complete any deferred setup.
pub trait Plugin {
    /// Adds systems, resources, and other configuration to the app.
    fn build(&self, app: &mut App);

    /// Returns `true` once any async initialisation this plugin requires is complete.
    ///
    /// Defaults to `true` (synchronous plugins are always ready immediately).
    fn ready(&self, _app: &App) -> bool {
        true
    }

    /// Called after all plugins are ready; perform final, order-sensitive setup here.
    fn finish(&self, _app: &mut App) {}

    fn name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
}

/// Plugin that inserts a [`Time`] resource and an `update_time` system,
/// plus a [`FrameStats`] rolling window of frame times.
pub struct TimePlugin;

fn update_time(mut time: ResMut<Time>) {
    time.update();
}

fn update_frame_stats(time: Res<Time>, mut stats: ResMut<FrameStats>) {
    let delta = time.delta();
    stats.push(delta);

    // Opt-in visibility without any UI: RUST_LOG=info prints a summary once
    // per second.
    if stats.tick_summary(delta) {
        log::info!(
            "frame: {:.2} ms avg / {:.2} ms p99 / {:.2} ms max ({:.0} FPS)",
            stats.average_ms(),
            stats.percentile_ms(0.99),
            stats.max_ms(),
            stats.fps(),
        );
    }
}

impl Plugin for TimePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Time::new());
        app.insert_resource(FrameStats::new());
        app.add_system(Update, update_time);
        app.add_system(LateUpdate, update_frame_stats);
    }
}

/// Plugin that inserts an [`AssetServer`] resource and the asset-event handler.
pub struct AssetManagerPlugin;

impl Plugin for AssetManagerPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(AssetServer::new());
        app.register_event::<AssetLifetimeEvent>();
        app.add_system(LateUpdate, handle_asset_load_events);
    }

    fn ready(&self, app: &App) -> bool {
        app.get_resource::<AssetServer>()
            .expect("AssetServer resource missing")
            .poll_initialize()
            .unwrap_or_else(|error| panic!("asset manager initialization failed: {error:#}"))
    }
}

/// Plugin that registers [`Transform`] lifecycle callbacks and the global-transform
/// propagation systems.
pub struct TransformPlugin;

impl Plugin for TransformPlugin {
    fn build(&self, app: &mut App) {
        app.add_system(LateUpdate, update_simple_entities)
            .add_system(LateUpdate, propagate_global_transforms);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use essential::assets::{content::AssetRegistry, AssetId, ContentAssetRoot};
    use std::time::{Duration, Instant};

    fn temp_root() -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("asset-plugin-{}", AssetId::new().simple_hex()));
        std::fs::create_dir_all(root.join("content")).unwrap();
        root
    }

    fn wait_for_plugins(app: &mut App) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.plugin_state() == PluginsState::Building {
            assert!(Instant::now() < deadline, "plugin initialization timed out");
            std::thread::yield_now();
        }
    }

    #[test]
    fn initialization_uses_the_servers_content_root() {
        let root = temp_root();
        AssetRegistry::new().save(&root).unwrap();
        let mut app = App::new();
        app.register_plugin(AssetManagerPlugin);
        let server = AssetServer::with_content_root(ContentAssetRoot::Directory(root.clone()));
        app.insert_resource(server.clone());
        wait_for_plugins(&mut app);
        assert!(server.poll_initialize().unwrap());
        // Startup can immediately resolve addresses against the initialized registry.
        app.add_system(
            crate::schedule_groups::Startup,
            |server: Res<AssetServer>| {
                assert!(server.poll_initialize().unwrap());
            },
        );
        app.finish_plugin_build();
        assert_eq!(app.plugin_state(), PluginsState::Finished);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_registry_stops_plugin_initialization_with_an_error() {
        let root = temp_root();
        std::fs::write(root.join("content/.registry.toml"), "broken = [").unwrap();
        let mut app = App::new();
        app.register_plugin(AssetManagerPlugin);
        app.insert_resource(AssetServer::with_content_root(ContentAssetRoot::Directory(
            root.clone(),
        )));
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| wait_for_plugins(&mut app)));
        let panic = result.expect_err("invalid registry should fail instead of polling forever");
        let message = panic.downcast_ref::<String>().unwrap();
        assert!(
            message.contains("asset manager initialization failed"),
            "{message}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
