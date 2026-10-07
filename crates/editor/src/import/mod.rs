//! Importing source files into the open project.

pub mod dialog;

use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::thread::JoinHandle;

use anyhow::{Context, bail};
use concerto_app::{App, Plugin, schedule_groups::Update};
use concerto_ecs::{ResMut, Resource};
use concerto_import::ImportedAsset;
use concerto_import::config::ContentConfig;

use crate::project::{EditorCommand, EditorCommands, ProjectState};

/// Copies a source into the project at `destination`, then imports it.
pub fn import_source_into(
    source: &Path,
    destination: &str,
    project_root: &Path,
) -> anyhow::Result<Vec<ImportedAsset>> {
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if !concerto_import::supported_extension(extension) {
        bail!("no importer handles '.{extension}'");
    }
    let config = ContentConfig::load_or_default(project_root)?;
    let target = project_root.join(destination);
    let parent = match target.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => project_root,
    };
    std::fs::create_dir_all(parent).with_context(|| format!("creating '{}'", parent.display()))?;
    let same = source
        .canonicalize()
        .ok()
        .zip(target.canonicalize().ok())
        .is_some_and(|(source, target)| source == target);
    if !same {
        copy_atomically(source, &target, parent)?;
    }
    concerto_import::import_source(&target, project_root, &config)
}

fn copy_atomically(source: &Path, target: &Path, parent: &Path) -> anyhow::Result<()> {
    let context = || format!("copying '{}' to '{}'", source.display(), target.display());
    let mut reader = std::fs::File::open(source).with_context(context)?;
    let mut staged = tempfile::NamedTempFile::new_in(parent).with_context(context)?;
    std::io::copy(&mut reader, &mut staged).with_context(context)?;
    staged.flush().with_context(context)?;
    staged
        .persist(target)
        .map_err(|error| error.error)
        .with_context(context)?;
    Ok(())
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
        row.state = row_state(row, project_root, counts[&normalised(&row.destination)]);
    }
}

pub(crate) fn destinations_clash(rows: &[StagedImport]) -> bool {
    destination_counts(rows).values().any(|&count| count > 1)
}

fn destination_counts(rows: &[StagedImport]) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for row in rows {
        *counts.entry(normalised(&row.destination)).or_default() += 1;
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

fn row_state(row: &StagedImport, project_root: &Path, occurrences: usize) -> RowState {
    if row.destination.is_empty() || row.destination.ends_with('/') {
        return RowState::Rejected("needs a file name".into());
    }
    let destination = Path::new(&row.destination);
    if destination.is_absolute()
        || destination
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return RowState::Rejected("must stay inside the project, without '..'".into());
    }
    if occurrences > 1 {
        return RowState::Rejected(
            "another file claims this destination; it is listed twice".into(),
        );
    }
    let source_extension = extension_of(&row.source);
    if !concerto_import::supported_extension(&source_extension) {
        return RowState::Rejected(format!("no importer handles '.{source_extension}'"));
    }
    if extension_of(destination) != source_extension {
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
    pending: VecDeque<StagedImport>,
    job: Option<JoinHandle<Result<Vec<ImportedAsset>, String>>>,
    current: Option<String>,
    failures: Vec<String>,
    done: usize,
    total: usize,
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
        self.pending.extend(rows);
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

/// Runs queued imports one at a time and refreshes the catalogue after each.
pub struct ImportPlugin;

impl Plugin for ImportPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ImportQueue::default());
        app.insert_resource(ImportStaging::default());
        app.register_plugin(dialog::DialogPlugin);
        app.add_system(Update, finish_import);
        app.add_system(Update, drive_imports);
    }
}

