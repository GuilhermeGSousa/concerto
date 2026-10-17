//! Importing source files into the open project.

pub mod dialog;

use std::collections::{HashMap, VecDeque};
use std::path::{Component, Path, PathBuf};
use std::thread::JoinHandle;

use concerto_app::{App, Plugin, schedule_groups::Update};
use concerto_ecs::{IntoSystemConfig, ResMut, Resource};
use concerto_import::ImportedSource;
use concerto_import::config::ContentConfig;

use crate::project::{EditorCommand, EditorCommands, ProjectState};

/// Imports a source into the project at `destination`, bringing the files it refers to along.
pub fn import_source_into(
    source: &Path,
    destination: &str,
    project_root: &Path,
) -> anyhow::Result<ImportedSource> {
    let config = ContentConfig::load_or_default(project_root)?;
    concerto_import::import_source_into(source, destination, project_root, &config)
}

/// A file chosen for import, and where it will be copied to.
#[derive(Debug, Clone)]
pub struct StagedImport {
    pub source: PathBuf,
    /// Project-relative, e.g. `assets/hero.glb`.
    pub destination: String,
    pub state: RowState,
}

/// What importing a row would do, or why it cannot be imported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowState {
    New,
    Replaces,
    Rejected(String),
}

impl StagedImport {
    /// Whether this row may be queued.
    pub fn importable(&self) -> bool {
        !matches!(self.state, RowState::Rejected(_))
    }
}

/// Proposes a destination under `assets/` for each picked source.
pub fn stage_sources(sources: Vec<PathBuf>) -> Vec<StagedImport> {
    sources
        .into_iter()
        .map(|source| {
            let name = source
                .file_name()
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_default();
            StagedImport {
                destination: format!("assets/{name}"),
                source,
                state: RowState::New,
            }
        })
        .collect()
}

/// Re-decides every row's state against the project on disk.
pub fn validate_rows(rows: &mut [StagedImport], project_root: &Path) {
    let counts = destination_counts(rows);
    for row in rows.iter_mut() {
        let occurrences = counts
            .get(&normalised(&row.destination))
            .copied()
            .unwrap_or(0);
        row.state = row_state(row, project_root, occurrences);
    }
}

pub(crate) fn destinations_clash(rows: &[StagedImport]) -> bool {
    destination_counts(rows).values().any(|&count| count > 1)
}

fn destination_counts(rows: &[StagedImport]) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for row in rows {
        if placement_problem(&row.destination).is_none() {
            *counts.entry(normalised(&row.destination)).or_default() += 1;
        }
    }
    counts
}

fn normalised(destination: &str) -> String {
    Path::new(destination)
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

fn placement_problem(destination: &str) -> Option<&'static str> {
    if destination.is_empty() || destination.ends_with('/') {
        return Some("needs a file name");
    }
    let destination = Path::new(destination);
    let inside = !destination.is_absolute()
        && destination
            .components()
            .all(|part| matches!(part, Component::Normal(_)));
    (!inside).then_some("outside the project")
}

fn row_state(row: &StagedImport, project_root: &Path, occurrences: usize) -> RowState {
    if let Some(problem) = placement_problem(&row.destination) {
        return RowState::Rejected(problem.into());
    }
    if occurrences > 1 {
        return RowState::Rejected("listed twice".into());
    }
    let source_extension = extension_of(&row.source);
    if !concerto_import::supported_extension(&source_extension) {
        return RowState::Rejected(format!("no importer for '.{source_extension}'"));
    }
    if extension_of(Path::new(&row.destination)) != source_extension {
        return RowState::Rejected(format!("extension must stay '.{source_extension}'"));
    }
    if project_root.join(&row.destination).exists() {
        RowState::Replaces
    } else {
        RowState::New
    }
}

