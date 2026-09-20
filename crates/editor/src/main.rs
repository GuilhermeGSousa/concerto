use app::{
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
    App,
};
use debug_gizmos::DebugGizmosPlugin;
use editor::EditorPlugin;
use render::{
    assets::material::StandardMaterial, material_plugin::MaterialPlugin,
    shadow_pipeline::ShadowPipelinePlugin,
};

fn main() -> anyhow::Result<()> {
    const USAGE: &str = "Usage: editor [--project <directory>] [--decorated]";
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
        .register_plugin(window::plugin::WindowPlugin)
        .register_plugin(TransformPlugin)
        .register_plugin(render::plugin::RenderPlugin)
        .register_plugin(DebugGizmosPlugin)
        .register_plugin(ShadowPipelinePlugin)
        .register_plugin(MaterialPlugin::<StandardMaterial>::default())
        .register_plugin(world_grid::plugin::WorldGridPlugin)
        .register_plugin(scene::plugin::ScenePlugin)
        .register_plugin(EditorPlugin { project, decorated })
        // Last: systems run in registration order, and the UI's layout pass is
        // the end of that order. Registering it before the panels would lay out
        // what they built on the previous frame.
        .register_plugin(ui::plugin::UIPlugin);
    app.run();
    Ok(())
}
