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

const EDITOR_FILES: &[(&str, &str)] = &[
    (
        "editor/Cargo.toml",
        include_str!("../templates/editor/Cargo.toml.template"),
    ),
    (
        "editor/src/main.rs",
        include_str!("../templates/editor/src/main.rs"),
    ),
];

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
pub const USAGE: &str = "usage: cargo concerto new <dir> [--engine <path>]\n       cargo concerto editor <dir> [--engine <path>]";

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
    let mut engine = Engine::Published;
    while let Some(arg) = args.next() {
        if arg == "--engine" {
            let path = args.next().context("--engine needs a path")?;
            engine = Engine::Checkout(PathBuf::from(path));
        } else if arg.starts_with('-') {
            bail!("unknown option `{arg}`");
        } else if dir.is_none() {
            dir = Some(PathBuf::from(arg));
        } else {
            bail!("unexpected argument `{arg}`");
        }
    }
    let dir = dir.with_context(|| format!("`{command}` needs a directory"))?;
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
    let mut report = Report::default();
    let written = GAME_FILES
        .iter()
        .try_for_each(|(path, template)| {
            write_file(&dir.join(path), &render(template, &vars), &mut report)
        })
        .and_then(|()| write_editor(&dir, &name, &crate_name, checkout.as_deref(), &mut report))
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
    ensure!(
        !dir.join("editor").exists(),
        "{} already exists",
        dir.join("editor").display()
    );
    let checkout = resolve(engine)?;
    let workspace = workspace_of(&dir)?;
    let mut report = Report::default();
    let added =
        write_editor(&dir, &name, &crate_name, checkout.as_deref(), &mut report).and_then(|()| {
            match workspace {
                Some(workspace) => {
                    join_workspace(&dir, workspace, &[dir.join("editor")], &mut report)
                }
                None => declare_own_workspace(&dir, &mut report),
            }
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
        "the editor calls `{crate_name}::game`; the game's library must export `pub fn game(app: &mut App)`"
    ));
    warn_if_published(checkout.as_deref(), &mut report);
    Ok(report)
}

fn write_editor(
    dir: &Path,
    name: &str,
    crate_name: &str,
    checkout: Option<&Path>,
    report: &mut Report,
) -> Result<()> {
    let editor = dependency(checkout, "crates/editor", &dir.join("editor"), dir);
    let vars = [
        ("name", name),
        ("crate", crate_name),
        ("concerto_editor", editor.as_str()),
    ];
    for (path, template) in EDITOR_FILES {
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
    let valid = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && !KEYWORDS.contains(&crate_name.as_str())
        && !RESERVED.contains(&crate_name.as_str());
    ensure!(
        valid,
        "`{name}` cannot be a package name: use letters, digits, `-` and `_`, do not start with a digit, and do not use a Rust keyword, `test`, a Cargo build directory name (`build`, `deps`, `examples`, `incremental`), or the name of an engine crate the project depends on"
    );
    Ok(())
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
