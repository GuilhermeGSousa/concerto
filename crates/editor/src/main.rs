use concerto_app::{
    App,
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
};
use concerto_debug_gizmos::DebugGizmosPlugin;
use concerto_editor::EditorPlugin;
use concerto_render::{
    assets::material::StandardMaterial, material_plugin::MaterialPlugin,
    shadow_pipeline::ShadowPipelinePlugin,
};

fn main() -> anyhow::Result<()> {
    const USAGE: &str = "Usage: concerto-editor [--project <directory>] [--decorated]";
    let mut project = None;
    let mut decorated = false;
    let mut args = std::env::args_os().skip(1);
    while let Some(argument) = args.next() {
        match argument {
            flag if flag == "--project" => {
                project = Some(
                    args.next()
                        .ok_or_else(|| anyhow::anyhow!("--project requires a directory"))?
                        .into(),
                )
            }
            flag if flag == "--decorated" => decorated = true,
            flag if flag == "--help" || flag == "-h" => {
                println!("{USAGE}");
                return Ok(());
            }
            _ => anyhow::bail!(USAGE),
        }
    }
    env_logger::init();
    let mut app = App::new();
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(AssetManagerPlugin)
        .register_plugin(TimePlugin)
        .register_plugin(concerto_window::plugin::WindowPlugin)
        .register_plugin(TransformPlugin)
        .register_plugin(concerto_render::plugin::RenderPlugin)
        .register_plugin(DebugGizmosPlugin)
        .register_plugin(ShadowPipelinePlugin)
        .register_plugin(MaterialPlugin::<StandardMaterial>::default())
        .register_plugin(concerto_world_grid::plugin::WorldGridPlugin)
        .register_plugin(concerto_scene::plugin::ScenePlugin)
        .register_plugin(EditorPlugin { project, decorated })
        .register_plugin(concerto_ui::plugin::UIPlugin);
    app.run();
    Ok(())
}
