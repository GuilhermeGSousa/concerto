use std::fmt;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Component, Path, PathBuf};
use std::process::{self, Stdio};

use anyhow::{bail, ensure, Context, Result};
use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, TableLike, Value};

const VERSION: &str = env!("CARGO_PKG_VERSION");

const GAME_MANIFEST_TEMPLATE: &str = include_str!("../templates/game/Cargo.toml.template");

const GAME_FILES: &[(&str, &str)] = &[
    ("Cargo.toml", GAME_MANIFEST_TEMPLATE),
    ("src/lib.rs", include_str!("../templates/game/src/lib.rs")),
    ("src/main.rs", include_str!("../templates/game/src/main.rs")),
    ("build.rs", include_str!("../templates/game/build.rs")),
    (
        "content/.gitkeep",
        include_str!("../templates/game/content/.gitkeep"),
    ),
    ("index.html", include_str!("../templates/game/index.html")),
    ("Trunk.toml", include_str!("../templates/game/Trunk.toml")),
    (".gitignore", include_str!("../templates/game/gitignore")),
];

const EDITOR_MANIFEST: &str = include_str!("../templates/editor/Cargo.toml.template");
const EDITOR_MAIN: &str = include_str!("../templates/editor/src/main.rs");
const EDITOR_MAIN_WITH_PLUGINS: &str = include_str!("../templates/editor/src/main_with_plugins.rs");

const PLUGIN_KEYS: &[&str] = &["package", "install", "version", "path"];
const ARRAY_WIDTH: usize = 60;

const WASM_TARGET: &str = "wasm32-unknown-unknown";
const COOK_MARKER: &str = ".concerto-cook";
const TRUNK_MISSING: &str = "trunk is not on PATH; install it with `cargo install trunk`";

const OWN_WORKSPACE: &str = "\n[workspace]\nmembers = [\".\", \"editor\"]\n\n[profile.dev]\nopt-level = 1\n\n[profile.dev.package.\"*\"]\nopt-level = 3";

const KEYWORDS: &[&str] = &[
    "abstract", "as", "async", "await", "become", "box", "break", "const", "continue", "crate",
    "do", "dyn", "else", "enum", "extern", "false", "final", "fn", "for", "gen", "if", "impl",
    "in", "let", "loop", "macro", "match", "mod", "move", "mut", "override", "priv", "pub", "ref",
    "return", "self", "Self", "static", "struct", "super", "trait", "true", "try", "type",
    "typeof", "unsafe", "unsized", "use", "virtual", "where", "while", "yield",
];

const RESERVED: &[&str] = &[
    "test",
    "build",
    "deps",
    "examples",
    "incremental",
    "concerto",
    "concerto_editor",
    "concerto_asset_build",
    "concerto_animation_editor",
];

/// How to run the tool.
pub const USAGE: &str = "usage: cargo concerto new <dir> [--engine <path>]\n       cargo concerto editor <dir> [--update] [--engine <path>]\n       cargo concerto build [<dir>] [--cook] [--release] [--target wasm32-unknown-unknown] [--public-url <url>] [--dist <dir>]\n       cargo concerto run [<dir>] [--cook] [--release] [-- <game args>]";

/// Where a generated project's engine dependencies come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Engine {
    Published,
    Checkout(PathBuf),
}

/// A parsed command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    New(PathBuf, Engine),
    Editor(PathBuf, Engine),
    UpdateEditor(PathBuf, Option<Engine>),
    Build(BuildOptions),
    Run(RunOptions),
}

/// What `build` was asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildOptions {
    pub dir: PathBuf,
    pub cook: bool,
    pub release: bool,
    pub wasm: bool,
    pub public_url: Option<String>,
    pub dist: Option<PathBuf>,
}

/// What `run` was asked for: a native build, and the arguments the game is run with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOptions {
    pub build: BuildOptions,
    pub args: Vec<String>,
}

/// The steps `build` runs, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildPlan {
    pub steps: Vec<BuildStep>,
}

/// One step of a [`BuildPlan`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildStep {
    Cargo { args: Vec<String> },
    Cook { out: PathBuf },
    Trunk { dir: PathBuf, args: Vec<String> },
    Overlay { from: PathBuf, onto: OverlayTarget },
    Run { args: Vec<String> },
}

/// Where an overlay step copies the cooked files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverlayTarget {
    BuiltContent,
    Dir(PathBuf),
}

/// The files a command wrote, the manifests it edited, and what it has to say.
#[derive(Debug, Default)]
pub struct Report {
    pub written: Vec<PathBuf>,
    pub edited: Vec<PathBuf>,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for path in &self.written {
            writeln!(f, "wrote {}", path.display())?;
        }
        for path in &self.edited {
            writeln!(f, "edited {}", path.display())?;
        }
        for note in &self.notes {
            writeln!(f, "note: {note}")?;
        }
        for warning in &self.warnings {
            writeln!(f, "warning: {warning}")?;
        }
        Ok(())
    }
}

/// Parses the arguments after the program name, skipping the `concerto` Cargo passes first.
pub fn parse<I, S>(args: I) -> Result<Command>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut args = args.into_iter().map(Into::into).peekable();
    if args.peek().is_some_and(|arg| arg == "concerto") {
        args.next();
    }
    let command = args.next().context("missing a command")?;
    if command == "build" || command == "run" {
        return parse_build(&command, args);
    }
    ensure!(
        command == "new" || command == "editor",
        "unknown command `{command}`"
    );
    let mut dir = None;
    let mut engine = None;
    let mut update = false;
    while let Some(arg) = args.next() {
        if arg == "--engine" {
            let path = args.next().context("--engine needs a path")?;
            engine = Some(Engine::Checkout(PathBuf::from(path)));
        } else if arg == "--update" && command == "editor" {
            update = true;
        } else if arg.starts_with('-') {
            bail!("unknown option `{arg}`");
        } else if dir.is_none() {
            dir = Some(PathBuf::from(arg));
        } else {
            bail!("unexpected argument `{arg}`");
        }
    }
    let dir = dir.with_context(|| format!("`{command}` needs a directory"))?;
    if update {
        return Ok(Command::UpdateEditor(dir, engine));
    }
    let engine = engine.unwrap_or(Engine::Published);
    Ok(match command.as_str() {
        "new" => Command::New(dir, engine),
        _ => Command::Editor(dir, engine),
    })
}

/// Runs a parsed command.
pub fn run(command: Command) -> Result<Report> {
    match command {
        Command::New(dir, engine) => new_project(dir, engine),
        Command::Editor(dir, engine) => add_editor(dir, engine),
        Command::UpdateEditor(dir, engine) => update_editor(dir, engine),
        Command::Build(options) => build(options),
        Command::Run(options) => run_game(options),
    }
}

fn parse_build(command: &str, mut args: impl Iterator<Item = String>) -> Result<Command> {
    let run = command == "run";
    let mut game_args = Vec::new();
    let mut dir = None;
    let mut cook = false;
    let mut release = false;
    let mut wasm = false;
    let mut public_url = None;
    let mut dist = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--" if run => {
                game_args.extend(args.by_ref());
                break;
            }
            "--target" | "--public-url" | "--dist" if run => {
                bail!("{arg} applies to `build`; `run` builds and runs natively")
            }
            "--cook" => cook = true,
            "--release" => release = true,
            "--target" => {
                let target = args.next().context("--target needs a target triple")?;
                ensure!(
                    target == WASM_TARGET,
                    "`build` builds natively or for --target {WASM_TARGET}, not `{target}`"
                );
                wasm = true;
            }
            "--public-url" => public_url = Some(args.next().context("--public-url needs a URL")?),
            "--dist" => {
                dist = Some(PathBuf::from(
                    args.next().context("--dist needs a directory")?,
                ))
            }
            "--engine" => bail!("--engine applies to `new` and `editor`, not `{command}`"),
            _ if arg.starts_with('-') => bail!("unknown option `{arg}`"),
            _ if dir.is_none() => dir = Some(PathBuf::from(arg)),
            _ => bail!("unexpected argument `{arg}`"),
        }
    }
    ensure!(
        wasm || public_url.is_none(),
        "--public-url applies only with --target {WASM_TARGET}"
    );
    ensure!(
        wasm || dist.is_none(),
        "--dist applies only with --target {WASM_TARGET}"
    );
    let options = BuildOptions {
        dir: dir.unwrap_or_else(|| PathBuf::from(".")),
        cook,
        release,
        wasm,
        public_url,
        dist,
    };
    Ok(if run {
        Command::Run(RunOptions {
            build: options,
            args: game_args,
        })
    } else {
        Command::Build(options)
    })
}

