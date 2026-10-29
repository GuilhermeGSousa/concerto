use std::{ffi::OsString, path::PathBuf};

use concerto_app::{
    App,
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
};

use crate::{
    EditorPlugin,
    reload::{ReloadSetup, ReloadTarget, RestartSlot, Session, SessionRestore},
};

const MCP_SIZE: (u32, u32) = (1600, 900);

/// The editor crate a game's `main` lives in, as `editor_crate!()` names it at compile time.
#[derive(Clone, Copy, Debug)]
pub struct EditorCrate {
    /// The editor crate's directory, holding its `Cargo.toml`.
    pub manifest_dir: &'static str,
    /// The editor crate's package name.
    pub package: &'static str,
    /// The name of the binary the editor runs as.
    pub bin: &'static str,
    /// The cargo that built the editor.
    pub cargo: &'static str,
}

impl EditorCrate {
    /// The game's directory, the parent of the editor crate's.
    pub fn project_root(&self) -> PathBuf {
        let manifest_dir = std::path::Path::new(self.manifest_dir);
        manifest_dir.parent().unwrap_or(manifest_dir).to_path_buf()
    }
}

/// Names the calling editor crate, for `concerto_editor::run`.
#[macro_export]
macro_rules! editor_crate {
    () => {
        $crate::EditorCrate {
            manifest_dir: env!("CARGO_MANIFEST_DIR"),
            package: env!("CARGO_PKG_NAME"),
            bin: env!("CARGO_BIN_NAME"),
            cargo: env!("CARGO"),
        }
    };
}

fn usage(program: Option<OsString>) -> String {
    let name = program
        .as_deref()
        .and_then(|program| std::path::Path::new(program).file_stem())
        .map_or_else(|| "editor".into(), |stem| stem.to_string_lossy());
    format!(
        "Usage: {name} [--project <directory>] [--decorated] [--restore-session <directory>] [--mcp [--mcp-size <width>x<height>]] [-h | --help]"
    )
}

pub(crate) struct Options {
    pub(crate) project: PathBuf,
    pub(crate) decorated: bool,
    pub(crate) mcp: Option<(u32, u32)>,
    pub(crate) restore: Option<PathBuf>,
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
    let mut restore = None;
    let mut args = args.into_iter();
    while let Some(argument) = args.next() {
        if argument == "--project" {
            project = args
                .next()
                .ok_or_else(|| ArgsError::Usage("--project requires a directory".to_string()))?
                .into();
        } else if argument == "--restore-session" {
            restore = Some(
                args.next()
                    .ok_or_else(|| {
                        ArgsError::Usage("--restore-session requires a directory".to_string())
                    })?
                    .into(),
            );
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
    if restore.is_some() && mcp {
        return Err(ArgsError::Usage(
            "--restore-session cannot be used with --mcp".to_string(),
        ));
    }
    Ok(Options {
        project,
        decorated,
        mcp: mcp.then(|| mcp_size.unwrap_or(MCP_SIZE)),
        restore,
    })
}

pub(crate) fn register_editor(
    app: &mut App,
    recipe: fn(&mut App),
    project: PathBuf,
    decorated: bool,
    reload: ReloadSetup,
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
            reload,
        })
        .register_plugin(concerto_ui::plugin::UIPlugin);
}