fn extension_of(path: &Path) -> String {
    path.extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// Rows awaiting the user's confirmation, and whether the dialog is shown.
#[derive(Resource, Default)]
pub struct ImportStaging {
    pub rows: Vec<StagedImport>,
    pub visible: bool,
}

/// Queued imports and the single worker running one of them.
#[derive(Resource, Default)]
pub struct ImportQueue {
    pending: VecDeque<QueuedImport>,
    job: Option<JoinHandle<Result<ImportedSource, String>>>,
    current: Option<String>,
    failures: Vec<(String, String)>,
    done: usize,
    total: usize,
}

struct QueuedImport {
    row: StagedImport,
    root: Option<PathBuf>,
}

impl ImportQueue {
    /// Queues every importable row; rejected rows are dropped.
    pub fn enqueue(&mut self, rows: Vec<StagedImport>) {
        let rows: Vec<_> = rows.into_iter().filter(StagedImport::importable).collect();
        if rows.is_empty() {
            return;
        }
        if self.job.is_none() && self.pending.is_empty() {
            self.done = 0;
            self.total = 0;
            self.failures.clear();
        }
        self.total += rows.len();
        self.pending
            .extend(rows.into_iter().map(|row| QueuedImport { row, root: None }));
    }

    pub(crate) fn bind(&mut self, project_root: &Path) {
        for queued in self.pending.iter_mut() {
            queued
                .root
                .get_or_insert_with(|| project_root.to_path_buf());
        }
    }

    /// Whether a worker thread is importing a row right now.
    pub fn is_running(&self) -> bool {
        self.job.is_some()
    }

    /// How many rows are still waiting to start.
    pub fn remaining(&self) -> usize {
        self.pending.len()
    }
}

const SUPPORTED_EXTENSIONS: &[&str] = &["gltf", "glb", "obj", "png", "jpg", "jpeg"];

/// The native file picker running on its own thread.
#[derive(Resource, Default)]
pub struct ImportPicker {
    job: Option<JoinHandle<Option<Vec<PathBuf>>>>,
    requested: bool,
}

impl ImportPicker {
    /// Asks for a picker on the next frame.
    pub fn request(&mut self) {
        self.requested = true;
    }

    /// Whether a picker is currently open.
    pub fn is_picking(&self) -> bool {
        self.job.is_some()
    }
}

/// Turns the picker's result into validated staging rows.
pub fn accept_picked(
    sources: Option<Vec<PathBuf>>,
    staging: &mut ImportStaging,
    state: &mut ProjectState,
) {
    let Some(sources) = sources else {
        state.status = "Import cancelled.".into();
        return;
    };
    let Some(root) = state.project.as_ref().map(|project| project.root.clone()) else {
        return;
    };
    let mut rows = stage_sources(sources);
    keep_project_sources_in_place(&mut rows, &root);
    validate_rows(&mut rows, &root);
    staging.rows = rows;
    staging.visible = !staging.rows.is_empty();
}

fn keep_project_sources_in_place(rows: &mut [StagedImport], project_root: &Path) {
    let Ok(root) = project_root.canonicalize() else {
        return;
    };
    for row in rows {
        let Ok(source) = row.source.canonicalize() else {
            continue;
        };
        if let Ok(inside) = source.strip_prefix(&root) {
            row.destination = normalised(&inside.to_string_lossy());
        }
    }
}

fn drive_picker(
    mut picker: ResMut<ImportPicker>,
    mut staging: ResMut<ImportStaging>,
    mut state: ResMut<ProjectState>,
) {
    if picker.job.as_ref().is_some_and(|job| job.is_finished()) {
        let finished = picker
            .job
            .take()
            .expect("the job was just observed as finished")
            .join();
        match finished {
            Ok(picked) => accept_picked(picked, &mut staging, &mut state),
            Err(_) => state.status = "The file picker failed.".into(),
        }
    }
    if !std::mem::take(&mut picker.requested)
        || picker.job.is_some()
        || staging.visible
        || state.project.is_none()
    {
        return;
    }
    picker.job = Some(std::thread::spawn(|| {
        rfd::FileDialog::new()
            .add_filter("Assets", SUPPORTED_EXTENSIONS)
            .pick_files()
    }));
}

/// Runs queued imports one at a time and refreshes the catalogue after each.
pub struct ImportPlugin;

impl Plugin for ImportPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ImportQueue::default());
        app.insert_resource(ImportStaging::default());
        app.insert_resource(ImportPicker::default());
        app.register_plugin(dialog::DialogPlugin);
        app.add_system(Update, (finish_import, drive_imports).chain());
        app.add_system(Update, drive_picker);
    }
}

