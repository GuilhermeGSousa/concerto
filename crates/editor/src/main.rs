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

const USAGE: &str = "\
Usage: concerto-editor [--project <directory>] [--decorated]
       concerto-editor --headless --mcp [--project <directory>] [--viewport <width>x<height>]

  --headless   Run without a window or panels. Requires --mcp.
  --mcp        Serve the editor's MCP tools over stdin/stdout. Requires --headless.
  --viewport   The headless viewport's render size (default 1280x720).";

fn main() -> anyhow::Result<()> {
    let mut editor = EditorPlugin::default();
    let mut mcp = false;
    let mut args = std::env::args_os().skip(1);
    while let Some(argument) = args.next() {
        match argument {
            flag if flag == "--project" => {
                editor.project = Some(
                    args.next()
                        .ok_or_else(|| anyhow::anyhow!("--project requires a directory"))?
                        .into(),
                )
            }
            flag if flag == "--decorated" => editor.decorated = true,
            flag if flag == "--headless" => editor.headless = true,
            flag if flag == "--mcp" => mcp = true,
            flag if flag == "--viewport" => {
                let size = args
                    .next()
                    .and_then(|size| size.into_string().ok())
                    .ok_or_else(|| anyhow::anyhow!("--viewport requires <width>x<height>"))?;
                editor.viewport_size = parse_size(&size).ok_or_else(|| {
                    anyhow::anyhow!("--viewport expects e.g. 1280x720, got {size}")
                })?;
            }
            flag if flag == "--help" || flag == "-h" => {
                println!("{USAGE}");
                return Ok(());
            }
            _ => anyhow::bail!(USAGE),
        }
    }
    // Headless has no way in but MCP, and MCP needs the stdio runner that only
    // a headless editor uses. Attaching to a windowed editor is future work.
    anyhow::ensure!(
        editor.headless == mcp,
        "--headless and --mcp go together.\n\n{USAGE}"
    );
    // Logs go to stderr, which keeps stdout clean for the protocol.
    env_logger::init();

    let headless = editor.headless;
    let mut app = App::new();
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(AssetManagerPlugin)
        .register_plugin(TimePlugin);
    if !headless {
        app.register_plugin(concerto_window::plugin::WindowPlugin);
    }
    app.register_plugin(TransformPlugin)
        .register_plugin(concerto_render::plugin::RenderPlugin)
        .register_plugin(DebugGizmosPlugin)
        .register_plugin(ShadowPipelinePlugin)
        .register_plugin(MaterialPlugin::<StandardMaterial>::default())
        .register_plugin(concerto_world_grid::plugin::WorldGridPlugin)
        .register_plugin(concerto_scene::plugin::ScenePlugin)
        .register_plugin(editor);
    if headless {
        serve_mcp(&mut app)?;
    } else {
        // Last: systems run in registration order, and the UI's layout pass is
        // the end of that order. Registering it before the panels would lay out
        // what they built on the previous frame.
        app.register_plugin(concerto_ui::plugin::UIPlugin);
    }
    app.run();
    Ok(())
}

#[cfg(feature = "mcp")]
fn serve_mcp(app: &mut App) -> anyhow::Result<()> {
    app.register_plugin(concerto_editor::mcp::EditorMcpPlugin)
        .register_plugin(concerto_mcp::McpRunnerPlugin {
            identity: concerto_editor::mcp::identity(),
        });
    Ok(())
}

#[cfg(not(feature = "mcp"))]
fn serve_mcp(_: &mut App) -> anyhow::Result<()> {
    anyhow::bail!("this editor was built without the `mcp` feature")
}

fn parse_size(size: &str) -> Option<[u32; 2]> {
    let (width, height) = size.split_once('x')?;
    let size = [width.parse().ok()?, height.parse().ok()?];
    size.iter().all(|&side| side > 0).then_some(size)
}
