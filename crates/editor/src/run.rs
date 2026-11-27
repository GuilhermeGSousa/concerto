use std::{ffi::OsString, path::PathBuf};

use concerto_app::{
    App,
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
};

use crate::EditorPlugin;

const MCP_SIZE: (u32, u32) = (1600, 900);

fn usage(program: Option<OsString>) -> String {
    let name = program
        .as_deref()
        .and_then(|program| std::path::Path::new(program).file_stem())
        .map_or_else(|| "editor".into(), |stem| stem.to_string_lossy());
    format!(
        "Usage: {name} [--project <directory>] [--decorated] [--mcp [--mcp-size <width>x<height>]] [-h | --help]"
    )
}

pub(crate) struct Options {
    pub(crate) project: PathBuf,
    pub(crate) decorated: bool,
    pub(crate) mcp: Option<(u32, u32)>,
}

#[derive(Debug)]
pub(crate) enum ArgsError {
    Help,
    Usage(String),
}

fn parse_size(size: &OsString) -> Option<(u32, u32)> {
    let (width, height) = size.to_str()?.split_once('x')?;
    let parse = |side: &str| {
        side.bytes()
            .all(|byte| byte.is_ascii_digit())
            .then(|| side.parse::<u32>().ok())
            .flatten()
            .filter(|&side| side > 0)
    };
    Some((parse(width)?, parse(height)?))
}

pub(crate) fn parse_args(
    args: impl IntoIterator<Item = OsString>,
    default_root: PathBuf,
) -> Result<Options, ArgsError> {
    let mut project = default_root;
    let mut decorated = false;
    let mut mcp = false;
    let mut mcp_size = None;
    let mut args = args.into_iter();
    while let Some(argument) = args.next() {
        if argument == "--project" {
            project = args
                .next()
                .ok_or_else(|| ArgsError::Usage("--project requires a directory".to_string()))?
                .into();
        } else if argument == "--decorated" {
            decorated = true;
        } else if argument == "--mcp" {
            mcp = true;
        } else if argument == "--mcp-size" {
            mcp_size = Some(args.next().as_ref().and_then(parse_size).ok_or_else(|| {
                ArgsError::Usage("--mcp-size requires a size such as 1600x900".to_string())
            })?);
        } else if argument == "--help" || argument == "-h" {
            return Err(ArgsError::Help);
        } else {
            return Err(ArgsError::Usage(format!(
                "unknown argument {}",
                argument.to_string_lossy()
            )));
        }
    }
    if mcp_size.is_some() && !mcp {
        return Err(ArgsError::Usage("--mcp-size requires --mcp".to_string()));
    }
    Ok(Options {
        project,
        decorated,
        mcp: mcp.then(|| mcp_size.unwrap_or(MCP_SIZE)),
    })
}

pub(crate) fn apply_plugins(app: &mut App, plugins: &[fn(&mut App)]) {
    for plugin in plugins {
        plugin(app);
    }
}

pub(crate) fn register_editor(
    app: &mut App,
    recipe: fn(&mut App),
    project: PathBuf,
    decorated: bool,
    plugins: &[fn(&mut App)],
) {
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(AssetManagerPlugin)
        .register_plugin(TimePlugin)
        .register_plugin(concerto_window::plugin::WindowPlugin)
        .register_plugin(TransformPlugin)
        .register_plugin(concerto_render::plugin::RenderPlugin)
        .register_plugin(EditorPlugin {
            project: Some(project),
            decorated,
            recipe,
        })
        .register_plugin(concerto_ui::plugin::UIPlugin)
        .insert_resource(crate::hosted::FinishBuildsOffThread);
    apply_plugins(app, plugins);
}

/// Opens the editor for the game whose recipe is `recipe`, on the project at `project_root`.
///
/// The `--project` argument names another project. With `--mcp`, the editor serves MCP on
/// stdin and stdout instead of opening a window.
pub fn run(recipe: fn(&mut App), project_root: impl Into<PathBuf>) {
    run_with(recipe, project_root, &[]);
}

/// Runs the editor with `plugins` added to its `App`, after `EditorPlugin`.
pub fn run_with(recipe: fn(&mut App), project_root: impl Into<PathBuf>, plugins: &[fn(&mut App)]) {
    let _ = env_logger::try_init();
    let mut arguments = std::env::args_os();
    let program = arguments.next();
    let options = match parse_args(arguments, project_root.into()) {
        Ok(options) => options,
        Err(ArgsError::Help) => {
            println!("{}", usage(program));
            return;
        }
        Err(ArgsError::Usage(message)) => {
            eprintln!("{message}\n{}", usage(program));
            std::process::exit(2);
        }
    };
    let plugins = plugins.to_vec();
    match options.mcp {
        Some(size) => {
            concerto_mcp::host::run(crate::mcp::editor(recipe, options.project, plugins), size)
        }
        None => concerto_winit::run(move |app| {
            register_editor(app, recipe, options.project, options.decorated, &plugins);
        }),
    }
}

#[cfg(test)]
mod tests {
    use concerto_app::{
        App,
        main_schedule::MainSchedulePlugin,
        plugins::{AssetManagerPlugin, TimePlugin},
    };
    use concerto_ecs::Resource;
    use concerto_window::plugin::WindowPlugin;

    use super::apply_plugins;
    use crate::{
        EditorPlugin,
        asset_editor::{AssetEditorAppExt, AssetEditorRegistry},
        hosted::testing::{PlainAsset, PlainEditor, plain_recipe},
    };

    #[derive(Resource)]
    struct Applied(Vec<&'static str>);

    fn first_extension(app: &mut App) {
        app.register_asset_editor::<PlainAsset, PlainEditor>()
            .expect("the editor's registry is in place");
        app.insert_resource(Applied(vec!["first"]));
    }

    fn second_extension(app: &mut App) {
        app.get_resource_mut::<Applied>()
            .expect("the first extension ran")
            .0
            .push("second");
    }

    #[test]
    fn plugins_are_applied_in_order_and_reach_what_the_editor_plugin_registered() {
        let mut app = App::new();
        app.register_plugin(MainSchedulePlugin)
            .register_plugin(AssetManagerPlugin)
            .register_plugin(TimePlugin)
            .register_plugin(WindowPlugin)
            .register_plugin(EditorPlugin {
                project: None,
                decorated: false,
                recipe: plain_recipe,
            });

        apply_plugins(&mut app, &[first_extension, second_extension]);

        assert_eq!(
            app.get_resource::<Applied>().unwrap().0,
            ["first", "second"]
        );
        assert!(
            app.get_resource::<AssetEditorRegistry>()
                .unwrap()
                .editor_kind("PlainAsset")
                .is_some()
        );
    }
}