fn drive_imports(mut queue: ResMut<ImportQueue>, mut state: ResMut<ProjectState>) {
    let root = state.project.as_ref().map(|project| project.root.clone());
    if let Some(root) = &root {
        queue.bind(root);
    }
    if queue.job.is_some() {
        return;
    }
    let queued = queue.pending.len();
    queue
        .pending
        .retain(|queued| queued.root.is_some() && queued.root == root);
    let dropped = queued - queue.pending.len();
    if dropped > 0 {
        queue.total = queue.total.saturating_sub(dropped);
        let reason = if root.is_some() {
            "another project was opened"
        } else {
            "no project is open"
        };
        let plural = if dropped == 1 { "" } else { "s" };
        state.status = format!("Dropped {dropped} queued import{plural}: {reason}");
        log::warn!("{}", state.status);
        return;
    }
    let (Some(root), Some(QueuedImport { row, .. })) = (root, queue.pending.pop_front()) else {
        return;
    };
    let name = row
        .source
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| row.destination.clone());
    state.status = format!("Importing {name} ({} of {})…", queue.done + 1, queue.total);
    queue.current = Some(name);
    queue.job = Some(std::thread::spawn(move || {
        import_source_into(&row.source, &row.destination, &root).map_err(|error| {
            let error = format!("{error:#}");
            log::error!(
                "importing '{}' to '{}' in '{}' failed: {error}",
                row.source.display(),
                row.destination,
                root.display()
            );
            failure_summary(&error, &root)
        })
    }));
}

fn finish_import(
    mut queue: ResMut<ImportQueue>,
    mut state: ResMut<ProjectState>,
    mut commands: ResMut<EditorCommands>,
) {
    if !queue.job.as_ref().is_some_and(|job| job.is_finished()) {
        return;
    }
    let name = queue.current.take().unwrap_or_default();
    let result = queue
        .job
        .take()
        .expect("the job was just observed as finished")
        .join()
        .unwrap_or_else(|_| {
            log::error!("the import worker panicked while importing '{name}'");
            Err("the import worker panicked".into())
        });
    queue.done += 1;
    let imported = match result {
        Ok(imported) => {
            commands.0.push_back(EditorCommand::RefreshCatalogue);
            let siblings = match imported.siblings.len() {
                0 => String::new(),
                1 => " · 1 sibling file".into(),
                count => format!(" · {count} sibling files"),
            };
            format!(
                "Imported {name} · {} assets{siblings}",
                imported.assets.len()
            )
        }
        Err(reason) => {
            queue.failures.push((name, reason));
            String::new()
        }
    };
    state.status = if queue.failures.is_empty() {
        imported
    } else {
        failure_status(&queue.failures, queue.total)
    };
}

fn failure_status(failures: &[(String, String)], total: usize) -> String {
    let Some((name, reason)) = failures.first() else {
        return String::new();
    };
    let mut status = format!("{} of {total} failed: {name} — {reason}", failures.len());
    if failures.len() > 1 {
        status.push_str(&format!(" (+{} more, see log)", failures.len() - 1));
    }
    status
}

fn failure_summary(error: &str, project_root: &Path) -> String {
    let cause = error
        .strip_prefix("importing '")
        .and_then(|rest| rest.split_once("': "))
        .map_or(error, |(_, cause)| cause);
    let mut summary = debug_fields(cause).unwrap_or_else(|| cause.to_owned());
    let roots = [project_root.canonicalize().ok(), Some(project_root.into())];
    for root in roots.into_iter().flatten() {
        let prefix = format!("{}{}", root.display(), std::path::MAIN_SEPARATOR);
        summary = summary.replace(&prefix, "");
    }
    if let Some(at) = summary
        .rfind(" (os error ")
        .filter(|_| summary.ends_with(')'))
    {
        summary.truncate(at);
    }
    summary
}

fn debug_fields(error: &str) -> Option<String> {
    let (_, fields) = error.strip_suffix(" }")?.split_once(" { ")?;
    let (subject, message) = fields.split_once(", message: ")?;
    let (_, subject) = subject.split_once(": ")?;
    Some(format!("{}: {}", unquoted(subject), unquoted(message)))
}