/// Turns `options` into the steps that build `package`, whose Cargo target directory is `target_dir`.
pub fn plan_build(options: &BuildOptions, package: &str, target_dir: &Path) -> BuildPlan {
    let cooked = target_dir.join("cooked").join(package);
    let mut steps = Vec::new();
    if options.cook || !options.wasm {
        let mut args = vec![
            "build".to_string(),
            "--manifest-path".to_string(),
            options.dir.join("Cargo.toml").display().to_string(),
            "-p".to_string(),
            package.to_string(),
            "--bin".to_string(),
            package.to_string(),
        ];
        if options.release {
            args.push("--release".to_string());
        }
        args.push("--message-format=json-render-diagnostics".to_string());
        steps.push(BuildStep::Cargo { args });
    }
    if options.cook {
        steps.push(BuildStep::Cook {
            out: cooked.clone(),
        });
    }
    if options.wasm {
        let dist = options
            .dist
            .clone()
            .unwrap_or_else(|| options.dir.join("dist"));
        let mut args = vec!["build".to_string()];
        if options.release {
            args.push("--release".to_string());
        }
        if let Some(url) = &options.public_url {
            args.push("--public-url".to_string());
            args.push(url.clone());
        }
        args.push("--dist".to_string());
        args.push(dist.display().to_string());
        steps.push(BuildStep::Trunk {
            dir: options.dir.clone(),
            args,
        });
        if options.cook {
            steps.push(BuildStep::Overlay {
                from: cooked,
                onto: OverlayTarget::Dir(dist),
            });
        }
    } else if options.cook {
        steps.push(BuildStep::Overlay {
            from: cooked,
            onto: OverlayTarget::BuiltContent,
        });
    }
    BuildPlan { steps }
}

/// The steps of [`plan_build`] for `options`, then one that runs the built `package` with `args`.
pub fn plan_run(
    options: &BuildOptions,
    args: &[String],
    package: &str,
    target_dir: &Path,
) -> BuildPlan {
    let mut plan = plan_build(options, package, target_dir);
    plan.steps.push(BuildStep::Run {
        args: args.to_vec(),
    });
    plan
}

/// Builds the game in `options.dir`, cooking and staging its derived data when asked.
pub fn build(options: BuildOptions) -> Result<Report> {
    execute(options, plan_build)
}

/// Builds the game as [`build`] does, then runs it with `options.args` and waits for it to exit.
pub fn run_game(options: RunOptions) -> Result<Report> {
    execute(options.build, |build, package, target_dir| {
        plan_run(build, &options.args, package, target_dir)
    })
}

fn execute(
    options: BuildOptions,
    plan: impl FnOnce(&BuildOptions, &str, &Path) -> BuildPlan,
) -> Result<Report> {
    let options = BuildOptions {
        dir: absolute(&options.dir)?,
        dist: options.dist.as_deref().map(absolute).transpose()?,
        ..options
    };
    if options.wasm {
        require_trunk("trunk".as_ref())?;
    }
    let package = package_name(&options.dir)?;
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let target_dir = target_directory(&cargo, &options.dir)?;
    let plan = plan(&options, &package, &target_dir);
    let mut report = Report::default();
    let mut executable = None;
    for step in &plan.steps {
        match step {
            BuildStep::Cargo { args } => {
                let mut command = process::Command::new(&cargo);
                command.args(args);
                executable = Some(run_cargo(command, &package)?);
            }
            BuildStep::Cook { out } => {
                let executable = executable
                    .as_ref()
                    .context("the cook needs the executable a cargo step builds")?;
                let mut command = process::Command::new(executable);
                command.arg("--cook").arg(out);
                run_step(command)?;
                report.written.extend(
                    files_under(out)?
                        .into_iter()
                        .filter(|file| file.as_path() != Path::new(COOK_MARKER))
                        .map(|file| out.join(file)),
                );
            }
            BuildStep::Trunk { dir, args } => {
                let mut command = process::Command::new("trunk");
                command.args(args).current_dir(dir);
                run_trunk(command)?;
            }
            BuildStep::Overlay { from, onto } => {
                let onto = match onto {
                    OverlayTarget::Dir(dir) => dir.clone(),
                    OverlayTarget::BuiltContent => executable
                        .as_deref()
                        .and_then(built_content_dir)
                        .context("staging the cook needs the executable a cargo step builds")?,
                };
                overlay(from, &onto, &mut report)?
            }
            BuildStep::Run { args } => {
                let executable = executable
                    .as_ref()
                    .context("running the game needs the executable a cargo step builds")?;
                let mut command = process::Command::new(executable);
                command.args(args);
                run_step(command)?;
            }
        }
    }
    Ok(report)
}

fn package_name(dir: &Path) -> Result<String> {
    let manifest_path = dir.join("Cargo.toml");
    let manifest: DocumentMut = fs::read_to_string(&manifest_path)
        .with_context(|| format!("{} has no Cargo.toml", dir.display()))?
        .parse()
        .with_context(|| format!("{} is not valid TOML", manifest_path.display()))?;
    Ok(manifest
        .get("package")
        .and_then(|package| package.get("name"))
        .and_then(Item::as_str)
        .with_context(|| format!("{} has no package name", manifest_path.display()))?
        .to_string())
}

fn target_directory(cargo: &std::ffi::OsStr, dir: &Path) -> Result<PathBuf> {
    let mut command = process::Command::new(cargo);
    command
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--manifest-path",
        ])
        .arg(dir.join("Cargo.toml"))
        .stderr(Stdio::inherit());
    let shown = command_line(&command);
    let output = command
        .output()
        .with_context(|| format!("could not run `{shown}`"))?;
    ensure!(
        output.status.success(),
        "`{shown}` failed with {}",
        output.status
    );
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout)
        .with_context(|| format!("`{shown}` printed something that is not JSON"))?;
    let target = metadata
        .get("target_directory")
        .and_then(serde_json::Value::as_str)
        .with_context(|| format!("`{shown}` printed no target_directory"))?;
    Ok(PathBuf::from(target))
}

fn command_line(command: &process::Command) -> String {
    std::iter::once(command.get_program())
        .chain(command.get_args())
        .map(|part| part.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

fn run_step(mut command: process::Command) -> Result<()> {
    let shown = command_line(&command);
    let status = command
        .status()
        .with_context(|| format!("could not run `{shown}`"))?;
    ensure!(status.success(), "`{shown}` failed with {status}");
    Ok(())
}

fn run_trunk(mut command: process::Command) -> Result<()> {
    let shown = command_line(&command);
    let status = match command.status() {
        Ok(status) => status,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            bail!(TRUNK_MISSING)
        }
        Err(error) => return Err(error).with_context(|| format!("could not run `{shown}`")),
    };
    ensure!(status.success(), "`{shown}` failed with {status}");
    Ok(())
}

fn require_trunk(trunk: &std::ffi::OsStr) -> Result<()> {
    let mut command = process::Command::new(trunk);
    command
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match command.status() {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => bail!(TRUNK_MISSING),
        Err(error) => {
            Err(error).with_context(|| format!("could not run `{}`", command_line(&command)))
        }
    }
}

fn run_cargo(mut command: process::Command, package: &str) -> Result<PathBuf> {
    let shown = command_line(&command);
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("could not run `{shown}`"))?;
    let stdout = child
        .stdout
        .take()
        .context("cargo's output was not captured")?;
    let mut executable = None;
    for line in BufReader::new(stdout).lines() {
        if let Some(found) = executable_of(&line?, package) {
            executable = Some(found);
        }
    }
    let status = child.wait()?;
    ensure!(status.success(), "`{shown}` failed with {status}");
    executable.with_context(|| format!("`{shown}` reported no executable for the bin `{package}`"))
}

fn executable_of(line: &str, package: &str) -> Option<PathBuf> {
    let message: serde_json::Value = serde_json::from_str(line).ok()?;
    if message.get("reason")?.as_str()? != "compiler-artifact" {
        return None;
    }
    let target = message.get("target")?;
    let is_bin = target
        .get("kind")?
        .as_array()?
        .iter()
        .any(|kind| kind.as_str() == Some("bin"));
    if !is_bin || target.get("name")?.as_str()? != package {
        return None;
    }
    message.get("executable")?.as_str().map(PathBuf::from)
}

