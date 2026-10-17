use std::{ffi::OsString, path::PathBuf};

use concerto_app::{
    App,
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
};

use crate::EditorPlugin;

fn usage(program: Option<OsString>) -> String {
    let name = program
        .as_deref()
        .and_then(|program| std::path::Path::new(program).file_stem())
        .map_or_else(|| "editor".into(), |stem| stem.to_string_lossy());
    format!("Usage: {name} [--project <directory>] [--decorated] [-h | --help]")
}

pub(crate) struct Options {
    pub(crate) project: PathBuf,
    pub(crate) decorated: bool,
}

#[derive(Debug)]
pub(crate) enum ArgsError {
    Help,
    Usage(String),
}

pub(crate) fn parse_args(
    args: impl IntoIterator<Item = OsString>,
    default_root: PathBuf,
) -> Result<Options, ArgsError> {
    let mut project = default_root;
    let mut decorated = false;
    let mut args = args.into_iter();
    while let Some(argument) = args.next() {
        if argument == "--project" {
            project = args
                .next()
                .ok_or_else(|| ArgsError::Usage("--project requires a directory".to_string()))?
                .into();
        } else if argument == "--decorated" {
            decorated = true;
        } else if argument == "--help" || argument == "-h" {
            return Err(ArgsError::Help);
        } else {
            return Err(ArgsError::Usage(format!(
                "unknown argument {}",
                argument.to_string_lossy()
            )));
        }
    }
    Ok(Options { project, decorated })
}

/// Opens the editor for the game whose recipe is `recipe`, on the project at `project_root` unless `--project` names another.
pub fn run(recipe: fn(&mut App), project_root: impl Into<PathBuf>) {
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
    concerto_winit::run(move |app| {
        app.register_plugin(MainSchedulePlugin)
            .register_plugin(AssetManagerPlugin)
            .register_plugin(TimePlugin)
            .register_plugin(concerto_window::plugin::WindowPlugin)
            .register_plugin(TransformPlugin)
            .register_plugin(concerto_render::plugin::RenderPlugin)
            .register_plugin(EditorPlugin {
                project: Some(options.project),
                decorated: options.decorated,
                recipe,
            })
            .register_plugin(concerto_ui::plugin::UIPlugin);
    });
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsString, path::PathBuf};

    use super::{ArgsError, parse_args, usage};

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn with_no_arguments_the_default_root_is_opened() {
        let options = parse_args(args(&[]), PathBuf::from("game")).unwrap();
        assert_eq!(options.project, PathBuf::from("game"));
        assert!(!options.decorated);
    }

    #[test]
    fn project_overrides_the_default_root() {
        let options = parse_args(args(&["--project", "copy"]), PathBuf::from("game")).unwrap();
        assert_eq!(options.project, PathBuf::from("copy"));
    }

    #[test]
    fn decorated_is_read() {
        assert!(
            parse_args(args(&["--decorated"]), PathBuf::from("game"))
                .unwrap()
                .decorated
        );
    }

    #[test]
    fn help_and_bad_arguments_are_errors() {
        assert!(matches!(
            parse_args(args(&["--help"]), PathBuf::new()),
            Err(ArgsError::Help)
        ));
        assert!(matches!(
            parse_args(args(&["-h"]), PathBuf::new()),
            Err(ArgsError::Help)
        ));
        assert!(matches!(
            parse_args(args(&["--project"]), PathBuf::new()),
            Err(ArgsError::Usage(_))
        ));
        assert!(matches!(
            parse_args(args(&["--nope"]), PathBuf::new()),
            Err(ArgsError::Usage(_))
        ));
    }

    #[test]
    fn the_usage_names_the_running_binary() {
        assert_eq!(
            usage(Some(OsString::from("/game/target/debug/my-game-editor"))),
            "Usage: my-game-editor [--project <directory>] [--decorated] [-h | --help]"
        );
        assert_eq!(
            usage(None),
            "Usage: editor [--project <directory>] [--decorated] [-h | --help]"
        );
    }
}
