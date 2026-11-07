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
pub use concerto_name as name;
pub use concerto_noise as noise;
pub use concerto_physics as physics;
pub use concerto_procedural_animation as procedural_animation;
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
#[cfg(not(target_arch = "wasm32"))]
use concerto_foundation::assets::derived::CookReport;
use concerto_motion_matching::MotionMatchingPlugin;
use concerto_physics::plugin::PhysicsPlugin;
use concerto_procedural_animation::ProceduralAnimationPlugin;
use concerto_render::{
    assets::material::StandardMaterial, plugin::RenderPlugin,
    shadow_pipeline::ShadowPipelinePlugin, MaterialPlugin,
};
use concerto_scene::plugin::ScenePlugin;
use concerto_skybox::plugin::SkyboxPlugin;
use concerto_ui::plugin::UIPlugin;
use concerto_window::plugin::WindowPlugin;
use concerto_world_grid::plugin::WorldGridPlugin;

/// Sets up logging, opens a window, and runs the `App` that `build` makes with `content`, from [`content_registry!`], installed; natively, `--cook <out>` runs [`cook`] instead and exits.
pub fn run(
    build: impl FnOnce(&mut app::App) + 'static,
    content: concerto_app::plugins::ContentSettings<'static>,
) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        match cook_arg(std::env::args().skip(1)) {
            CookArg::Absent => {}
            CookArg::Dir(out) => std::process::exit(report_cook(cook(build, content, &out))),
            CookArg::Missing => std::process::exit(report_cook(Err(anyhow::anyhow!(
                "--cook needs the directory to write the cooked content to"
            )))),
            CookArg::Flag(flag) => std::process::exit(report_cook(Err(anyhow::anyhow!(
                "--cook needs the directory to write the cooked content to, not the flag `{flag}`"
            )))),
        }
    }
    init_logging();
    concerto_winit::run(move |app| {
        build(app);
        concerto_app::plugins::install_content_registry(app, content);
    });
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, PartialEq, Eq)]
enum CookArg {
    Absent,
    Missing,
    Flag(String),
    Dir(std::path::PathBuf),
}

#[cfg(not(target_arch = "wasm32"))]
fn cook_arg(args: impl IntoIterator<Item = String>) -> CookArg {
    let mut args = args.into_iter();
    if !args.any(|arg| arg == "--cook") {
        return CookArg::Absent;
    }
    match args.next() {
        None => CookArg::Missing,
        Some(out) if out.starts_with('-') => CookArg::Flag(out),
        Some(out) => CookArg::Dir(out.into()),
    }
}

#[cfg(not(target_arch = "wasm32"))]
const COOK_MARKER: &str = ".concerto-cook";

/// Builds the `App` that `build` makes, with no GPU and no window, and writes every asset of `content` that derives to `out` with its derived blob; `out` must be missing, empty, or a previous cook output, which it replaces.
#[cfg(not(target_arch = "wasm32"))]
pub fn cook(
    build: impl FnOnce(&mut App),
    content: concerto_app::plugins::ContentSettings,
    out: &std::path::Path,
) -> anyhow::Result<CookReport> {
    use anyhow::Context;
    use concerto_foundation::assets::{
        asset_server::AssetServer, derived::DerivedDataCache, registry::AssetRegistry,
        ContentAssetRoot,
    };

    init_logging();
    let project_root = std::path::PathBuf::from(
        content
            .project_root
            .context("the cook needs the project root that `content_registry!()` embeds")?,
    );
    let registry = AssetRegistry::parse(content.registry)
        .context("the embedded content registry is malformed")?;
    let mut app = App::new();
    build(&mut app);
    let server = app
        .get_resource::<AssetServer>()
        .context("the cook needs an AssetServer; register AssetManagerPlugin")?;
    server.publish_content_source(ContentAssetRoot::Directory(project_root.clone()), registry);
    server.set_derived_cache(DerivedDataCache::in_project(&project_root));
    if out.exists() {
        let empty = std::fs::read_dir(out)
            .with_context(|| format!("failed to read '{}'", out.display()))?
            .next()
            .is_none();
        anyhow::ensure!(
            empty || out.join(COOK_MARKER).is_file(),
            "'{}' is not empty and is not a previous cook output, which holds a `{COOK_MARKER}` file",
            out.display()
        );
        std::fs::remove_dir_all(out)
            .with_context(|| format!("failed to remove '{}'", out.display()))?;
    }
    std::fs::create_dir_all(out)
        .with_context(|| format!("failed to create '{}'", out.display()))?;
    std::fs::write(out.join(COOK_MARKER), "")
        .with_context(|| format!("failed to write '{}'", out.join(COOK_MARKER).display()))?;
    server.cook(out)
}