/// Opens the editor for the game whose recipe is `recipe`, on the project above `editor`'s crate unless `--project` names another; `--mcp` serves it over MCP on stdin and stdout instead of opening a window.
pub fn run(recipe: fn(&mut App), editor: EditorCrate) {
    let _ = env_logger::try_init();
    let mut arguments = std::env::args_os();
    let program = arguments.next();
    let original: Vec<OsString> = arguments.collect();
    let options = match parse_args(original.iter().cloned(), editor.project_root()) {
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
    match options.mcp {
        Some(size) => concerto_mcp::host::run(crate::mcp::editor(recipe, options.project), size),
        None => {
            let restart = RestartSlot::default();
            let (restore, restore_error) = match &options.restore {
                None => (None, None),
                Some(dir) => match Session::read(dir) {
                    Ok(session) => (Some(SessionRestore::new(session, dir.clone())), None),
                    Err(error) => (
                        None,
                        Some(format!(
                            "Could not restore the session in {}: {error:#}",
                            dir.display()
                        )),
                    ),
                },
            };
            let reload = ReloadSetup {
                target: std::env::current_exe()
                    .map_err(|error| format!("Reload cannot find the running editor: {error}"))
                    .and_then(|exe| ReloadTarget::locate(&editor, &exe)),
                restart: restart.clone(),
                restore,
                restore_error,
            };
            concerto_winit::run(move |app| {
                register_editor(app, recipe, options.project, options.decorated, reload);
            });
            if let Some(restart) = restart.take() {
                crate::reload::replace_process(restart, &original);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsString, path::PathBuf};

    use super::{ArgsError, EditorCrate, parse_args, usage};

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
    fn without_mcp_the_editor_opens_a_window() {
        assert_eq!(
            parse_args(args(&["--decorated"]), PathBuf::new())
                .unwrap()
                .mcp,
            None
        );
    }

    #[test]
    fn mcp_serves_at_the_default_size_and_ignores_decorated() {
        let options = parse_args(args(&["--mcp", "--decorated"]), PathBuf::new()).unwrap();
        assert_eq!(options.mcp, Some((1600, 900)));
    }

    #[test]
    fn mcp_size_sets_the_size_in_either_order() {
        for list in [
            ["--mcp", "--mcp-size", "800x600"],
            ["--mcp-size", "800x600", "--mcp"],
        ] {
            assert_eq!(
                parse_args(args(&list), PathBuf::new()).unwrap().mcp,
                Some((800, 600))
            );
        }
    }

    #[test]
    fn a_bad_or_lone_mcp_size_is_an_error() {
        for list in [
            &["--mcp", "--mcp-size"][..],
            &["--mcp", "--mcp-size", "800"],
            &["--mcp", "--mcp-size", "800x"],
            &["--mcp", "--mcp-size", "0x600"],
            &["--mcp", "--mcp-size", "800x600x2"],
            &["--mcp", "--mcp-size", "-800x600"],
            &["--mcp-size", "800x600"],
        ] {
            assert!(
                matches!(
                    parse_args(args(list), PathBuf::new()),
                    Err(ArgsError::Usage(_))
                ),
                "{list:?}"
            );
        }
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
    fn the_editor_crate_names_the_project_above_it() {
        let editor = EditorCrate {
            manifest_dir: "/games/my-game/editor",
            package: "my-game-editor",
            bin: "my-game-editor",
            cargo: "/usr/bin/cargo",
        };
        assert_eq!(editor.project_root(), PathBuf::from("/games/my-game"));
    }

    #[test]
    fn the_usage_names_the_running_binary() {
        assert_eq!(
            usage(Some(OsString::from("/game/target/debug/my-game-editor"))),
            "Usage: my-game-editor [--project <directory>] [--decorated] [--restore-session <directory>] [--mcp [--mcp-size <width>x<height>]] [-h | --help]"
        );
        assert_eq!(
            usage(None),
            "Usage: editor [--project <directory>] [--decorated] [--restore-session <directory>] [--mcp [--mcp-size <width>x<height>]] [-h | --help]"
        );
    }

    #[test]
    fn restore_session_is_read() {
        let options = parse_args(
            args(&["--project", "copy", "--restore-session", "sessions/a"]),
            PathBuf::from("game"),
        )
        .unwrap();
        assert_eq!(options.restore, Some(PathBuf::from("sessions/a")));
        assert_eq!(options.project, PathBuf::from("copy"));
        assert_eq!(
            parse_args(args(&[]), PathBuf::from("game"))
                .unwrap()
                .restore,
            None
        );
    }

    #[test]
    fn restore_session_requires_a_directory() {
        assert!(matches!(
            parse_args(args(&["--restore-session"]), PathBuf::new()),
            Err(ArgsError::Usage(_))
        ));
    }

    #[test]
    fn restore_session_is_refused_with_mcp() {
        assert!(matches!(
            parse_args(
                args(&["--mcp", "--restore-session", "sessions/a"]),
                PathBuf::new()
            ),
            Err(ArgsError::Usage(_))
        ));
    }
}