fn built_content_dir(executable: &Path) -> Option<PathBuf> {
    let stem = executable.file_stem()?.to_string_lossy();
    Some(executable.parent()?.join(format!("{stem}-content")))
}

fn files_under(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(relative) = pending.pop() {
        let dir = root.join(&relative);
        for entry in fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))? {
            let entry = entry?;
            let path = relative.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                pending.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

fn overlay(from: &Path, onto: &Path, report: &mut Report) -> Result<()> {
    for file in files_under(from)? {
        if file.as_path() == Path::new(COOK_MARKER) {
            continue;
        }
        let target = onto.join(&file);
        if target.is_file() {
            fs::copy(from.join(&file), &target)
                .with_context(|| format!("copying over {}", target.display()))?;
            report.edited.push(target);
        } else {
            report.notes.push(format!(
                "skipped {}: {} has no such file",
                slashed(&file),
                onto.display()
            ));
        }
    }
    Ok(())
}

/// Creates a game project in `dir`, with its editor crate, and makes both workspace members.
pub fn new_project(dir: impl AsRef<Path>, engine: Engine) -> Result<Report> {
    let dir = absolute(dir.as_ref())?;
    let name = dir
        .file_name()
        .and_then(|name| name.to_str())
        .with_context(|| format!("{} has no usable directory name", dir.display()))?
        .to_string();
    validate_name(&name)?;
    if dir.exists() {
        ensure!(
            dir.is_dir() && fs::read_dir(&dir)?.next().is_none(),
            "{} exists and is not empty",
            dir.display()
        );
    }
    let created = !dir.exists();
    let checkout = resolve(engine)?;
    let workspace = workspace_of(&dir)?;
    let own_workspace = if workspace.is_some() {
        ""
    } else {
        OWN_WORKSPACE
    };
    let crate_name = name.replace('-', "_");
    let vars = [
        ("name", name.as_str()),
        ("crate", crate_name.as_str()),
        ("concerto", &dependency(checkout.as_deref(), "", &dir, &dir)),
        (
            "concerto_asset_build",
            &dependency(checkout.as_deref(), "crates/asset-build", &dir, &dir),
        ),
        ("workspace", own_workspace),
    ];
    let manifest: DocumentMut = render(GAME_MANIFEST_TEMPLATE, &vars)
        .parse()
        .context("the game template is not valid TOML")?;
    let game = Game {
        dir: dir.clone(),
        name: name.clone(),
        crate_name: crate_name.clone(),
        plugins: editor_plugins(&manifest, &name)?,
    };
    let mut report = Report::default();
    let written = GAME_FILES
        .iter()
        .try_for_each(|(path, template)| {
            write_file(&dir.join(path), &render(template, &vars), &mut report)
        })
        .and_then(|()| write_editor(&game, checkout.as_deref(), &mut report))
        .and_then(|()| match workspace {
            Some(workspace) => join_workspace(
                &dir,
                workspace,
                &[dir.clone(), dir.join("editor")],
                &mut report,
            ),
            None => Ok(()),
        });
    if let Err(error) = written {
        return Err(match remove_project(&dir, created) {
            Ok(()) => error,
            Err(cleanup) => error.context(format!(
                "and {} could not be cleaned up: {cleanup}",
                dir.display()
            )),
        });
    }
    warn_if_published(checkout.as_deref(), &mut report);
    Ok(report)
}

fn remove_project(dir: &Path, created: bool) -> std::io::Result<()> {
    if created {
        return fs::remove_dir_all(dir);
    }
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            fs::remove_dir_all(path)?;
        } else {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

/// Adds the generated `editor/` crate to the game in `dir` and makes it a workspace member.
pub fn add_editor(dir: impl AsRef<Path>, engine: Engine) -> Result<Report> {
    let dir = absolute(dir.as_ref())?;
    let game = read_game(&dir)?;
    ensure!(
        !dir.join("editor").exists(),
        "{} already exists; --update rewrites its generated files",
        dir.join("editor").display()
    );
    let checkout = resolve(engine)?;
    let workspace = workspace_of(&dir)?;
    let mut report = Report::default();
    let added =
        write_editor(&game, checkout.as_deref(), &mut report).and_then(|()| match workspace {
            Some(workspace) => join_workspace(&dir, workspace, &[dir.join("editor")], &mut report),
            None => declare_own_workspace(&dir, &mut report),
        });
    if let Err(error) = added {
        let editor = dir.join("editor");
        return Err(match fs::remove_dir_all(&editor) {
            Ok(()) => error,
            Err(cleanup) => error.context(format!(
                "and {} could not be removed: {cleanup}",
                editor.display()
            )),
        });
    }
    report.notes.push(format!(
        "the editor calls `{}::game`; the game's library must export `pub fn game(app: &mut App)`",
        game.crate_name
    ));
    warn_if_published(checkout.as_deref(), &mut report);
    Ok(report)
}

/// Rewrites `editor/Cargo.toml` and `editor/src/main.rs` from the game in `dir`; `editor/Cargo.toml` names the engine unless `engine` does.
pub fn update_editor(dir: impl AsRef<Path>, engine: Option<Engine>) -> Result<Report> {
    let dir = absolute(dir.as_ref())?;
    let game = read_game(&dir)?;
    let editor = dir.join("editor");
    ensure!(
        editor.is_dir(),
        "{} does not exist; `cargo concerto editor` without --update creates it",
        editor.display()
    );
    let engine = match engine {
        Some(engine) => engine,
        None => engine_of_editor(&editor)?,
    };
    let checkout = resolve(engine)?;
    let mut report = Report::default();
    write_editor(&game, checkout.as_deref(), &mut report)?;
    warn_if_published(checkout.as_deref(), &mut report);
    Ok(report)
}

struct Game {
    dir: PathBuf,
    name: String,
    crate_name: String,
    plugins: Vec<EditorPlugin>,
}

struct EditorPlugin {
    package: String,
    install: String,
    version: Option<String>,
    path: Option<String>,
}

fn read_game(dir: &Path) -> Result<Game> {
    let manifest_path = dir.join("Cargo.toml");
    let manifest: DocumentMut = fs::read_to_string(&manifest_path)
        .with_context(|| format!("{} has no Cargo.toml", dir.display()))?
        .parse()
        .with_context(|| format!("{} is not valid TOML", manifest_path.display()))?;
    let name = manifest
        .get("package")
        .and_then(|package| package.get("name"))
        .and_then(Item::as_str)
        .with_context(|| format!("{} has no package name", manifest_path.display()))?
        .to_string();
    let crate_name = manifest
        .get("lib")
        .and_then(|lib| lib.get("name"))
        .and_then(Item::as_str)
        .map_or_else(|| name.replace('-', "_"), str::to_string);
    ensure!(
        dir.join("src/lib.rs").is_file(),
        "{} has no library: the editor calls the game's recipe from src/lib.rs",
        dir.display()
    );
    let plugins = editor_plugins(&manifest, &name)
        .with_context(|| format!("the editor plugins in {}", manifest_path.display()))?;
    Ok(Game {
        dir: dir.to_path_buf(),
        name,
        crate_name,
        plugins,
    })
}

fn editor_plugins(manifest: &DocumentMut, game: &str) -> Result<Vec<EditorPlugin>> {
    let Some(list) = manifest
        .get("package")
        .and_then(|package| package.get("metadata"))
        .and_then(|metadata| metadata.get("concerto"))
        .and_then(|concerto| concerto.get("editor"))
        .and_then(|editor| editor.get("plugins"))
    else {
        return Ok(Vec::new());
    };
    let list = list
        .as_array()
        .context("`plugins` must be an array of `{ package, install }` tables")?;
    let mut taken = vec![game.to_string(), "concerto-editor".to_string()];
    let mut plugins = Vec::new();
    for entry in list {
        let plugin = editor_plugin(entry, &taken)?;
        taken.push(plugin.package.clone());
        plugins.push(plugin);
    }
    Ok(plugins)
}

fn editor_plugin(entry: &Value, taken: &[String]) -> Result<EditorPlugin> {
    let shown = entry.to_string();
    let shown = shown.trim();
    let table = entry
        .as_inline_table()
        .with_context(|| format!("`{shown}` is not a `{{ package, install }}` table"))?;
    if let Some((key, _)) = table.iter().find(|(key, _)| !PLUGIN_KEYS.contains(key)) {
        bail!("`{shown}` has an unknown key `{key}`");
    }
    let text = |key: &str| {
        table
            .get(key)
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_string)
                    .with_context(|| format!("`{shown}` has a `{key}` that is not a string"))
            })
            .transpose()
    };
    let package = text("package")?.with_context(|| format!("`{shown}` has no `package`"))?;
    let install = text("install")?.with_context(|| format!("`{shown}` has no `install`"))?;
    let version = text("version")?;
    let path = text("path")?;
    ensure!(
        is_package_name(&package),
        "`{shown}` does not name a Cargo package"
    );
    ensure!(
        !taken.contains(&package),
        "`{shown}` names `{package}`, which the editor already depends on"
    );
    ensure!(
        is_rust_path(&install),
        "`{shown}` has an `install` that is not a Rust path such as `{}::install`",
        package.replace('-', "_")
    );
    ensure!(
        version.is_some() || path.is_some() || engine_crate(&package).is_some(),
        "`{shown}` is not a `concerto-<dir>` package, so it must give `version` or `path`"
    );
    Ok(EditorPlugin {
        package,
        install,
        version,
        path,
    })
}