#[cfg(not(target_arch = "wasm32"))]
fn report_cook(result: anyhow::Result<CookReport>) -> i32 {
    match result {
        Ok(report) => {
            for address in &report.cooked {
                println!("cooked {address}");
            }
            println!("cooked {} derived assets", report.cooked.len());
            0
        }
        Err(error) => {
            eprintln!("error: {error:#}");
            1
        }
    }
}

/// The asset registry and project root the package's `build.rs` recorded through `concerto_asset_build::track_assets`.
#[macro_export]
macro_rules! content_registry {
    () => {
        $crate::app::plugins::ContentSettings {
            registry: include_str!(concat!(env!("OUT_DIR"), "/concerto_registry.toml")),
            project_root: option_env!("CONCERTO_PROJECT_ROOT"),
        }
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
            .register_plugin(ProceduralAnimationPlugin)
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

#[cfg(all(test, not(target_arch = "wasm32")))]
mod cook_arg_tests {
    use super::{cook_arg, CookArg};

    fn arg(args: &[&str]) -> CookArg {
        cook_arg(args.iter().map(|arg| arg.to_string()))
    }

    #[test]
    fn no_cook_flag_is_absent() {
        assert_eq!(arg(&[]), CookArg::Absent);
        assert_eq!(arg(&["--forest"]), CookArg::Absent);
    }

    #[test]
    fn a_cook_flag_with_nothing_after_it_is_missing() {
        assert_eq!(arg(&["--forest", "--cook"]), CookArg::Missing);
    }

    #[test]
    fn a_cook_flag_followed_by_a_flag_is_refused() {
        assert_eq!(
            arg(&["--cook", "--forest"]),
            CookArg::Flag("--forest".to_string())
        );
    }

    #[test]
    fn a_cook_flag_takes_the_directory_after_it() {
        assert_eq!(
            arg(&["--cook", "out"]),
            CookArg::Dir(std::path::PathBuf::from("out"))
        );
        assert_eq!(
            arg(&["--forest", "--cook", "cooked/dir", "--sanatorium"]),
            CookArg::Dir(std::path::PathBuf::from("cooked/dir"))
        );
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod cook_tests {
    use concerto_app::{plugins::ContentSettings, App};
    use concerto_foundation::assets::{
        content::{read_content_asset_parts, save_content_asset_into},
        derived::{DerivedData, DerivedInputs},
        registry::AssetRegistry,
        Asset, AssetId, LoadableAsset,
    };
    use concerto_render::gpu::Gpu;
    use serde::{Deserialize, Serialize};

    use crate::DefaultPlugins;

    #[derive(Serialize, Deserialize)]
    struct Doubled {
        value: u64,
        #[serde(skip)]
        doubled: Option<u64>,
    }

    impl Asset for Doubled {
        fn name() -> &'static str {
            "Doubled"
        }
    }

    impl LoadableAsset for Doubled {}

    impl DerivedData for Doubled {
        const DERIVED_VERSION: u32 = 1;

        fn derived_inputs(&self) -> Vec<AssetId> {
            Vec::new()
        }

        fn derived_key(&self, _inputs: &DerivedInputs) -> anyhow::Result<u64> {
            Ok(self.value)
        }

        fn derive(
            &self,
            _inputs: &DerivedInputs,
            _progress: &mut dyn FnMut(usize, usize),
        ) -> anyhow::Result<Vec<u8>> {
            Ok((self.value * 2).to_le_bytes().to_vec())
        }

        fn apply_derived(&mut self, blob: &[u8]) -> anyhow::Result<()> {
            let bytes: [u8; 8] = blob.try_into()?;
            self.doubled = Some(u64::from_le_bytes(bytes));
            Ok(())
        }
    }

    fn recipe(app: &mut App) {
        app.register_plugin(DefaultPlugins)
            .register_derived_asset::<Doubled>();
        assert!(app.get_resource::<Gpu>().is_none());
    }

    struct DoubledProject {
        dir: tempfile::TempDir,
        registry: String,
    }

    impl DoubledProject {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let mut registry = AssetRegistry::new();
            save_content_asset_into(
                &Doubled {
                    value: 21,
                    doubled: None,
                },
                dir.path(),
                "content/doubled.gasset",
                &mut registry,
            )
            .unwrap();
            Self {
                dir,
                registry: registry.to_toml(),
            }
        }

        fn path(&self) -> &std::path::Path {
            self.dir.path()
        }

        fn cook(&self, out: &std::path::Path) -> anyhow::Result<super::CookReport> {
            super::cook(
                recipe,
                ContentSettings {
                    registry: &self.registry,
                    project_root: self.path().to_str(),
                },
                out,
            )
        }
    }

    #[test]
    fn a_cook_with_no_gpu_writes_each_derived_asset_and_its_blob() {
        let project = DoubledProject::new();
        let out = project.path().join("cooked");

        let report = project.cook(&out).unwrap();

        assert_eq!(report.cooked, vec!["content/doubled.gasset"]);
        assert!(out.join(".concerto-cook").is_file());
        let bytes = std::fs::read(out.join("content/doubled.gasset")).unwrap();
        let parts = read_content_asset_parts(&bytes).unwrap();
        assert_eq!(parts.header.derived.map(|section| section.key), Some(21));
        assert_eq!(parts.derived_blob, Some(&42u64.to_le_bytes()[..]));
    }

    #[test]
    fn a_cook_replaces_a_previous_cook_output() {
        let project = DoubledProject::new();
        let out = project.path().join("cooked");
        project.cook(&out).unwrap();
        std::fs::write(out.join("stale.gasset"), b"stale").unwrap();

        let report = project.cook(&out).unwrap();

        assert_eq!(report.cooked, vec!["content/doubled.gasset"]);
        assert!(!out.join("stale.gasset").exists());
        assert!(out.join("content/doubled.gasset").is_file());
    }

    #[test]
    fn a_cook_refuses_a_non_empty_directory_it_did_not_write() {
        let project = DoubledProject::new();
        let out = project.path().join("mine");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join("notes.txt"), b"mine").unwrap();

        let error = project.cook(&out).unwrap_err();

        assert!(error.to_string().contains("not empty"), "{error:#}");
        assert_eq!(std::fs::read(out.join("notes.txt")).unwrap(), b"mine");
        assert!(!out.join("content").exists());
    }

    #[test]
    fn a_cook_without_a_project_root_is_refused() {
        let out = tempfile::tempdir().unwrap();

        let error = super::cook(
            recipe,
            ContentSettings {
                registry: "",
                project_root: None,
            },
            &out.path().join("cooked"),
        )
        .unwrap_err();

        assert!(error.to_string().contains("project root"), "{error:#}");
    }

    #[test]
    fn a_cook_of_an_app_without_an_asset_server_is_refused() {
        let project = tempfile::tempdir().unwrap();

        let error = super::cook(
            |_| {},
            ContentSettings {
                registry: "",
                project_root: project.path().to_str(),
            },
            &project.path().join("cooked"),
        )
        .unwrap_err();

        assert!(error.to_string().contains("AssetServer"), "{error:#}");
    }
}