fn drive_imports(mut queue: ResMut<ImportQueue>, mut state: ResMut<ProjectState>) {
    if queue.job.is_some() {
        return;
    }
    let Some(root) = state.project.as_ref().map(|project| project.root.clone()) else {
        return;
    };
    let Some(row) = queue.pending.pop_front() else {
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
        import_source_into(&row.source, &row.destination, &root)
            .map_err(|error| format!("{error:#}"))
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
    let result = queue
        .job
        .take()
        .expect("the job was just observed as finished")
        .join()
        .unwrap_or_else(|_| Err("the import worker panicked".into()));
    let name = queue.current.take().unwrap_or_default();
    queue.done += 1;
    let headline = match result {
        Ok(written) => {
            commands.0.push_back(EditorCommand::RefreshCatalogue);
            format!("Imported {name} · {} assets", written.len())
        }
        Err(error) => {
            queue.failures.push(format!("{name} ({error})"));
            format!("Could not import {name}")
        }
    };
    state.status = if queue.failures.is_empty() {
        headline
    } else {
        format!("{headline} · failed: {}", queue.failures.join("; "))
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source_dir() -> tempfile::TempDir {
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../obj/tests/fixtures/square.obj");
        let text = std::fs::read_to_string(fixture).expect("the obj fixture exists");
        let dir = tempfile::tempdir().expect("tempdir");
        let without_materials: String = text
            .lines()
            .filter(|line| !line.starts_with("mtllib") && !line.starts_with("usemtl"))
            .map(|line| format!("{line}\n"))
            .collect();
        std::fs::write(dir.path().join("square.obj"), without_materials).expect("write source");
        dir
    }

    #[test]
    fn an_external_source_is_copied_to_its_destination_and_imported() {
        let source = source_dir();
        let project = tempfile::tempdir().expect("tempdir");
        let written = import_source_into(
            &source.path().join("square.obj"),
            "assets/square.obj",
            project.path(),
        )
        .expect("import succeeds");
        assert!(
            project.path().join("assets/square.obj").is_file(),
            "the source is copied into the project"
        );
        assert!(!written.is_empty(), "the import emits at least one asset");
        assert!(
            project.path().join(&written[0].address).is_file(),
            "the content asset is written at its registered address"
        );
    }

    #[test]
    fn a_destination_at_the_project_root_needs_no_parent_directory() {
        let source = source_dir();
        let project = tempfile::tempdir().expect("tempdir");
        import_source_into(
            &source.path().join("square.obj"),
            "square.obj",
            project.path(),
        )
        .expect("a destination with no parent directory still imports");
        assert!(project.path().join("square.obj").is_file());
    }

    #[test]
    fn a_source_already_at_its_destination_is_left_in_place() {
        let source = source_dir();
        let project = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(project.path().join("assets")).expect("assets");
        let inside = project.path().join("assets/square.obj");
        std::fs::copy(source.path().join("square.obj"), &inside).expect("seed the source");
        let before = std::fs::metadata(&inside).expect("metadata").len();
        import_source_into(&inside, "assets/square.obj", project.path()).expect("import succeeds");
        assert_eq!(
            std::fs::metadata(&inside).expect("metadata").len(),
            before,
            "an in-project source at its own destination is not copied over itself"
        );
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
    fn importing_the_same_source_twice_keeps_every_asset_id() {
        let source = source_dir();
        let project = tempfile::tempdir().expect("tempdir");
        let first = import_source_into(
            &source.path().join("square.obj"),
            "assets/square.obj",
            project.path(),
        )
        .expect("first import");
        let second = import_source_into(
            &source.path().join("square.obj"),
            "assets/square.obj",
            project.path(),
        )
        .expect("second import");
        let ids = |assets: &[concerto_import::ImportedAsset]| {
            assets.iter().map(|a| a.asset_id).collect::<Vec<_>>()
        };
        assert_eq!(
            ids(&first),
            ids(&second),
            "re-importing reuses the sidecar, so asset identities are stable"
        );
    }

    #[test]
    fn a_copy_leaves_no_staging_files_behind() {
        let source = source_dir();
        let project = tempfile::tempdir().expect("tempdir");
        import_source_into(
            &source.path().join("square.obj"),
            "assets/square.obj",
            project.path(),
        )
        .expect("import");
        let names: Vec<_> = std::fs::read_dir(project.path().join("assets"))
            .expect("assets")
            .map(|entry| entry.expect("entry").file_name())
            .collect();
        assert!(
            names
                .iter()
                .all(|name| !name.to_string_lossy().starts_with(".tmp")),
            "no staging file remains, got: {names:?}"
        );
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
