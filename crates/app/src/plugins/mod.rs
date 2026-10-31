use concerto_foundation::assets::asset_server::{
    handle_asset_load_events, sync_asset_registry, AssetServer,
};
use concerto_foundation::assets::handle::AssetLifetimeEvent;
use concerto_foundation::assets::registry::AssetRegistry;
use concerto_foundation::time::{FrameStats, Time};

use concerto_ecs::resource::{Res, ResMut};
use concerto_foundation::transform::systems::{
    propagate_global_transforms, update_simple_entities,
};
use concerto_foundation::transform::Transform;

use crate::edit_time::AtEditTime;
use crate::schedule_groups::{First, LateUpdate, Update};
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
pub trait Plugin: Send + Sync + 'static {
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
        app.add_system(Update, update_time.at_edit_time());
        app.add_system(LateUpdate, update_frame_stats.at_edit_time());
    }
}

/// Plugin that inserts an [`AssetServer`] resource and the asset-event handler.
pub struct AssetManagerPlugin;

impl Plugin for AssetManagerPlugin {
    fn build(&self, app: &mut App) {
        let server = AssetServer::new();
        server.set_type_registry(app.main_mut().world_mut().type_registry());
        app.insert_resource(server);
        app.register_event::<AssetLifetimeEvent>();
        app.add_system(LateUpdate, handle_asset_load_events.at_edit_time());
        app.add_system(First, sync_asset_registry.at_edit_time());
    }

    fn finish(&self, app: &mut App) {
        if app.get_resource::<AssetRegistry>().is_some() {
            return;
        }
        let registry = app
            .get_resource::<AssetServer>()
            .and_then(AssetServer::registry)
            .map(|registry| (*registry).clone())
            .unwrap_or_default();
        app.insert_resource(registry);
    }
}

/// Parses embedded registry text into the app's [`AssetRegistry`] and publishes it to its [`AssetServer`], if any; panics when malformed.
pub fn install_content_registry(app: &mut App, text: &str) {
    let registry = AssetRegistry::parse(text)
        .unwrap_or_else(|error| panic!("the embedded content registry is malformed: {error:#}"));
    if let Some(server) = app.get_resource::<AssetServer>() {
        server.publish_registry(registry.clone());
    }
    app.insert_resource(registry);
}

/// Plugin that registers [`Transform`] lifecycle callbacks and the global-transform
/// propagation systems.
pub struct TransformPlugin;