fn unquoted(text: &str) -> String {
    text.strip_prefix('"')
        .and_then(|text| text.strip_suffix('"'))
        .unwrap_or(text)
        .replace("\\\"", "\"")
        .replace("\\\\", "\\")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_picker_filter_lists_only_extensions_an_importer_handles() {
        for extension in SUPPORTED_EXTENSIONS {
            assert!(
                concerto_import::supported_extension(extension),
                "{extension}"
            );
        }
    }

    #[test]
    fn every_common_asset_extension_an_importer_accepts_is_in_the_picker_filter() {
        let candidates = [
            "png", "jpg", "jpeg", "gltf", "glb", "obj", "bmp", "tga", "gif", "webp", "tiff", "tif",
            "hdr", "exr", "dds", "ktx2", "fbx", "dae", "ply", "stl", "usd", "usdz", "blend", "3ds",
            "mtl",
        ];
        for extension in candidates {
            if concerto_import::supported_extension(extension) {
                assert!(
                    SUPPORTED_EXTENSIONS.contains(&extension),
                    "an importer handles '.{extension}' but the picker filter omits it"
                );
            }
        }
    }

    fn source_dir() -> tempfile::TempDir {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../obj/tests/fixtures");
        let dir = tempfile::tempdir().expect("tempdir");
        for name in ["square.obj", "square.mtl"] {
            std::fs::copy(fixtures.join(name), dir.path().join(name)).expect("copy the fixture");
        }
        dir
    }

    #[test]
    fn an_external_source_is_imported_with_the_files_it_refers_to() {
        let source = source_dir();
        let project = tempfile::tempdir().expect("tempdir");
        let imported = import_source_into(
            &source.path().join("square.obj"),
            "assets/props/square.obj",
            project.path(),
        )
        .expect("import succeeds");
        assert!(project.path().join("assets/props/square.obj").is_file());
        assert!(
            project.path().join("assets/props/square.mtl").is_file(),
            "the material library comes along"
        );
        assert_eq!(imported.siblings, ["assets/props/square.mtl"]);
        assert!(!imported.assets.is_empty());
        for asset in &imported.assets {
            assert!(project.path().join(&asset.address).is_file());
        }
    }

    #[test]
    fn the_project_content_config_decides_where_assets_are_written() {
        let source = source_dir();
        let project = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            project.path().join("content.toml"),
            "root = \"cooked\"\nextension = \"bin\"\n",
        )
        .expect("write config");
        let imported = import_source_into(
            &source.path().join("square.obj"),
            "assets/square.obj",
            project.path(),
        )
        .expect("import succeeds");
        assert!(!imported.assets.is_empty());
        for asset in &imported.assets {
            assert!(
                asset.address.starts_with("cooked/") && asset.address.ends_with(".bin"),
                "got {}",
                asset.address
            );
        }
    }

    #[test]
    fn a_source_already_at_its_destination_is_left_in_place() {
        let source = source_dir();
        let project = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(project.path().join("assets")).expect("assets");
        for name in ["square.obj", "square.mtl"] {
            std::fs::copy(
                source.path().join(name),
                project.path().join("assets").join(name),
            )
            .expect("seed the source");
        }
        let inside = project.path().join("assets/square.obj");
        let imported =
            import_source_into(&inside, "assets/square.obj", project.path()).expect("import");
        assert!(imported.siblings.is_empty(), "nothing needed copying");
        assert!(!imported.assets.is_empty());
    }

    #[test]
    fn an_unsupported_extension_errors_before_copying_anything() {
        let project = tempfile::tempdir().expect("tempdir");
        let source = project.path().join("notes.txt");
        std::fs::write(&source, b"not an asset").expect("write");
        let error = import_source_into(&source, "assets/notes.txt", project.path())
            .expect_err("an unsupported extension is refused");
        assert!(
            format!("{error:#}").contains("txt"),
            "the error names the extension, got: {error:#}"
        );
        assert!(
            !project.path().join("assets/notes.txt").exists(),
            "nothing is copied when the extension is refused"
        );
    }

    #[test]
    fn a_destination_the_dialog_would_reject_is_refused_before_anything_is_written() {
        let source = source_dir();
        let outer = tempfile::tempdir().expect("tempdir");
        let project = outer.path().join("project");
        std::fs::create_dir_all(&project).expect("project");
        let absolute = outer.path().join("absolute/square.obj");
        for destination in [
            "../escape/square.obj",
            "assets/../../escape/square.obj",
            absolute.to_str().expect("utf-8 tempdir"),
            "assets/square.png",
            "assets/",
            "",
        ] {
            assert!(
                import_source_into(&source.path().join("square.obj"), destination, &project)
                    .is_err(),
                "'{destination}' must be refused"
            );
        }
        assert!(!outer.path().join("escape").exists());
        assert!(!outer.path().join("absolute").exists());
        assert_eq!(
            std::fs::read_dir(&project).expect("project").count(),
            0,
            "a refused destination writes nothing into the project"
        );
    }

    #[test]
    fn replacing_a_source_with_new_bytes_keeps_every_asset_id() {
        let source = source_dir();
        let path = source.path().join("square.obj");
        let project = tempfile::tempdir().expect("tempdir");
        let first =
            import_source_into(&path, "assets/square.obj", project.path()).expect("first import");
        let moved = std::fs::read_to_string(&path)
            .expect("source")
            .replace("v 1.0 1.0 0.0", "v 2.0 2.0 0.0");
        std::fs::write(&path, &moved).expect("change the source");
        let second =
            import_source_into(&path, "assets/square.obj", project.path()).expect("second import");
        assert_eq!(
            std::fs::read_to_string(project.path().join("assets/square.obj")).expect("copy"),
            moved,
            "the destination holds the new bytes"
        );
        let ids = |assets: &[concerto_import::ImportedAsset]| {
            assets.iter().map(|a| a.asset_id).collect::<Vec<_>>()
        };
        assert!(!first.assets.is_empty());
        assert_eq!(ids(&first.assets), ids(&second.assets));
    }

    #[test]
    fn an_import_records_its_asset_ids_in_the_registry_file() {
        use concerto_foundation::assets::content::{AssetRegistry, REGISTRY_FILE_NAME};
        let source = source_dir();
        let project = tempfile::tempdir().expect("tempdir");
        let imported = import_source_into(
            &source.path().join("square.obj"),
            "assets/square.obj",
            project.path(),
        )
        .expect("import");
        assert!(project.path().join(REGISTRY_FILE_NAME).is_file());
        let registry = AssetRegistry::load(project.path()).expect("registry");
        assert!(!imported.assets.is_empty());
        for asset in &imported.assets {
            assert_eq!(registry.get(asset.asset_id), Some(asset.address.as_str()));
        }
    }

    #[test]
    fn an_importer_error_is_summarised_without_its_debug_form_or_the_project_root() {
        let summary = failure_summary(
            "importing '/home/me/game/assets/square.obj': SourceUnreadable { source_path: \"/home/me/game/assets/square.mtl\", message: \"No such file or directory (os error 2)\" }",
            Path::new("/home/me/game"),
        );
        assert_eq!(summary, "assets/square.mtl: No such file or directory");
    }

    #[test]
    fn an_error_in_another_shape_is_kept_with_project_paths_made_relative() {
        let summary = failure_summary(
            "writing '/home/me/game/assets/a \"b\".obj': Permission denied (os error 13)",
            Path::new("/home/me/game"),
        );
        assert_eq!(summary, "writing 'assets/a \"b\".obj': Permission denied");
        assert_eq!(
            failure_summary(
                "importing '/p/a.obj': SerializationFailed { sub_asset_name: \"mesh_0\", message: \"a \\\"quoted\\\" word\" }",
                Path::new("/p"),
            ),
            "mesh_0: a \"quoted\" word"
        );
    }

    #[test]
    fn a_batch_status_names_the_first_failure_and_counts_the_rest() {
        let one = vec![("a.obj".to_string(), "a.mtl: missing".to_string())];
        assert_eq!(
            failure_status(&one, 3),
            "1 of 3 failed: a.obj — a.mtl: missing"
        );
        let mut three = one;
        three.push(("b.obj".into(), "second".into()));
        three.push(("c.obj".into(), "third".into()));
        assert_eq!(
            failure_status(&three, 5),
            "3 of 5 failed: a.obj — a.mtl: missing (+2 more, see log)"
        );
    }

    #[test]
    fn the_rejection_reasons_lead_with_the_word_that_tells_them_apart() {
        let project = tempfile::tempdir().expect("tempdir");
        let mut staged = rows(&[
            ("/tmp/a.obj", ""),
            ("/tmp/b.obj", "../b.obj"),
            ("/one/c.obj", "assets/c.obj"),
            ("/two/c.obj", "assets/c.obj"),
            ("/tmp/d.glb", "assets/d.png"),
            ("/tmp/e.txt", "assets/e.txt"),
        ]);
        validate_rows(&mut staged, project.path());
        let reasons: Vec<_> = staged.iter().map(rejection).collect();
        assert_eq!(
            reasons,
            [
                "needs a file name",
                "outside the project",
                "listed twice",
                "listed twice",
                "extension must stay '.glb'",
                "no importer for '.txt'",
            ]
        );
    }

    #[test]
    fn rows_already_rejected_for_their_own_destination_never_count_as_a_clash() {
        let project = tempfile::tempdir().expect("tempdir");
        let mut staged = rows(&[
            ("/tmp/a.obj", ""),
            ("/tmp/b.obj", ""),
            ("/tmp/c.obj", "../shared.obj"),
            ("/tmp/d.obj", "../shared.obj"),
            ("/tmp/e.obj", "assets/e.obj"),
        ]);
        validate_rows(&mut staged, project.path());
        assert!(!destinations_clash(&staged));
        assert!(rejection(&staged[0]).contains("file name"));
        assert!(rejection(&staged[1]).contains("file name"));
        assert!(rejection(&staged[2]).contains("project"));
        assert!(rejection(&staged[3]).contains("project"));
        assert!(staged[4].importable());
    }

    fn rows(pairs: &[(&str, &str)]) -> Vec<StagedImport> {
        pairs
            .iter()
            .map(|(source, destination)| StagedImport {
                source: PathBuf::from(source),
                destination: (*destination).to_string(),
                state: RowState::New,
            })
            .collect()
    }

    fn rejection(row: &StagedImport) -> &str {
        match &row.state {
            RowState::Rejected(reason) => reason,
            other => panic!("expected a rejection, got {other:?}"),
        }
    }

    #[test]
    fn staging_prefills_a_destination_under_assets() {
        let staged = stage_sources(vec![PathBuf::from("/home/someone/hero.glb")]);
        assert_eq!(staged.len(), 1);
        assert_eq!(staged[0].destination, "assets/hero.glb");
        assert!(matches!(staged[0].state, RowState::New));
    }

    #[test]
    fn a_destination_escaping_the_project_is_rejected() {
        let project = tempfile::tempdir().expect("tempdir");
        let mut staged = rows(&[
            ("/tmp/a.obj", "../outside/a.obj"),
            ("/tmp/b.obj", "/etc/b.obj"),
        ]);
        validate_rows(&mut staged, project.path());
        assert!(rejection(&staged[0]).contains("project"));
        assert!(rejection(&staged[1]).contains("project"));
    }

    #[test]
    fn a_destination_without_a_file_name_is_rejected() {
        let project = tempfile::tempdir().expect("tempdir");
        let mut staged = rows(&[("/tmp/a.obj", ""), ("/tmp/b.obj", "assets/")]);
        validate_rows(&mut staged, project.path());
        assert!(rejection(&staged[0]).contains("file name"));
        assert!(rejection(&staged[1]).contains("file name"));
    }

    #[test]
    fn a_destination_that_changes_the_extension_is_rejected() {
        let project = tempfile::tempdir().expect("tempdir");
        let mut staged = rows(&[("/tmp/hero.glb", "assets/hero.png")]);
        validate_rows(&mut staged, project.path());
        assert!(
            rejection(&staged[0]).contains("extension"),
            "the importer is chosen from the destination extension, so it must match the source"
        );
    }

    #[test]
    fn an_unsupported_source_is_rejected_by_its_extension() {
        let project = tempfile::tempdir().expect("tempdir");
        let mut staged = rows(&[("/tmp/notes.txt", "assets/notes.txt")]);
        validate_rows(&mut staged, project.path());
        assert!(rejection(&staged[0]).contains("txt"));
    }

    #[test]
    fn an_occupied_destination_is_marked_as_a_replacement_not_a_rejection() {
        let project = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(project.path().join("assets")).expect("assets");
        std::fs::write(project.path().join("assets/hero.glb"), b"old").expect("write");
        let mut staged = rows(&[("/tmp/hero.glb", "assets/hero.glb")]);
        validate_rows(&mut staged, project.path());
        assert!(matches!(staged[0].state, RowState::Replaces));
        assert!(staged[0].importable());
    }

    #[test]
    fn two_rows_sharing_a_destination_are_both_rejected() {
        let project = tempfile::tempdir().expect("tempdir");
        let mut staged = rows(&[
            ("/one/hero.glb", "assets/hero.glb"),
            ("/two/hero.glb", "assets/hero.glb"),
        ]);
        validate_rows(&mut staged, project.path());
        assert!(rejection(&staged[0]).contains("twice"));
        assert!(rejection(&staged[1]).contains("twice"));
        assert!(!staged[0].importable() && !staged[1].importable());
    }

    #[test]
    fn destinations_that_normalise_to_the_same_path_collide() {
        let project = tempfile::tempdir().expect("tempdir");
        let mut staged = rows(&[
            ("/one/a.obj", "assets/a.obj"),
            ("/two/a.obj", "assets//a.obj"),
        ]);
        validate_rows(&mut staged, project.path());
        assert!(rejection(&staged[0]).contains("twice"));
        assert!(rejection(&staged[1]).contains("twice"));
    }
}
