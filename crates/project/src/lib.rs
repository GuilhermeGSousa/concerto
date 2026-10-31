use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, TableLike, Value};

const VERSION: &str = env!("CARGO_PKG_VERSION");

const GAME_FILES: &[(&str, &str)] = &[
    (
        "Cargo.toml",
        include_str!("../templates/game/Cargo.toml.template"),
    ),
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

const OWN_WORKSPACE: &str = "\n[workspace]\nmembers = [\".\", \"editor\"]\n\n[profile.dev.package.concerto-jolt-ffi]\nopt-level = 3";

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
];

/// How to run the tool.
pub const USAGE: &str = "usage: cargo concerto new <dir> [--engine <path>]\n       cargo concerto editor <dir> [--update] [--engine <path>]";

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
    }
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
    let game = Game {
        dir: dir.clone(),
        crate_name: name.replace('-', "_"),
        name,
        plugins: Vec::new(),
    };
    let vars = [
        ("name", game.name.as_str()),
        ("crate", game.crate_name.as_str()),
        ("concerto", &dependency(checkout.as_deref(), "", &dir, &dir)),
        (
            "concerto_asset_build",
            &dependency(checkout.as_deref(), "crates/asset-build", &dir, &dir),
        ),
        ("workspace", own_workspace),
    ];
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
        write(dir.join("Cargo.toml"), GAME_MANIFEST);
        write(dir.join("src/lib.rs"), "pub fn game() {}\n");
    }

    fn canonical(path: &Path) -> PathBuf {
        simplified(path.canonicalize().unwrap())
    }

    #[test]
    fn a_template_checked_out_with_crlf_renders_with_lf() {
        assert_eq!(
            render(
                "fn main() {\r\n    {{crate}}::run();\r\n}\r\n",
                &[("crate", "g")]
            ),
            "fn main() {\n    g::run();\n}\n"
        );
    }

    #[test]
    fn new_writes_a_buildable_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("my-game");

        new_project(&dir, Engine::Published).unwrap();

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
            assert!(dir.join(file).is_file(), "{file} was not written");
        }
        let manifest = read(dir.join("Cargo.toml"));
        assert!(manifest.contains("name = \"my-game\""));
        assert!(manifest.contains(&format!("concerto = \"{VERSION}\"")));
        assert!(manifest.contains(&format!("concerto-asset-build = \"{VERSION}\"")));
        assert!(manifest.contains("[lib]"));
        assert!(!manifest.contains("crate-type"));
        assert!(manifest.contains("[workspace]\nmembers = [\".\", \"editor\"]\n"));
        assert!(manifest.contains("[profile.dev.package.concerto-jolt-ffi]\nopt-level = 3\n"));
        assert!(read(dir.join("src/main.rs"))
            .contains("concerto::run(my_game::game, concerto::content_registry!());"));
        assert!(!dir.join("content/.registry.toml").exists());
        assert!(read(dir.join(".gitignore"))
            .lines()
            .any(|line| line == "content/.registry.toml"));
        assert!(read(dir.join("index.html")).contains("<title>my-game</title>"));
        let editor = read(dir.join("editor/Cargo.toml"));
        assert!(editor.contains("name = \"my-game-editor\""));
        assert!(editor.contains("my-game = { path = \"..\" }"));
        assert!(editor.contains(&format!("concerto-editor = \"{VERSION}\"")));
        assert_eq!(
            read(dir.join("editor/src/main.rs")),
            "fn main() {\n    concerto_editor::run(my_game::game, concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/..\"));\n}\n"
        );
        assert!(!read(dir.join("build.rs")).contains("//"));
    }

    #[test]
    fn engine_checkout_writes_each_crates_own_path() {
        let tmp = tempfile::tempdir().unwrap();
        let checkout = tmp.path().join("engine");
        fs::create_dir_all(&checkout).unwrap();
        let dir = tmp.path().join("projects/g");

        new_project(&dir, Engine::Checkout(checkout.clone())).unwrap();

        let checkout = slashed(&canonical(&checkout));
        let manifest = read(dir.join("Cargo.toml"));
        assert!(manifest.contains(&format!(
            "concerto = {{ version = \"{VERSION}\", path = \"{checkout}\" }}"
        )));
        assert!(manifest.contains(&format!(
            "concerto-asset-build = {{ version = \"{VERSION}\", path = \"{checkout}/crates/asset-build\" }}"
        )));
        assert!(read(dir.join("editor/Cargo.toml")).contains(&format!(
            "concerto-editor = {{ version = \"{VERSION}\", path = \"{checkout}/crates/editor\" }}"
        )));
    }

    #[test]
    fn inside_the_checkout_paths_are_relative() {
        let tmp = tempfile::tempdir().unwrap();
        let checkout = tmp.path().join("engine");
        fs::create_dir_all(&checkout).unwrap();
        let dir = checkout.join("examples/g");

        new_project(&dir, Engine::Checkout(checkout)).unwrap();

        let manifest = read(dir.join("Cargo.toml"));
        assert!(manifest.contains(&format!(
            "concerto = {{ version = \"{VERSION}\", path = \"../..\" }}"
        )));
        assert!(manifest.contains(&format!(
            "concerto-asset-build = {{ version = \"{VERSION}\", path = \"../../crates/asset-build\" }}"
        )));
        assert!(read(dir.join("editor/Cargo.toml")).contains(&format!(
            "concerto-editor = {{ version = \"{VERSION}\", path = \"../../../crates/editor\" }}"
        )));
    }

    #[test]
    fn new_refuses_a_non_empty_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("g");
        write(dir.join("notes.txt"), "mine");

        assert!(new_project(&dir, Engine::Published).is_err());

        assert_eq!(read(dir.join("notes.txt")), "mine");
        assert!(!dir.join("Cargo.toml").exists());
    }

    #[test]
    fn new_refuses_an_invalid_package_name() {
        let tmp = tempfile::tempdir().unwrap();
        for name in [
            "1game",
            "has space",
            "test",
            "Self",
            "self",
            "build",
            "deps",
            "examples",
            "incremental",
            "concerto",
            "concerto-editor",
            "concerto_editor",
            "concerto-asset-build",
        ] {
            let dir = tmp.path().join(name);

            let error = new_project(&dir, Engine::Published).unwrap_err();

            assert!(error.to_string().contains(name), "{error}");
            assert!(!dir.exists());
        }
    }

    #[test]
    fn editor_refuses_an_existing_editor_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("g");
        write_game(&dir);
        write(dir.join("editor/notes.txt"), "mine");

        assert!(add_editor(&dir, Engine::Published).is_err());

        assert!(!dir.join("editor/Cargo.toml").exists());
        assert_eq!(read(dir.join("Cargo.toml")), GAME_MANIFEST);
    }

    #[test]
    fn editor_requires_a_library() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("g");
        write(dir.join("Cargo.toml"), GAME_MANIFEST);
        write(dir.join("src/main.rs"), "fn main() {}\n");

        assert!(add_editor(&dir, Engine::Published).is_err());

        assert!(!dir.join("editor").exists());
        assert_eq!(read(dir.join("Cargo.toml")), GAME_MANIFEST);
    }

    #[test]
    fn editor_names_the_recipe_it_assumes() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("g");
        write_game(&dir);

        let report = add_editor(&dir, Engine::Published).unwrap();

        assert!(report.notes.iter().any(|note| note.contains("g::game")));
        assert_eq!(
            read(dir.join("editor/src/main.rs")),
            "fn main() {\n    concerto_editor::run(g::game, concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/..\"));\n}\n"
        );
    }

    #[test]
    fn an_enclosing_workspace_gains_the_editor_member() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("ws");
        let before = "[package]\nname = \"root\"\n\n[workspace]\n# every engine crate and example\nmembers = [\"crates/*\", \"examples/*\"] # globs\n\n[workspace.package]\nversion = \"0.1.0\"\n";
        write(ws.join("Cargo.toml"), before);
        let game = ws.join("examples/g");
        write_game(&game);

        let report = add_editor(&game, Engine::Published).unwrap();

        assert_eq!(
            read(ws.join("Cargo.toml")),
            before.replace("\"examples/*\"]", "\"examples/*\", \"examples/*/editor\"]")
        );
        assert_eq!(read(game.join("Cargo.toml")), GAME_MANIFEST);
        assert_eq!(report.edited, vec![canonical(&ws).join("Cargo.toml")]);
    }

    #[test]
    fn a_multi_line_member_list_keeps_its_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("ws");
        let before =
            "[workspace]\nmembers = [\n    \"tools\",\n    # the game\n    \"games/g\",\n]\n";
        write(ws.join("Cargo.toml"), before);
        let game = ws.join("games/g");
        write_game(&game);

        add_editor(&game, Engine::Published).unwrap();

        assert_eq!(
            read(ws.join("Cargo.toml")),
            "[workspace]\nmembers = [\n    \"tools\",\n    # the game\n    \"games/g\",\n    \"games/g/editor\",\n]\n"
        );
    }

    #[test]
    fn a_covered_member_is_left_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("ws");
        let before =
            "[workspace]\nmembers = [\"crates/*\", \"examples/*\", \"examples/*/editor\"]\n";
        write(ws.join("Cargo.toml"), before);
        let game = ws.join("examples/g");
        write_game(&game);

        let report = add_editor(&game, Engine::Published).unwrap();

        assert_eq!(read(ws.join("Cargo.toml")), before);
        assert!(report.edited.is_empty());
        assert!(game.join("editor/Cargo.toml").is_file());
    }

    #[test]
    fn an_excluded_game_is_not_added() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("ws");
        let before = "[workspace]\nmembers = [\"examples/*\"]\nexclude = [\"examples/g\"]\n";
        write(ws.join("Cargo.toml"), before);
        let game = ws.join("examples/g");
        write_game(&game);

        add_editor(&game, Engine::Published).unwrap();

        assert_eq!(read(ws.join("Cargo.toml")), before);
        assert_eq!(
            read(game.join("Cargo.toml")),
            format!("{GAME_MANIFEST}\n[workspace]\nmembers = [\".\", \"editor\"]\n")
        );
    }

    #[test]
    fn a_standalone_game_gains_its_own_workspace() {
        let tmp = tempfile::tempdir().unwrap();
        let game = tmp.path().join("g");
        write_game(&game);

        let report = add_editor(&game, Engine::Published).unwrap();

        assert_eq!(
            read(game.join("Cargo.toml")),
            format!("{GAME_MANIFEST}\n[workspace]\nmembers = [\".\", \"editor\"]\n")
        );
        assert_eq!(report.edited, vec![canonical(&game).join("Cargo.toml")]);
    }

    #[test]
    fn a_game_that_is_its_own_workspace_gains_the_member() {
        let tmp = tempfile::tempdir().unwrap();
        let game = tmp.path().join("g");
        let before = format!(
            "{GAME_MANIFEST}\n[workspace]\nmembers = [\".\", \"tools\"]\nresolver = \"2\"\n\n[workspace.dependencies]\nserde = \"1\"\n"
        );
        write(game.join("Cargo.toml"), &before);
        write(game.join("src/lib.rs"), "pub fn game() {}\n");

        let report = add_editor(&game, Engine::Published).unwrap();

        assert_eq!(
            read(game.join("Cargo.toml")),
            before.replace("[\".\", \"tools\"]", "[\".\", \"tools\", \"editor\"]")
        );
        assert_eq!(report.edited, vec![canonical(&game).join("Cargo.toml")]);
    }

    #[test]
    fn editor_writes_nothing_when_the_workspace_cannot_be_read() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("ws");
        write(ws.join("Cargo.toml"), "name = {{bad}}\n");
        let game = ws.join("g");
        write_game(&game);

        assert!(add_editor(&game, Engine::Published).is_err());
        assert!(!game.join("editor").exists());

        write(ws.join("Cargo.toml"), "[workspace]\nmembers = [\"g\"]\n");
        add_editor(&game, Engine::Published).unwrap();
        assert!(game.join("editor/Cargo.toml").is_file());
    }

    #[test]
    fn editor_removes_what_it_wrote_when_it_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("ws");
        write(ws.join("Cargo.toml"), "[workspace]\nmembers = \"g\"\n");
        let game = ws.join("g");
        write_game(&game);

        assert!(add_editor(&game, Engine::Published).is_err());
        assert!(!game.join("editor").exists());
        assert_eq!(
            read(ws.join("Cargo.toml")),
            "[workspace]\nmembers = \"g\"\n"
        );

        write(ws.join("Cargo.toml"), "[workspace]\nmembers = [\"g\"]\n");
        add_editor(&game, Engine::Published).unwrap();
        assert!(game.join("editor/Cargo.toml").is_file());
    }

    #[test]
    fn new_removes_the_project_it_created_when_it_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("ws");
        write(ws.join("Cargo.toml"), "[workspace]\nmembers = \"g\"\n");
        let dir = ws.join("g");

        let error = new_project(&dir, Engine::Published).unwrap_err();
        let manifest = canonical(&ws).join("Cargo.toml");
        assert!(format!("{error:#}").contains(&manifest.display().to_string()));
        assert!(!dir.exists());
        assert_eq!(
            read(ws.join("Cargo.toml")),
            "[workspace]\nmembers = \"g\"\n"
        );

        write(ws.join("Cargo.toml"), "[workspace]\nmembers = [\"g\"]\n");
        new_project(&dir, Engine::Published).unwrap();
        assert!(dir.join("editor/Cargo.toml").is_file());
    }

    #[test]
    fn new_empties_a_directory_it_did_not_create_when_it_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("ws");
        write(ws.join("Cargo.toml"), "[workspace]\nmembers = \"g\"\n");
        let dir = ws.join("g");
        fs::create_dir_all(&dir).unwrap();

        assert!(new_project(&dir, Engine::Published).is_err());
        assert!(dir.is_dir());
        assert!(fs::read_dir(&dir).unwrap().next().is_none());

        write(ws.join("Cargo.toml"), "[workspace]\nmembers = [\"g\"]\n");
        new_project(&dir, Engine::Published).unwrap();
        assert!(dir.join("editor/Cargo.toml").is_file());
    }

    #[test]
    fn manifest_paths_lose_the_verbatim_prefix_and_use_forward_slashes() {
        assert_eq!(
            manifest_path(r"\\?\C:\Users\me\engine"),
            "C:/Users/me/engine"
        );
        assert_eq!(
            manifest_path(r"\\?\UNC\server\share\engine"),
            "//server/share/engine"
        );
        assert_eq!(manifest_path(r"C:\Users\me\engine"), "C:/Users/me/engine");
        assert_eq!(manifest_path("/home/me/engine"), "/home/me/engine");
    }

    #[test]
    fn a_dependency_path_is_escaped_as_a_toml_string() {
        let path = r#"C:/Users/o"brien\engine's"#;

        let document: DocumentMut = format!("concerto = {}", dependency_value(path))
            .parse()
            .unwrap();

        assert_eq!(document["concerto"]["path"].as_str(), Some(path));
        assert_eq!(document["concerto"]["version"].as_str(), Some(VERSION));
    }

    #[test]
    fn new_inside_a_workspace_joins_it() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("ws");
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
        assert!(!manifest.contains("concerto-jolt-ffi"));
    }

    #[test]
    fn the_report_warns_without_engine() {
        let tmp = tempfile::tempdir().unwrap();
        let checkout = tmp.path().join("engine");
        fs::create_dir_all(&checkout).unwrap();

        let published = new_project(tmp.path().join("a"), Engine::Published).unwrap();
        let local = new_project(tmp.path().join("b"), Engine::Checkout(checkout)).unwrap();

        assert!(published
            .warnings
            .iter()
            .any(|warning| warning.contains("release")));
        assert!(local.warnings.is_empty());
    }

    #[test]
    fn the_parser_skips_cargo_s_subcommand_name() {
        assert_eq!(
            parse(["concerto", "new", "x"]).unwrap(),
            Command::New(PathBuf::from("x"), Engine::Published)
        );
        assert_eq!(
            parse(["concerto", "editor", "x", "--engine", "."]).unwrap(),
            Command::Editor(PathBuf::from("x"), Engine::Checkout(PathBuf::from(".")))
        );
        assert_eq!(
            parse(["new", "--engine", "e", "x"]).unwrap(),
            Command::New(PathBuf::from("x"), Engine::Checkout(PathBuf::from("e")))
        );
        assert!(parse(["concerto"]).is_err());
        assert!(parse(["concerto", "build", "x"]).is_err());
        assert!(parse(["concerto", "new"]).is_err());
        assert!(parse(["concerto", "new", "x", "--engine"]).is_err());
        assert!(parse(["concerto", "new", "x", "y"]).is_err());
        assert!(parse(["concerto", "new", "x", "--bogus"]).is_err());
    }

    const PLUGINS: &str = "\n[package.metadata.concerto.editor]\nplugins = [{ package = \"concerto-motion-matching-editor\", install = \"concerto_motion_matching_editor::install\" }]\n";

    fn write_game_with_plugins(dir: &Path, plugins: &str) {
        write(dir.join("Cargo.toml"), &format!("{GAME_MANIFEST}{plugins}"));
        write(dir.join("src/lib.rs"), "pub fn game() {}\n");
    }

    fn plugin_list(entries: &str) -> String {
        format!("\n[package.metadata.concerto.editor]\nplugins = [{entries}]\n")
    }

    fn editor_manifest(dependency: &str) -> String {
        format!(
            "[package]\nname = \"g-editor\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n[dependencies]\ng = {{ path = \"..\" }}\nconcerto-editor = \"{VERSION}\"\n{dependency}"
        )
    }

    const PLAIN_MAIN: &str = "fn main() {\n    concerto_editor::run(g::game, concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/..\"));\n}\n";

    const MOTION_MAIN: &str = "fn main() {\n    concerto_editor::run_with(\n        g::game,\n        concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/..\"),\n        &[concerto_motion_matching_editor::install],\n    );\n}\n";

    #[test]
    fn without_a_plugin_list_the_editor_calls_run() {
        for plugins in ["", &plugin_list("")] {
            let tmp = tempfile::tempdir().unwrap();
            let dir = tmp.path().join("g");
            write_game_with_plugins(&dir, plugins);

            add_editor(&dir, Engine::Published).unwrap();

            assert_eq!(read(dir.join("editor/Cargo.toml")), editor_manifest(""));
            assert_eq!(read(dir.join("editor/src/main.rs")), PLAIN_MAIN);
        }
    }

    #[test]
    fn a_plugin_list_names_the_crate_and_the_call() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("g");
        write_game_with_plugins(&dir, PLUGINS);

        add_editor(&dir, Engine::Published).unwrap();

        assert_eq!(
            read(dir.join("editor/Cargo.toml")),
            editor_manifest(&format!(
                "concerto-motion-matching-editor = \"{VERSION}\"\n"
            ))
        );
        assert_eq!(read(dir.join("editor/src/main.rs")), MOTION_MAIN);
        assert!(read(dir.join("Cargo.toml")).contains(PLUGINS));
        assert!(!read(dir.join("Cargo.toml")).contains("concerto-motion-matching-editor ="));
    }

    #[test]
    fn a_long_plugin_list_is_written_the_way_cargo_fmt_keeps_it() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("g");
        write_game_with_plugins(
            &dir,
            &plugin_list(
                "{ package = \"concerto-first-long-extension\", install = \"concerto_first_long_extension::install\" }, { package = \"concerto-second\", install = \"concerto_second::setup::install\" }",
            ),
        );

        add_editor(&dir, Engine::Published).unwrap();

        assert_eq!(
            read(dir.join("editor/src/main.rs")),
            "fn main() {\n    concerto_editor::run_with(\n        g::game,\n        concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/..\"),\n        &[\n            concerto_first_long_extension::install,\n            concerto_second::setup::install,\n        ],\n    );\n}\n"
        );
    }

    #[test]
    fn one_plugin_stays_on_one_line_however_long() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("g");
        write_game_with_plugins(
            &dir,
            &plugin_list(
                "{ package = \"concerto-a-very-long-plugin-crate-name-that-fills-the-row\", install = \"a_very_long_plugin_crate_name_that_fills_the_row::setup::install\" }",
            ),
        );

        add_editor(&dir, Engine::Published).unwrap();

        assert!(read(dir.join("editor/src/main.rs")).contains(
            "        &[a_very_long_plugin_crate_name_that_fills_the_row::setup::install],\n"
        ));
    }

    #[test]
    fn a_concerto_package_resolves_to_its_crate_in_the_checkout() {
        let tmp = tempfile::tempdir().unwrap();
        let checkout = tmp.path().join("engine");
        fs::create_dir_all(&checkout).unwrap();
        let inside = checkout.join("examples/g");
        let outside = tmp.path().join("projects/g");
        for dir in [&inside, &outside] {
            write_game_with_plugins(dir, PLUGINS);
            add_editor(dir, Engine::Checkout(checkout.clone())).unwrap();
        }

        assert!(read(inside.join("editor/Cargo.toml")).contains(&format!(
            "concerto-motion-matching-editor = {{ version = \"{VERSION}\", path = \"../../../crates/motion-matching-editor\" }}"
        )));
        let checkout = slashed(&canonical(&checkout));
        assert!(read(outside.join("editor/Cargo.toml")).contains(&format!(
            "concerto-motion-matching-editor = {{ version = \"{VERSION}\", path = \"{checkout}/crates/motion-matching-editor\" }}"
        )));
    }

    #[test]
    fn another_package_uses_the_version_and_path_its_entry_gives() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("g");
        write_game_with_plugins(
            &dir,
            &plugin_list(
                "{ package = \"by-version\", install = \"by_version::install\", version = \"2.1\" }, { package = \"by-path\", install = \"by_path::install\", path = \"../tools/by-path\" }, { package = \"by-both\", install = \"by_both::install\", version = \"3\", path = \"plugins/by-both\" }, { package = \"concerto-own\", install = \"concerto_own::install\", path = \"/elsewhere/own\" }",
            ),
        );

        add_editor(&dir, Engine::Published).unwrap();

        let manifest = read(dir.join("editor/Cargo.toml"));
        assert!(manifest.contains("by-version = { version = \"2.1\" }\n"));
        assert!(manifest.contains("by-path = { path = \"../../tools/by-path\" }\n"));
        assert!(manifest.contains("by-both = { version = \"3\", path = \"../plugins/by-both\" }\n"));
        assert!(manifest.contains("concerto-own = { path = \"/elsewhere/own\" }\n"));
    }

    #[test]
    fn a_malformed_entry_names_itself_and_nothing_is_written() {
        for (entries, named) in [
            ("\"loose\"", "loose"),
            ("{ install = \"a::install\" }", "a::install"),
            ("{ package = \"a-b\" }", "a-b"),
            ("{ package = 1, install = \"a::install\" }", "`package`"),
            (
                "{ package = \"bad name\", install = \"a::install\", version = \"1\" }",
                "bad name",
            ),
            (
                "{ package = \"thing\", install = \"thing::install\" }",
                "thing",
            ),
            (
                "{ package = \"concerto-\", install = \"a::install\" }",
                "concerto-",
            ),
            (
                "{ package = \"concerto-a\", install = \"install\" }",
                "concerto-a",
            ),
            (
                "{ package = \"concerto-a\", install = \"a::\" }",
                "concerto-a",
            ),
            (
                "{ package = \"concerto-a\", install = \"a::b(); evil()\" }",
                "concerto-a",
            ),
            (
                "{ package = \"concerto-a\", install = \"a::fn\" }",
                "concerto-a",
            ),
            (
                "{ package = \"concerto-a\", install = \"a::install\", version = 1 }",
                "`version`",
            ),
            (
                "{ package = \"concerto-a\", install = \"a::install\", pth = \"x\" }",
                "pth",
            ),
            (
                "{ package = \"concerto-editor\", install = \"a::install\" }",
                "concerto-editor",
            ),
            ("{ package = \"g\", install = \"a::install\", version = \"1\" }", "`g`"),
            (
                "{ package = \"concerto-a\", install = \"a::i\" }, { package = \"concerto-a\", install = \"a::j\" }",
                "a::j",
            ),
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let dir = tmp.path().join("g");
            write_game_with_plugins(&dir, &plugin_list(entries));

            let added = add_editor(&dir, Engine::Published).unwrap_err();

            assert!(format!("{added:#}").contains(named), "{added:#}");
            assert!(!dir.join("editor").exists(), "{entries}");
            assert!(!read(dir.join("Cargo.toml")).contains("[workspace]"));

            write(dir.join("editor/Cargo.toml"), "mine");
            write(dir.join("editor/src/main.rs"), "mine too");

            let updated = update_editor(&dir, None).unwrap_err();

            assert!(format!("{updated:#}").contains(named), "{updated:#}");
            assert_eq!(read(dir.join("editor/Cargo.toml")), "mine");
            assert_eq!(read(dir.join("editor/src/main.rs")), "mine too");
        }
    }

    #[test]
    fn a_plugin_list_that_is_not_an_array_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("g");
        write_game_with_plugins(
            &dir,
            "\n[package.metadata.concerto.editor]\nplugins = \"all\"\n",
        );

        let error = add_editor(&dir, Engine::Published).unwrap_err();

        assert!(format!("{error:#}").contains("plugins"), "{error:#}");
        assert!(!dir.join("editor").exists());
    }

    fn tree(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut files = Vec::new();
        let mut folders = vec![dir.to_path_buf()];
        while let Some(folder) = folders.pop() {
            for entry in fs::read_dir(folder).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    folders.push(path);
                } else {
                    let contents = fs::read(&path).unwrap();
                    files.push((path, contents));
                }
            }
        }
        files.sort();
        files
    }

    #[test]
    fn update_rewrites_only_the_editors_two_files() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("ws");
        write(ws.join("Cargo.toml"), "[workspace]\nmembers = [\"g\"]\n");
        let dir = ws.join("g");
        write_game(&dir);
        add_editor(&dir, Engine::Published).unwrap();
        write(dir.join("editor/src/notes.rs"), "mine");
        write(dir.join("editor/README.md"), "also mine");
        write_game_with_plugins(&dir, PLUGINS);
        let before = tree(&ws);

        let report = update_editor(&dir, None).unwrap();

        let dir = canonical(&dir);
        let changed: Vec<_> = tree(&ws)
            .into_iter()
            .filter(|file| !before.contains(file))
            .map(|(path, _)| canonical(&path))
            .collect();
        assert_eq!(
            changed,
            vec![
                dir.join("editor/Cargo.toml"),
                dir.join("editor/src/main.rs")
            ]
        );
        assert_eq!(tree(&ws).len(), before.len());
        assert_eq!(
            report.written,
            vec![
                dir.join("editor/Cargo.toml"),
                dir.join("editor/src/main.rs")
            ]
        );
        assert!(report.edited.is_empty());
        assert_eq!(read(dir.join("editor/src/main.rs")), MOTION_MAIN);
    }

    #[test]
    fn update_with_an_empty_list_goes_back_to_run() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("g");
        write_game_with_plugins(&dir, PLUGINS);
        add_editor(&dir, Engine::Published).unwrap();
        let manifest = read(dir.join("Cargo.toml")).replace(PLUGINS, "");
        write(dir.join("Cargo.toml"), &manifest);

        update_editor(&dir, None).unwrap();

        assert_eq!(read(dir.join("editor/Cargo.toml")), editor_manifest(""));
        assert_eq!(read(dir.join("editor/src/main.rs")), PLAIN_MAIN);
    }

    #[test]
    fn update_keeps_the_engine_the_editor_already_has() {
        let tmp = tempfile::tempdir().unwrap();
        let checkout = tmp.path().join("engine");
        fs::create_dir_all(&checkout).unwrap();
        let inside = checkout.join("examples/g");
        let outside = tmp.path().join("projects/g");
        for dir in [&inside, &outside] {
            write_game(dir);
            add_editor(dir, Engine::Checkout(checkout.clone())).unwrap();
            let before = read(dir.join("editor/Cargo.toml"));

            let report = update_editor(dir, None).unwrap();

            assert_eq!(read(dir.join("editor/Cargo.toml")), before);
            assert!(report.warnings.is_empty());
        }

        write_game_with_plugins(&inside, PLUGINS);
        update_editor(&inside, None).unwrap();

        assert!(read(inside.join("editor/Cargo.toml")).contains(&format!(
            "concerto-motion-matching-editor = {{ version = \"{VERSION}\", path = \"../../../crates/motion-matching-editor\" }}"
        )));
    }

    #[test]
    fn update_keeps_the_published_engine_and_warns() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("g");
        write_game(&dir);
        add_editor(&dir, Engine::Published).unwrap();

        let report = update_editor(&dir, None).unwrap();

        assert_eq!(read(dir.join("editor/Cargo.toml")), editor_manifest(""));
        assert!(report
            .warnings
            .iter()
            .any(|warning| warning.contains("release")));
    }

    #[test]
    fn update_takes_the_engine_it_is_given() {
        let tmp = tempfile::tempdir().unwrap();
        let checkout = tmp.path().join("engine");
        fs::create_dir_all(&checkout).unwrap();
        let dir = tmp.path().join("g");
        write_game(&dir);
        add_editor(&dir, Engine::Published).unwrap();

        update_editor(&dir, Some(Engine::Checkout(checkout.clone()))).unwrap();

        let checkout = slashed(&canonical(&checkout));
        assert!(read(dir.join("editor/Cargo.toml")).contains(&format!(
            "concerto-editor = {{ version = \"{VERSION}\", path = \"{checkout}/crates/editor\" }}"
        )));
    }

    #[test]
    fn update_refuses_an_engine_it_cannot_read_from_the_editor() {
        for dependency in [
            "concerto-editor = { workspace = true }\n",
            "concerto-editor = { path = \"../somewhere/else\" }\n",
            "concerto-editor = { version = \"1\", git = \"https://example.com/e\" }\n",
            "concerto-editor = { version = \"1\", registry = \"corp\" }\n",
            "serde = \"1\"\n",
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let dir = tmp.path().join("g");
            write_game(&dir);
            let manifest = format!("[dependencies]\n{dependency}");
            write(dir.join("editor/Cargo.toml"), &manifest);

            let error = update_editor(&dir, None).unwrap_err();

            assert!(format!("{error:#}").contains("--engine"), "{error:#}");
            assert_eq!(read(dir.join("editor/Cargo.toml")), manifest);
        }
    }

    #[test]
    fn update_refuses_a_missing_editor_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("g");
        write_game(&dir);

        let error = update_editor(&dir, None).unwrap_err();

        assert!(format!("{error:#}").contains("editor"), "{error:#}");
        assert!(!dir.join("editor").exists());
    }

    #[test]
    fn the_parser_reads_update() {
        assert_eq!(
            parse(["concerto", "editor", "x", "--update"]).unwrap(),
            Command::UpdateEditor(PathBuf::from("x"), None)
        );
        assert_eq!(
            parse(["editor", "--update", "x", "--engine", "e"]).unwrap(),
            Command::UpdateEditor(
                PathBuf::from("x"),
                Some(Engine::Checkout(PathBuf::from("e")))
            )
        );
        assert!(parse(["new", "x", "--update"]).is_err());
    }

    #[test]
    fn a_new_project_is_a_project_the_editor_accepts() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("g");

        new_project(&dir, Engine::Published).unwrap();

        assert!(dir.is_dir());
        assert!(dir.join("content").is_dir());
        assert!(dir.join("content/.gitkeep").is_file());
        assert!(!dir.join("content/.registry.toml").exists());
    }
}