fn is_rust_path(path: &str) -> bool {
    let segments: Vec<&str> = path.split("::").collect();
    segments.len() > 1 && segments.into_iter().all(is_identifier)
}

fn is_identifier(segment: &str) -> bool {
    segment != "_"
        && !segment.is_empty()
        && !segment.starts_with(|c: char| c.is_ascii_digit())
        && segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !KEYWORDS.contains(&segment)
}

fn engine_crate(package: &str) -> Option<String> {
    package
        .strip_prefix("concerto-")
        .filter(|dir| !dir.is_empty())
        .map(|dir| format!("crates/{dir}"))
}

fn plugin_dependency(
    plugin: &EditorPlugin,
    game: &Path,
    checkout: Option<&Path>,
) -> Result<String> {
    let editor = game.join("editor");
    if let (None, None, Some(crate_dir)) =
        (&plugin.version, &plugin.path, engine_crate(&plugin.package))
    {
        return Ok(dependency(checkout, &crate_dir, &editor, game));
    }
    let mut table = InlineTable::new();
    if let Some(version) = &plugin.version {
        table.insert("version", version.as_str().into());
    }
    if let Some(path) = &plugin.path {
        let written = if Path::new(path).has_root() {
            path.clone()
        } else {
            relative(&editor, &absolute(&game.join(path))?)
        };
        table.insert("path", written.as_str().into());
    }
    table.fmt();
    Ok(table.to_string())
}

fn plugin_list(plugins: &[EditorPlugin]) -> String {
    let installs: Vec<&str> = plugins
        .iter()
        .map(|plugin| plugin.install.as_str())
        .collect();
    let line = installs.join(", ");
    if installs.len() == 1 || line.len() <= ARRAY_WIDTH {
        format!("&[{line}]")
    } else {
        format!(
            "&[\n            {},\n        ]",
            installs.join(",\n            ")
        )
    }
}

fn engine_of_editor(editor: &Path) -> Result<Engine> {
    let manifest = editor.join("Cargo.toml");
    let unknown = || {
        format!(
            "{} does not say which engine it uses; pass --engine <path>",
            manifest.display()
        )
    };
    let document = read_manifest(&manifest)?.with_context(unknown)?;
    let dependency = document
        .get("dependencies")
        .and_then(|dependencies| dependencies.get("concerto-editor"))
        .with_context(unknown)?;
    if dependency.is_str() {
        return Ok(Engine::Published);
    }
    let table = dependency.as_table_like().with_context(unknown)?;
    let Some(path) = table.get("path") else {
        ensure!(
            table.contains_key("version")
                && !["git", "registry"]
                    .iter()
                    .any(|key| table.contains_key(key)),
            unknown()
        );
        return Ok(Engine::Published);
    };
    let path = absolute(&editor.join(path.as_str().with_context(unknown)?))?;
    ensure!(path.ends_with("crates/editor"), unknown());
    let checkout = path.parent().and_then(Path::parent).with_context(unknown)?;
    Ok(Engine::Checkout(checkout.to_path_buf()))
}

fn write_editor(game: &Game, checkout: Option<&Path>, report: &mut Report) -> Result<()> {
    let dir = &game.dir;
    let editor = dependency(checkout, "crates/editor", &dir.join("editor"), dir);
    let mut plugin_dependencies = String::new();
    for plugin in &game.plugins {
        let dependency = plugin_dependency(plugin, dir, checkout)?;
        plugin_dependencies.push_str(&format!("\n{} = {dependency}", plugin.package));
    }
    let plugins = plugin_list(&game.plugins);
    let vars = [
        ("name", game.name.as_str()),
        ("crate", game.crate_name.as_str()),
        ("concerto_editor", editor.as_str()),
        ("plugin_dependencies", plugin_dependencies.as_str()),
        ("plugins", plugins.as_str()),
    ];
    let main = if game.plugins.is_empty() {
        EDITOR_MAIN
    } else {
        EDITOR_MAIN_WITH_PLUGINS
    };
    for (path, template) in [
        ("editor/Cargo.toml", EDITOR_MANIFEST),
        ("editor/src/main.rs", main),
    ] {
        write_file(&dir.join(path), &render(template, &vars), report)?;
    }
    Ok(())
}

fn warn_if_published(checkout: Option<&Path>, report: &mut Report) {
    if checkout.is_none() {
        report.warnings.push(format!(
            "the engine dependencies name version {VERSION} from crates.io; the project builds only against a published release that contains this work, so until one exists pass --engine <path to a Concerto checkout>"
        ));
    }
}

fn validate_name(name: &str) -> Result<()> {
    let crate_name = name.replace('-', "_");
    let valid = is_package_name(name)
        && !KEYWORDS.contains(&crate_name.as_str())
        && !RESERVED.contains(&crate_name.as_str());
    ensure!(
        valid,
        "`{name}` cannot be a package name: use letters, digits, `-` and `_`, do not start with a digit, and do not use a Rust keyword, `test`, a Cargo build directory name (`build`, `deps`, `examples`, `incremental`), or the name of an engine crate the project depends on"
    );
    Ok(())
}

fn is_package_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && !name.starts_with(|c: char| c.is_ascii_digit())
}

fn render(template: &str, vars: &[(&str, &str)]) -> String {
    let mut text = template.replace("\r\n", "\n");
    for (key, value) in vars {
        text = text.replace(&format!("{{{{{key}}}}}"), value);
    }
    format!("{}\n", text.trim_end())
}

fn write_file(path: &Path, contents: &str, report: &mut Report) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, contents).with_context(|| format!("writing {}", path.display()))?;
    report.written.push(path.to_path_buf());
    Ok(())
}

fn resolve(engine: Engine) -> Result<Option<PathBuf>> {
    match engine {
        Engine::Published => Ok(None),
        Engine::Checkout(path) => {
            let path = absolute(&path)?;
            ensure!(
                path.is_dir(),
                "the engine checkout {} does not exist",
                path.display()
            );
            Ok(Some(path))
        }
    }
}

fn dependency(
    checkout: Option<&Path>,
    crate_dir: &str,
    manifest_dir: &Path,
    project: &Path,
) -> String {
    let Some(checkout) = checkout else {
        return Value::from(VERSION).to_string();
    };
    let target = if crate_dir.is_empty() {
        checkout.to_path_buf()
    } else {
        checkout.join(crate_dir)
    };
    let path = if project.starts_with(checkout) {
        relative(manifest_dir, &target)
    } else {
        slashed(&target)
    };
    dependency_value(&path)
}

fn dependency_value(path: &str) -> String {
    let mut table = InlineTable::new();
    table.insert("version", VERSION.into());
    table.insert("path", path.into());
    table.fmt();
    table.to_string()
}

fn absolute(path: &Path) -> Result<PathBuf> {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut normal = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normal.pop();
            }
            other => normal.push(other),
        }
    }
    let mut existing = normal.as_path();
    let mut missing = Vec::new();
    while !existing.exists() {
        let (Some(name), Some(parent)) = (existing.file_name(), existing.parent()) else {
            return Ok(normal);
        };
        missing.push(name.to_os_string());
        existing = parent;
    }
    let mut resolved = simplified(existing.canonicalize()?);
    resolved.extend(missing.iter().rev());
    Ok(resolved)
}