impl Plugin for TransformPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Transform>();
        app.add_system(LateUpdate, update_simple_entities.at_edit_time())
            .add_system(LateUpdate, propagate_global_transforms.at_edit_time());
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use concerto_foundation::assets::{
        registry::{AssetEntry, AssetRegistry},
        AssetId, ContentAssetRoot,
    };
    use concerto_reflect::ReflectDefault;
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
    fn the_published_registry_becomes_a_resource_once_plugins_finish() {
        let root = temp_root();
        let entry = AssetEntry {
            id: AssetId::new(),
            address: "content/hero.gasset".into(),
            kind: "Mesh".into(),
            kind_version: None,
            provenance: None,
        };
        let mut registry = AssetRegistry::new();
        registry.insert(entry.clone());
        let mut app = App::new();
        app.register_plugin(AssetManagerPlugin);
        let server = AssetServer::with_content_root(ContentAssetRoot::Directory(root.clone()));
        server.publish_registry(registry);
        app.insert_resource(server);
        wait_for_plugins(&mut app);
        app.finish_plugin_build();

        let registry = app.get_resource::<AssetRegistry>().unwrap();
        assert_eq!(registry.get(entry.id), Some(&entry));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_registry_the_host_supplied_is_kept_when_plugins_finish() {
        let root = temp_root();
        let supplied = AssetEntry {
            id: AssetId::new(),
            address: "content/supplied.gasset".into(),
            kind: "Mesh".into(),
            kind_version: None,
            provenance: None,
        };
        let mut registry = AssetRegistry::new();
        registry.insert(supplied.clone());
        let mut app = App::new();
        app.register_plugin(AssetManagerPlugin);
        app.insert_resource(AssetServer::with_content_root(ContentAssetRoot::Directory(
            root.clone(),
        )));
        app.insert_resource(registry);
        wait_for_plugins(&mut app);
        app.finish_plugin_build();

        let registry = app.get_resource::<AssetRegistry>().unwrap();
        assert_eq!(registry.get(supplied.id), Some(&supplied));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
    struct Note {
        text: String,
    }

    impl concerto_foundation::assets::Asset for Note {
        fn name() -> &'static str {
            "Note"
        }
    }

    impl concerto_foundation::assets::LoadableAsset for Note {}

    #[test]
    fn an_installed_registry_loads_an_asset_by_id_with_no_registry_file() {
        use concerto_foundation::assets::{
            asset_store::AssetStore, content::save_content_asset_into,
        };

        let root = temp_root();
        let mut scanned = AssetRegistry::new();
        let note = Note {
            text: "hello".into(),
        };
        let id =
            save_content_asset_into(&note, &root, "content/note.gasset", &mut scanned).unwrap();
        let mut app = App::new();
        app.register_plugin(crate::main_schedule::MainSchedulePlugin)
            .register_plugin(TimePlugin)
            .register_plugin(AssetManagerPlugin);
        app.insert_resource(AssetServer::with_content_root(ContentAssetRoot::Directory(
            root.clone(),
        )));
        app.register_asset::<Note>();

        install_content_registry(&mut app, &scanned.to_toml());
        wait_for_plugins(&mut app);
        app.finish_plugin_build();
        assert!(!root.join("content/.registry.toml").exists());

        let handle = app.get_resource::<AssetServer>().unwrap().load::<Note>(id);
        let deadline = Instant::now() + Duration::from_secs(10);
        while app
            .get_resource::<AssetStore<Note>>()
            .unwrap()
            .get(&handle)
            .is_none()
        {
            assert!(Instant::now() < deadline, "the note never loaded");
            app.update();
            std::thread::yield_now();
        }
        assert_eq!(
            app.get_resource::<AssetStore<Note>>().unwrap().get(&handle),
            Some(&note)
        );
        assert_eq!(
            app.get_resource::<AssetRegistry>().unwrap().get(id),
            Some(&AssetEntry {
                kind_version: None,
                ..scanned.get(id).unwrap().clone()
            })
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[derive(concerto_reflect::Reflect, Default)]
    #[reflect(Default)]
    struct Probed;

    #[derive(serde::Serialize, serde::Deserialize)]
    struct Probe {
        #[serde(skip)]
        saw_default: bool,
    }

    impl concerto_foundation::assets::Asset for Probe {
        fn name() -> &'static str {
            "Probe"
        }
    }

    impl concerto_foundation::assets::LoadableAsset for Probe {
        fn on_load(
            &mut self,
            context: &concerto_foundation::assets::asset_server::AssetLoadContext,
        ) -> anyhow::Result<()> {
            self.saw_default = context
                .type_registry()
                .get_type_data::<ReflectDefault>(std::any::TypeId::of::<Probed>())
                .is_some();
            Ok(())
        }
    }

    #[test]
    fn a_pool_load_reads_types_registered_in_the_app_after_the_plugin_built() {
        use concerto_foundation::assets::{
            asset_store::AssetStore, content::save_content_asset_into,
        };

        let root = temp_root();
        let mut scanned = AssetRegistry::new();
        let id = save_content_asset_into(
            &Probe { saw_default: false },
            &root,
            "content/probe.gasset",
            &mut scanned,
        )
        .unwrap();
        let mut app = App::new();
        app.register_plugin(crate::main_schedule::MainSchedulePlugin)
            .register_plugin(TimePlugin)
            .register_plugin(AssetManagerPlugin);
        app.register_asset::<Probe>();
        wait_for_plugins(&mut app);
        app.finish_plugin_build();
        app.register_type::<Probed>();
        let server = app.get_resource::<AssetServer>().unwrap();
        server.publish_content_source(ContentAssetRoot::Directory(root.clone()), scanned);

        let handle = server.load::<Probe>(id);
        let deadline = Instant::now() + Duration::from_secs(10);
        while app
            .get_resource::<AssetStore<Probe>>()
            .unwrap()
            .get(&handle)
            .is_none()
        {
            assert!(Instant::now() < deadline, "the probe never loaded");
            app.update();
            std::thread::yield_now();
        }
        let probe = app
            .get_resource::<AssetStore<Probe>>()
            .unwrap()
            .get(&handle)
            .unwrap();
        assert!(probe.saw_default);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[should_panic(expected = "content registry")]
    fn installing_a_malformed_registry_panics() {
        let mut app = App::new();
        app.register_plugin(AssetManagerPlugin);
        install_content_registry(&mut app, "broken = [");
    }

    #[test]
    fn an_app_without_an_asset_server_gets_the_registry_resource() {
        let entry = AssetEntry {
            id: AssetId::new(),
            address: "content/hero.gasset".into(),
            kind: "Mesh".into(),
            kind_version: None,
            provenance: None,
        };
        let mut registry = AssetRegistry::new();
        registry.insert(entry.clone());
        let mut app = App::new();

        install_content_registry(&mut app, &registry.to_toml());

        assert_eq!(
            app.get_resource::<AssetRegistry>().unwrap().get(entry.id),
            Some(&entry)
        );
    }
}