fn relative(from: &Path, to: &Path) -> String {
    let from: Vec<_> = from.components().collect();
    let to: Vec<_> = to.components().collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let parts: Vec<String> = std::iter::repeat_n("..".to_string(), from.len() - common)
        .chain(
            to[common..]
                .iter()
                .map(|part| part.as_os_str().to_string_lossy().into_owned()),
        )
        .collect();
    if parts.is_empty() {
        ".".to_string()
    } else {
        parts.join("/")
    }
}

fn simplified(path: PathBuf) -> PathBuf {
    match path.to_str().and_then(strip_verbatim) {
        Some(stripped) => PathBuf::from(stripped),
        None => path,
    }
}

fn strip_verbatim(path: &str) -> Option<String> {
    if let Some(share) = path.strip_prefix(r"\\?\UNC\") {
        return Some(format!(r"\\{share}"));
    }
    let rest = path.strip_prefix(r"\\?\")?;
    let drive = rest.as_bytes();
    (drive.len() >= 2 && drive[0].is_ascii_alphabetic() && drive[1] == b':')
        .then(|| rest.to_string())
}

fn manifest_path(path: &str) -> String {
    strip_verbatim(path)
        .unwrap_or_else(|| path.to_string())
        .replace('\\', "/")
}

fn slashed(path: &Path) -> String {
    manifest_path(&path.to_string_lossy())
}

struct Workspace {
    root: PathBuf,
    manifest: PathBuf,
    document: DocumentMut,
}

fn read_manifest(path: &Path) -> Result<Option<DocumentMut>> {
    if !path.is_file() {
        return Ok(None);
    }
    let document = fs::read_to_string(path)?
        .parse()
        .with_context(|| format!("{} is not valid TOML", path.display()))?;
    Ok(Some(document))
}

fn workspace_table(document: &DocumentMut) -> Option<&dyn TableLike> {
    document.get("workspace").and_then(Item::as_table_like)
}

fn workspace_of(game: &Path) -> Result<Option<Workspace>> {
    let own = game.join("Cargo.toml");
    if let Some(document) = read_manifest(&own)? {
        if workspace_table(&document).is_some() {
            return Ok(Some(Workspace {
                root: game.to_path_buf(),
                manifest: own,
                document,
            }));
        }
    }
    for root in game.ancestors().skip(1) {
        let manifest = root.join("Cargo.toml");
        let Some(document) = read_manifest(&manifest)? else {
            continue;
        };
        let Some(table) = workspace_table(&document) else {
            continue;
        };
        let game_path = slashed(game.strip_prefix(root)?);
        let game_parts = parts(&game_path);
        let excluded = strings(table.get("exclude"))
            .iter()
            .any(|path| game_parts.starts_with(&parts(path)));
        if excluded {
            return Ok(None);
        }
        return Ok(Some(Workspace {
            root: root.to_path_buf(),
            manifest,
            document,
        }));
    }
    Ok(None)
}

fn declare_own_workspace(game: &Path, report: &mut Report) -> Result<()> {
    let manifest = game.join("Cargo.toml");
    let mut document = read_manifest(&manifest)?.context("the game has no Cargo.toml")?;
    let mut members = Array::new();
    members.push(".");
    members.push("editor");
    let mut table = Table::new();
    table.insert("members", toml_edit::value(members));
    table.decor_mut().set_prefix("\n");
    document.insert("workspace", Item::Table(table));
    fs::write(&manifest, document.to_string())?;
    report.edited.push(manifest);
    Ok(())
}

fn join_workspace(
    game: &Path,
    mut workspace: Workspace,
    crates: &[PathBuf],
    report: &mut Report,
) -> Result<()> {
    let table = workspace
        .document
        .get_mut("workspace")
        .and_then(Item::as_table_like_mut)
        .context("the workspace table vanished")?;
    if table.get("members").is_none() {
        table.insert("members", toml_edit::value(Array::new()));
    }
    let members = table
        .get_mut("members")
        .and_then(Item::as_array_mut)
        .with_context(|| {
            format!(
                "the workspace's members in {} are not an array",
                workspace.manifest.display()
            )
        })?;
    let mut patterns = members
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect::<Vec<_>>();
    let game_path = slashed(game.strip_prefix(&workspace.root)?);
    let mut changed = false;
    for path in crates {
        let path = slashed(path.strip_prefix(&workspace.root)?);
        if patterns.iter().any(|pattern| matches(pattern, &path)) {
            continue;
        }
        let member = patterns
            .iter()
            .find_map(|pattern| {
                let parent = pattern.strip_suffix("/*")?;
                let covers = !parent.contains(['*', '?', '['])
                    && matches(pattern, &game_path)
                    && path == format!("{game_path}/editor");
                covers.then(|| format!("{pattern}/editor"))
            })
            .unwrap_or(path);
        push_member(members, &member);
        patterns.push(member);
        changed = true;
    }
    if changed {
        fs::write(&workspace.manifest, workspace.document.to_string())?;
        report.edited.push(workspace.manifest);
    }
    Ok(())
}

fn push_member(members: &mut Array, member: &str) {
    let prefix = members.iter().last().map(|last| {
        let prefix = last
            .decor()
            .prefix()
            .and_then(|prefix| prefix.as_str())
            .unwrap_or("");
        match prefix.rfind('\n') {
            Some(newline) => prefix[newline..].to_string(),
            None => " ".to_string(),
        }
    });
    members.push(member);
    if let (Some(prefix), Some(value)) = (prefix, members.iter_mut().last()) {
        value.decor_mut().set_prefix(prefix);
    }
}

fn strings(item: Option<&Item>) -> Vec<String> {
    item.and_then(Item::as_array)
        .map(|array| {
            array
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn parts(path: &str) -> Vec<&str> {
    path.split(['/', '\\'])
        .filter(|part| !part.is_empty() && *part != ".")
        .collect()
}

fn matches(pattern: &str, path: &str) -> bool {
    let pattern = parts(pattern);
    let path = parts(path);
    pattern.len() == path.len()
        && pattern
            .iter()
            .zip(&path)
            .all(|(pattern, part)| glob(pattern.as_bytes(), part.as_bytes()))
}

fn glob(pattern: &[u8], text: &[u8]) -> bool {
    match pattern.split_first() {
        None => text.is_empty(),
        Some((b'*', rest)) => (0..=text.len()).any(|skip| glob(rest, &text[skip..])),
        Some((b'?', rest)) => !text.is_empty() && glob(rest, &text[1..]),
        Some((c, rest)) => text.first() == Some(c) && glob(rest, &text[1..]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VERSION: &str = env!("CARGO_PKG_VERSION");

    fn read(path: impl AsRef<Path>) -> String {
        fs::read_to_string(path).unwrap()
    }

    fn write(path: impl AsRef<Path>, contents: &str) {
        let path = path.as_ref();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    const GAME_MANIFEST: &str = "[package]\nname = \"g\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n# the game\n[dependencies]\n";

    fn write_game(dir: &Path) {
        write_game_with_plugins(dir, "");
    }

    fn write_game_with_plugins(dir: &Path, plugins: &str) {
        write(dir.join("Cargo.toml"), &format!("{GAME_MANIFEST}{plugins}"));
        write(dir.join("src/lib.rs"), "pub fn game() {}\n");
    }

    fn plugin_list(entries: &str) -> String {
        format!("\n[package.metadata.concerto.editor]\nplugins = [{entries}]\n")
    }

    const PLUGINS: &str = "\n[package.metadata.concerto.editor]\nplugins = [{ package = \"concerto-motion-matching-editor\", install = \"concerto_motion_matching_editor::install\" }]\n";

    const PLAIN_MAIN: &str = "fn main() {\n    concerto_editor::run(g::game, concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/..\"));\n}\n";

    const MOTION_MAIN: &str = "fn main() {\n    concerto_editor::run_with(\n        g::game,\n        concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/..\"),\n        &[concerto_motion_matching_editor::install],\n    );\n}\n";

    fn editor_manifest(dependency: &str) -> String {
        format!(
            "[package]\nname = \"g-editor\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n[features]\ntracy = [\"concerto-editor/tracy\"]\n\n[dependencies]\ng = {{ path = \"..\" }}\nconcerto-editor = \"{VERSION}\"\n{dependency}"
        )
    }

    fn canonical(path: &Path) -> PathBuf {
        simplified(path.canonicalize().unwrap())
    }

    #[test]
    fn new_writes_a_buildable_project_that_names_its_engine_by_version_or_checkout_path() {
        let tmp = tempfile::tempdir().unwrap();
        let checkout = tmp.path().join("engine");
        fs::create_dir_all(&checkout).unwrap();
        let published = tmp.path().join("my-game");
        let outside = tmp.path().join("projects/g");
        let inside = checkout.join("examples/g");

        let published_report = new_project(&published, Engine::Published).unwrap();
        let outside_report = new_project(&outside, Engine::Checkout(checkout.clone())).unwrap();
        new_project(&inside, Engine::Checkout(checkout.clone())).unwrap();

        for file in [
            "Cargo.toml",
            "src/lib.rs",
            "src/main.rs",
            "build.rs",
            "content/.gitkeep",
            "index.html",
            "Trunk.toml",
            ".gitignore",
            "editor/Cargo.toml",
            "editor/src/main.rs",
        ] {
            assert!(published.join(file).is_file(), "{file} was not written");
        }
        let manifest = read(published.join("Cargo.toml"));
        assert!(manifest.contains("name = \"my-game\""));
        assert!(manifest.contains(&format!("concerto = \"{VERSION}\"")));
        assert!(manifest.contains(&format!("concerto-asset-build = \"{VERSION}\"")));
        assert!(manifest.contains("[workspace]\nmembers = [\".\", \"editor\"]\n"));
        assert!(manifest.contains(
            "[profile.dev]\nopt-level = 1\n\n[profile.dev.package.\"*\"]\nopt-level = 3\n"
        ));
        assert!(read(published.join("src/main.rs"))
            .contains("concerto::run(my_game::game, concerto::content_registry!());"));
        let ignored = read(published.join(".gitignore"));
        assert!(ignored.lines().any(|line| line == "content/.registry.toml"));
        assert!(ignored.lines().any(|line| line == "derived-cache/"));
        let editor = read(published.join("editor/Cargo.toml"));
        assert!(editor.contains("name = \"my-game-editor\""));
        assert!(editor.contains("my-game = { path = \"..\" }"));
        assert!(editor.contains(&format!("concerto-editor = \"{VERSION}\"")));
        assert_eq!(
            read(published.join("editor/src/main.rs")),
            "fn main() {\n    concerto_editor::run_with(\n        my_game::game,\n        concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/..\"),\n        &[concerto_animation_editor::install],\n    );\n}\n"
        );
        assert!(!published_report.warnings.is_empty());

        let absolute = slashed(&canonical(&checkout));
        assert!(read(outside.join("Cargo.toml")).contains(&format!(
            "concerto = {{ version = \"{VERSION}\", path = \"{absolute}\" }}"
        )));
        assert!(read(outside.join("editor/Cargo.toml")).contains(&format!(
            "concerto-animation-editor = {{ version = \"{VERSION}\", path = \"{absolute}/crates/animation-editor\" }}"
        )));
        assert!(outside_report.warnings.is_empty());
        assert!(read(inside.join("Cargo.toml")).contains(&format!(
            "concerto-asset-build = {{ version = \"{VERSION}\", path = \"../../crates/asset-build\" }}"
        )));
        assert!(read(inside.join("editor/Cargo.toml")).contains(&format!(
            "concerto-editor = {{ version = \"{VERSION}\", path = \"../../../crates/editor\" }}"
        )));
    }

    #[test]
    fn new_joins_an_enclosing_workspace_and_leaves_nothing_behind_when_it_cannot() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("ws");
        let unusable = "[workspace]\nmembers = \"g\"\n";
        write(ws.join("Cargo.toml"), unusable);
        let created = ws.join("g");
        let existing = ws.join("h");
        fs::create_dir_all(&existing).unwrap();

        assert!(new_project(&created, Engine::Published).is_err());
        assert!(new_project(&existing, Engine::Published).is_err());

        assert!(!created.exists());
        assert!(fs::read_dir(&existing).unwrap().next().is_none());
        assert_eq!(read(ws.join("Cargo.toml")), unusable);

        write(
            ws.join("Cargo.toml"),
            "[workspace]\nmembers = [\"tools\"]\n",
        );
        let dir = ws.join("games/g");
        new_project(&dir, Engine::Published).unwrap();

        assert_eq!(
            read(ws.join("Cargo.toml")),
            "[workspace]\nmembers = [\"tools\", \"games/g\", \"games/g/editor\"]\n"
        );
        let manifest = read(dir.join("Cargo.toml"));
        assert!(!manifest.contains("[workspace]"));
        assert!(!manifest.contains("[profile"));
    }

    #[test]
    fn new_refuses_a_directory_in_use_or_a_name_cargo_or_the_engine_owns() {
        let tmp = tempfile::tempdir().unwrap();
        let in_use = tmp.path().join("g");
        write(in_use.join("notes.txt"), "mine");

        assert!(new_project(&in_use, Engine::Published).is_err());
        assert_eq!(read(in_use.join("notes.txt")), "mine");
        assert!(!in_use.join("Cargo.toml").exists());

        for name in ["1game", "test", "concerto-editor"] {
            let dir = tmp.path().join(name);
            assert!(new_project(&dir, Engine::Published).is_err(), "{name}");
            assert!(!dir.exists(), "{name}");
        }
    }

    struct Membership {
        name: &'static str,
        workspace: Option<&'static str>,
        game: &'static str,
        game_manifest: &'static str,
        workspace_after: Option<String>,
        game_after: String,
    }

    #[test]
    fn the_editor_joins_the_workspace_that_holds_the_game_or_gives_the_game_its_own() {
        let own = format!("{GAME_MANIFEST}\n[workspace]\nmembers = [\".\", \"editor\"]\n");
        let globbed = "[package]\nname = \"root\"\n\n[workspace]\n# every engine crate and example\nmembers = [\"crates/*\", \"examples/*\"] # globs\n\n[workspace.package]\nversion = \"0.1.0\"\n";
        let covered =
            "[workspace]\nmembers = [\"crates/*\", \"examples/*\", \"examples/*/editor\"]\n";
        let excluded = "[workspace]\nmembers = [\"examples/*\"]\nexclude = [\"examples/g\"]\n";
        let multi_line =
            "[workspace]\nmembers = [\n    \"tools\",\n    # the game\n    \"games/g\",\n]\n";
        let own_with_tools = "[package]\nname = \"g\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[workspace]\nmembers = [\".\", \"tools\"]\nresolver = \"2\"\n";
        let memberships = [
            Membership {
                name: "an enclosing workspace with globs and comments",
                workspace: Some(globbed),
                game: "examples/g",
                game_manifest: GAME_MANIFEST,
                workspace_after: Some(globbed.replace(
                    "\"examples/*\"]",
                    "\"examples/*\", \"examples/*/editor\"]",
                )),
                game_after: GAME_MANIFEST.to_string(),
            },
            Membership {
                name: "a multi-line member list",
                workspace: Some(multi_line),
                game: "games/g",
                game_manifest: GAME_MANIFEST,
                workspace_after: Some(
                    "[workspace]\nmembers = [\n    \"tools\",\n    # the game\n    \"games/g\",\n    \"games/g/editor\",\n]\n".to_string(),
                ),
                game_after: GAME_MANIFEST.to_string(),
            },
            Membership {
                name: "a member already covered",
                workspace: Some(covered),
                game: "examples/g",
                game_manifest: GAME_MANIFEST,
                workspace_after: Some(covered.to_string()),
                game_after: GAME_MANIFEST.to_string(),
            },
            Membership {
                name: "an excluded game",
                workspace: Some(excluded),
                game: "examples/g",
                game_manifest: GAME_MANIFEST,
                workspace_after: Some(excluded.to_string()),
                game_after: own.clone(),
            },
            Membership {
                name: "a standalone game",
                workspace: None,
                game: "g",
                game_manifest: GAME_MANIFEST,
                workspace_after: None,
                game_after: own.clone(),
            },
            Membership {
                name: "a game that is its own workspace",
                workspace: None,
                game: "g",
                game_manifest: own_with_tools,
                workspace_after: None,
                game_after: own_with_tools.replace("[\".\", \"tools\"]", "[\".\", \"tools\", \"editor\"]"),
            },
        ];
        for membership in memberships {
            let tmp = tempfile::tempdir().unwrap();
            let ws = tmp.path().join("ws");
            if let Some(workspace) = membership.workspace {
                write(ws.join("Cargo.toml"), workspace);
            }
            let game = ws.join(membership.game);
            write(game.join("Cargo.toml"), membership.game_manifest);
            write(game.join("src/lib.rs"), "pub fn game() {}\n");

            add_editor(&game, Engine::Published).unwrap();

            assert_eq!(
                read(game.join("editor/src/main.rs")),
                PLAIN_MAIN,
                "{}",
                membership.name
            );
            assert_eq!(
                read(game.join("Cargo.toml")),
                membership.game_after,
                "{}",
                membership.name
            );
            if let Some(after) = membership.workspace_after {
                assert_eq!(read(ws.join("Cargo.toml")), after, "{}", membership.name);
            }
        }
    }

    struct Refusal {
        name: &'static str,
        workspace: &'static str,
        plugins: String,
        editor_notes: bool,
        library: bool,
    }

    #[test]
    fn the_editor_refuses_or_rolls_back_leaving_the_game_and_its_workspace_as_they_were() {
        let refusals = [
            Refusal {
                name: "an editor directory in use",
                workspace: "[workspace]\nmembers = [\"g\"]\n",
                plugins: String::new(),
                editor_notes: true,
                library: true,
            },
            Refusal {
                name: "a game without a library",
                workspace: "[workspace]\nmembers = [\"g\"]\n",
                plugins: String::new(),
                editor_notes: false,
                library: false,
            },
            Refusal {
                name: "a workspace that cannot be read",
                workspace: "name = {{bad}}\n",
                plugins: String::new(),
                editor_notes: false,
                library: true,
            },
            Refusal {
                name: "a workspace that cannot take the member",
                workspace: "[workspace]\nmembers = \"g\"\n",
                plugins: String::new(),
                editor_notes: false,
                library: true,
            },
            Refusal {
                name: "a plugin install that is not a path",
                workspace: "[workspace]\nmembers = [\"g\"]\n",
                plugins: plugin_list("{ package = \"concerto-a\", install = \"a::b(); evil()\" }"),
                editor_notes: false,
                library: true,
            },
        ];
        for refusal in refusals {
            let tmp = tempfile::tempdir().unwrap();
            let ws = tmp.path().join("ws");
            write(ws.join("Cargo.toml"), refusal.workspace);
            let game = ws.join("g");
            let manifest = format!("{GAME_MANIFEST}{}", refusal.plugins);
            write(game.join("Cargo.toml"), &manifest);
            if refusal.library {
                write(game.join("src/lib.rs"), "pub fn game() {}\n");
            }
            if refusal.editor_notes {
                write(game.join("editor/notes.txt"), "mine");
            }

            assert!(
                add_editor(&game, Engine::Published).is_err(),
                "{}",
                refusal.name
            );

            assert_eq!(read(game.join("Cargo.toml")), manifest, "{}", refusal.name);
            assert_eq!(
                read(ws.join("Cargo.toml")),
                refusal.workspace,
                "{}",
                refusal.name
            );
            assert!(!game.join("editor/Cargo.toml").exists(), "{}", refusal.name);
            if refusal.editor_notes {
                assert_eq!(read(game.join("editor/notes.txt")), "mine");
            }
        }
    }

    #[test]
    fn the_editor_depends_on_and_installs_each_listed_plugin_where_its_entry_says() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("g");
        write_game_with_plugins(
            &dir,
            &plugin_list(
                "{ package = \"by-version\", install = \"by_version::install\", version = \"2.1\" }, { package = \"by-path\", install = \"by_path::install\", path = \"../tools/by-path\" }, { package = \"by-both\", install = \"by_both::setup::install\", version = \"3\", path = \"plugins/by-both\" }, { package = \"concerto-own\", install = \"concerto_own::install\", path = \"/elsewhere/own\" }",
            ),
        );

        add_editor(&dir, Engine::Published).unwrap();

        let manifest = read(dir.join("editor/Cargo.toml"));
        assert!(manifest.contains("by-version = { version = \"2.1\" }\n"));
        assert!(manifest.contains("by-path = { path = \"../../tools/by-path\" }\n"));
        assert!(manifest.contains("by-both = { version = \"3\", path = \"../plugins/by-both\" }\n"));
        assert!(manifest.contains("concerto-own = { path = \"/elsewhere/own\" }\n"));
        assert_eq!(
            read(dir.join("editor/src/main.rs")),
            "fn main() {\n    concerto_editor::run_with(\n        g::game,\n        concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/..\"),\n        &[\n            by_version::install,\n            by_path::install,\n            by_both::setup::install,\n            concerto_own::install,\n        ],\n    );\n}\n"
        );
    }

    fn tree(dir: &Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        let mut folders = vec![dir.to_path_buf()];
        while let Some(folder) = folders.pop() {
            for entry in fs::read_dir(folder).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    folders.push(path);
                } else {
                    files.push(path);
                }
            }
        }
        files.sort();
        files
    }

    #[test]
    fn update_rewrites_only_the_editors_two_files_from_the_plugin_list_keeping_or_taking_its_engine(
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let checkout = tmp.path().join("engine");
        fs::create_dir_all(&checkout).unwrap();
        let ws = tmp.path().join("ws");
        write(ws.join("Cargo.toml"), "[workspace]\nmembers = [\"g\"]\n");
        let dir = ws.join("g");
        write_game(&dir);
        add_editor(&dir, Engine::Published).unwrap();
        write(dir.join("editor/src/notes.rs"), "mine");
        write_game_with_plugins(&dir, PLUGINS);
        let files = tree(&ws);
        let others = || -> Vec<String> {
            files
                .iter()
                .filter(|path| {
                    !path.ends_with("editor/Cargo.toml") && !path.ends_with("editor/src/main.rs")
                })
                .map(read)
                .collect()
        };
        let untouched = others();

        let report = update_editor(&dir, None).unwrap();

        assert_eq!(tree(&ws), files);
        assert_eq!(others(), untouched);
        assert_eq!(
            read(dir.join("editor/Cargo.toml")),
            editor_manifest(&format!(
                "concerto-motion-matching-editor = \"{VERSION}\"\n"
            ))
        );
        assert_eq!(read(dir.join("editor/src/main.rs")), MOTION_MAIN);
        assert!(!report.warnings.is_empty());

        write_game(&dir);
        update_editor(&dir, Some(Engine::Checkout(checkout.clone()))).unwrap();

        assert_eq!(read(dir.join("editor/src/main.rs")), PLAIN_MAIN);
        let taken = read(dir.join("editor/Cargo.toml"));
        let absolute = slashed(&canonical(&checkout));
        assert!(taken.contains(&format!(
            "concerto-editor = {{ version = \"{VERSION}\", path = \"{absolute}/crates/editor\" }}"
        )));

        let report = update_editor(&dir, None).unwrap();

        assert_eq!(read(dir.join("editor/Cargo.toml")), taken);
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn update_refuses_without_an_editor_an_engine_or_a_valid_plugin_list_and_writes_nothing() {
        let rows = [
            Some("[dependencies]\nconcerto-editor = { workspace = true }\n"),
            Some("[dependencies]\nconcerto-editor = { version = \"1\", git = \"https://example.com/e\" }\n"),
            None,
        ];
        for editor in rows {
            let tmp = tempfile::tempdir().unwrap();
            let dir = tmp.path().join("g");
            write_game(&dir);
            if let Some(manifest) = editor {
                write(dir.join("editor/Cargo.toml"), manifest);
            }

            assert!(update_editor(&dir, None).is_err(), "{editor:?}");

            match editor {
                Some(manifest) => assert_eq!(read(dir.join("editor/Cargo.toml")), manifest),
                None => assert!(!dir.join("editor").exists()),
            }
        }

        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("g");
        write_game(&dir);
        add_editor(&dir, Engine::Published).unwrap();
        let manifest = read(dir.join("editor/Cargo.toml"));
        write_game_with_plugins(
            &dir,
            &plugin_list("{ package = \"concerto-a\", install = \"a::fn\" }"),
        );

        assert!(update_editor(&dir, None).is_err());

        assert_eq!(read(dir.join("editor/Cargo.toml")), manifest);
        assert_eq!(read(dir.join("editor/src/main.rs")), PLAIN_MAIN);
    }

    #[test]
    fn the_parser_refuses_options_that_do_not_apply_to_the_command() {
        for args in [
            &["build", "--target", "x86_64-unknown-linux-gnu"][..],
            &["build", "--public-url", "./"],
            &["run", "--target", "wasm32-unknown-unknown"],
            &["build", "--", "--forest"],
            &["new", "x", "--update"],
        ] {
            assert!(parse(args.iter().copied()).is_err(), "{args:?}");
        }
    }

    fn args(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| arg.to_string()).collect()
    }

    fn cargo_step(game: &Path, release: bool) -> BuildStep {
        let mut cargo = args(&["build", "--manifest-path"]);
        cargo.push(game.join("Cargo.toml").display().to_string());
        cargo.extend(args(&["-p", "g", "--bin", "g"]));
        if release {
            cargo.push("--release".to_string());
        }
        cargo.push("--message-format=json-render-diagnostics".to_string());
        BuildStep::Cargo { args: cargo }
    }

    fn build_options(dir: &str) -> BuildOptions {
        BuildOptions {
            dir: PathBuf::from(dir),
            cook: false,
            release: false,
            wasm: false,
            public_url: None,
            dist: None,
        }
    }

    struct Plan {
        name: &'static str,
        options: BuildOptions,
        game_args: Option<Vec<String>>,
        steps: Vec<BuildStep>,
    }

    #[test]
    fn each_build_and_run_plans_its_cargo_cook_trunk_overlay_and_run_steps() {
        let game = Path::new("/g");
        let target = Path::new("/t");
        let dist = Path::new("/d");
        let cooked = target.join("cooked").join("g");
        let staged = || BuildStep::Overlay {
            from: cooked.clone(),
            onto: OverlayTarget::BuiltContent,
        };
        let cook = || BuildStep::Cook {
            out: cooked.clone(),
        };
        let mut web = args(&["build", "--dist"]);
        web.push(game.join("dist").display().to_string());
        let mut cooked_web = args(&["build", "--release", "--public-url", "./", "--dist"]);
        cooked_web.push(dist.display().to_string());
        let plans = [
            Plan {
                name: "a native cooked release",
                options: BuildOptions {
                    cook: true,
                    release: true,
                    ..build_options("/g")
                },
                game_args: None,
                steps: vec![cargo_step(game, true), cook(), staged()],
            },
            Plan {
                name: "a cooked run",
                options: BuildOptions {
                    cook: true,
                    ..build_options("/g")
                },
                game_args: Some(args(&["--forest"])),
                steps: vec![
                    cargo_step(game, false),
                    cook(),
                    staged(),
                    BuildStep::Run {
                        args: args(&["--forest"]),
                    },
                ],
            },
            Plan {
                name: "a web build",
                options: BuildOptions {
                    wasm: true,
                    ..build_options("/g")
                },
                game_args: None,
                steps: vec![BuildStep::Trunk {
                    dir: game.to_path_buf(),
                    args: web,
                }],
            },
            Plan {
                name: "a cooked web release",
                options: BuildOptions {
                    cook: true,
                    release: true,
                    wasm: true,
                    public_url: Some("./".to_string()),
                    dist: Some(dist.to_path_buf()),
                    ..build_options("/g")
                },
                game_args: None,
                steps: vec![
                    cargo_step(game, true),
                    cook(),
                    BuildStep::Trunk {
                        dir: game.to_path_buf(),
                        args: cooked_web,
                    },
                    BuildStep::Overlay {
                        from: cooked.clone(),
                        onto: OverlayTarget::Dir(dist.to_path_buf()),
                    },
                ],
            },
        ];
        for plan in plans {
            let planned = match &plan.game_args {
                Some(game_args) => plan_run(&plan.options, game_args, "g", target),
                None => plan_build(&plan.options, "g", target),
            };
            assert_eq!(planned.steps, plan.steps, "{}", plan.name);
        }
    }

    #[test]
    fn an_overlay_replaces_the_files_that_exist_and_names_the_ones_it_skips() {
        let tmp = tempfile::tempdir().unwrap();
        let from = tmp.path().join("cooked");
        let onto = tmp.path().join("dist");
        write(from.join(".concerto-cook"), "");
        write(from.join("content/a.gasset"), "cooked a");
        write(from.join("content/deep/b.gasset"), "cooked b");
        write(from.join("content/c.gasset"), "cooked c");
        write(onto.join("content/a.gasset"), "raw a");
        write(onto.join("content/deep/b.gasset"), "raw b");
        write(onto.join("index.html"), "page");
        let mut report = Report::default();

        overlay(&from, &onto, &mut report).unwrap();

        assert_eq!(read(onto.join("content/a.gasset")), "cooked a");
        assert_eq!(read(onto.join("content/deep/b.gasset")), "cooked b");
        assert_eq!(read(onto.join("index.html")), "page");
        assert!(!onto.join("content/c.gasset").exists());
        assert!(!onto.join(".concerto-cook").exists());
        assert!(
            report
                .notes
                .iter()
                .any(|note| note.contains("content/c.gasset")),
            "{:?}",
            report.notes
        );
    }

    struct Written {
        name: &'static str,
        produced: String,
        expected: &'static str,
    }

    #[test]
    fn windows_paths_and_crlf_templates_are_written_the_way_cargo_reads_them() {
        let rows = [
            Written {
                name: "a verbatim drive path",
                produced: manifest_path(r"\\?\C:\Users\me\engine"),
                expected: "C:/Users/me/engine",
            },
            Written {
                name: "a verbatim share path",
                produced: manifest_path(r"\\?\UNC\server\share\engine"),
                expected: "//server/share/engine",
            },
            Written {
                name: "a template checked out with CRLF",
                produced: render(
                    "fn main() {\r\n    {{crate}}::run();\r\n}\r\n",
                    &[("crate", "g")],
                ),
                expected: "fn main() {\n    g::run();\n}\n",
            },
        ];
        for row in rows {
            assert_eq!(row.produced, row.expected, "{}", row.name);
        }
    }

    const CARGO_MESSAGES: &str = r#"{"reason":"compiler-artifact","package_id":"path+file:///g#0.1.0","manifest_path":"/g/Cargo.toml","target":{"kind":["custom-build"],"crate_types":["bin"],"name":"build-script-build","src_path":"/g/build.rs","edition":"2024","doc":false,"doctest":false,"test":false},"features":[],"filenames":["/t/debug/build/g-1/build-script-build"],"executable":null,"fresh":true}
{"reason":"build-script-executed","package_id":"path+file:///g#0.1.0","linked_libs":[],"linked_paths":[],"cfgs":[],"env":[],"out_dir":"/t/debug/build/g-2/out"}
{"reason":"compiler-artifact","package_id":"path+file:///g#0.1.0","manifest_path":"/g/Cargo.toml","target":{"kind":["lib"],"crate_types":["lib"],"name":"g","src_path":"/g/src/lib.rs","edition":"2024","doc":true,"doctest":true,"test":true},"features":[],"filenames":["/t/debug/libg.rlib"],"executable":null,"fresh":false}
{"reason":"compiler-artifact","package_id":"path+file:///g#0.1.0","manifest_path":"/g/Cargo.toml","target":{"kind":["bin"],"crate_types":["bin"],"name":"g-tool","src_path":"/g/src/bin/g-tool.rs","edition":"2024","doc":true,"doctest":false,"test":true},"features":[],"filenames":["/t/debug/g-tool"],"executable":"/t/debug/g-tool","fresh":false}
{"reason":"compiler-artifact","package_id":"path+file:///g#0.1.0","manifest_path":"/g/Cargo.toml","target":{"kind":["bin"],"crate_types":["bin"],"name":"g","src_path":"/g/src/main.rs","edition":"2024","doc":true,"doctest":false,"test":true},"features":[],"filenames":["/t/debug/g"],"executable":"/t/debug/g","fresh":false}
{"reason":"build-finished","success":true}"#;

    struct Built {
        executable: &'static str,
        content: &'static str,
    }

    #[test]
    fn run_launches_the_packages_own_bin_and_stages_content_beside_it() {
        let found: Vec<PathBuf> = CARGO_MESSAGES
            .lines()
            .filter_map(|line| executable_of(line, "g"))
            .collect();
        assert_eq!(found, vec![PathBuf::from("/t/debug/g")]);

        for built in [
            Built {
                executable: "/t/x86_64-unknown-linux-gnu/release/g",
                content: "/t/x86_64-unknown-linux-gnu/release/g-content",
            },
            Built {
                executable: "/t/debug/g.exe",
                content: "/t/debug/g-content",
            },
        ] {
            assert_eq!(
                built_content_dir(Path::new(built.executable)),
                Some(PathBuf::from(built.content)),
                "{}",
                built.executable
            );
        }
    }
}
