//! Importing source files into the open project.

pub mod dialog;
mod reimport;

pub use reimport::{ReimportArgs, ReimportAsset, StageReimport};

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Component, Path, PathBuf};
use std::thread::JoinHandle;

use concerto_app::{
    App, Plugin,
    schedule_groups::{LateUpdate, Update},
};
use concerto_ecs::{
    Event, IntoSystemConfig, Res, ResMut, Resource, events::event_writer::EventWriter,
    reflect::AppTypeRegistry,
};
use concerto_foundation::assets::{
    AssetId,
    registry::{AssetEntry, AssetRegistry},
};
use concerto_import::config::ContentConfig;
use concerto_import::{
    ContentScan, ImportSettings, ImportedSource, StagedSettings, importer_for, importers,
};
use concerto_mcp::{AppMcpExt, McpTool, Needs, ToolRegistry};
use concerto_reflect::{Reflect, TypeRegistry, to_json};

use crate::content::AssetRowMenu;
use crate::project::{AssetsWritten, ProjectState};

/// Imports a source into the project at `destination` with `settings`, bringing the files it refers to along.
pub fn import_source_into(
    source: &Path,
    destination: &str,
    project_root: &Path,
    settings: ImportSettings,
) -> anyhow::Result<ImportedSource> {
    let config = ContentConfig::load_or_default(project_root)?;
    concerto_import::import_source_into(source, destination, project_root, &config, settings)
}

fn reimport_into(
    asset: AssetId,
    source: Option<&Path>,
    project_root: &Path,
    settings: ImportSettings,
) -> anyhow::Result<ImportedSource> {
    let config = ContentConfig::load_or_default(project_root)?;
    concerto_import::reimport(asset, source, project_root, &config, settings)
}

/// Scans the content of the project at `project_root` once, for staging a batch of rows against it.
pub fn scan_content(project_root: &Path) -> anyhow::Result<ContentScan> {
    let config = ContentConfig::load_or_default(project_root)?;
    ContentScan::new(project_root, &config)
}

/// `settings` and `asset_settings` as TOML text, each a table of its reflected fields.
pub fn settings_text(
    settings: Option<&dyn Reflect>,
    asset_settings: Option<&dyn Reflect>,
    types: &TypeRegistry,
) -> String {
    let mut table = toml::Table::new();
    let mut add = |name: &str, value: Option<&dyn Reflect>| {
        if let Some(value) =
            value.and_then(|value| toml::Value::try_from(to_json(value, types)).ok())
        {
            table.insert(name.to_owned(), value);
        }
    };
    add("settings", settings);
    add("asset_settings", asset_settings);
    toml::to_string(&table).unwrap_or_default()
}

/// A file chosen for import, and where it will be copied to.
pub struct StagedImport {
    pub source: PathBuf,
    /// Project-relative, e.g. `assets/hero.glb`.
    pub destination: String,
    pub state: RowState,
    /// The settings importing to `destination` would use unedited, or why they cannot be read; `None` until [`stage_row`].
    pub staged: Option<Result<StagedSettings, String>>,
    /// The importer settings edited for this row; `None` imports with the import record's.
    pub settings: Option<Box<dyn Reflect>>,
    /// The asset settings edited for this row; `None` keeps the asset's own.
    pub asset_settings: Option<Box<dyn Reflect>>,
    /// The asset whose import this row imports again, with `destination` its recorded source; `None` for a new import.
    pub reimport: Option<AssetId>,
}

/// What importing a row would do, or why it cannot be imported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowState {
    New,
    Replaces,
    SourceMissing,
    Rejected(String),
}

impl StagedImport {
    /// Whether this row may be queued.
    pub fn importable(&self) -> bool {
        !matches!(self.state, RowState::Rejected(_) | RowState::SourceMissing)
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
                staged: None,
                settings: None,
                asset_settings: None,
                reimport: None,
            }
        })
        .collect()
}

/// Reads the settings `row` would import with at its destination from `scan`, dropping its edits.
pub fn stage_row(row: &mut StagedImport, scan: &anyhow::Result<ContentScan>) {
    row.settings = None;
    row.asset_settings = None;
    row.staged = Some(match scan {
        Ok(scan) => scan
            .staged_settings(&row.source, &row.destination)
            .map_err(|error| format!("{error:#}")),
        Err(error) => Err(format!("{error:#}")),
    });
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
    if row.reimport.is_some() && !row.source.is_file() {
        return RowState::SourceMissing;
    }
    let source_extension = extension_of(&row.source);
    if importer_for(&source_extension).is_none() {
        return RowState::Rejected(format!("no importer for '.{source_extension}'"));
    }
    if extension_of(Path::new(&row.destination)) != source_extension {
        return RowState::Rejected(format!("extension must stay '.{source_extension}'"));
    }
    if let Some(Err(error)) = &row.staged {
        return RowState::Rejected(error.clone());
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
    pub(crate) scan: Option<anyhow::Result<ContentScan>>,
}

/// A queued import ended: what it wrote to `destination`, or why it failed.
#[derive(Event, Clone, Debug, PartialEq)]
pub struct ImportFinished {
    pub destination: String,
    /// The settings the import used, as [`settings_text`] writes them; empty when they could not be read.
    pub settings: String,
    pub result: Result<ImportedSource, String>,
}

struct FinishedJob {
    root: PathBuf,
    settings: String,
    result: Result<ImportedSource, String>,
}

/// Queued imports and the single worker running one of them.
#[derive(Resource, Default)]
pub struct ImportQueue {
    pending: VecDeque<QueuedImport>,
    job: Option<JoinHandle<FinishedJob>>,
    current: Option<String>,
    current_destination: Option<String>,
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

/// The native file picker running on its own thread.
#[derive(Resource, Default)]
pub struct ImportPicker {
    job: Option<JoinHandle<Option<Vec<PathBuf>>>>,
    job_row: Option<usize>,
    requested: bool,
    requested_row: Option<usize>,
}

impl ImportPicker {
    /// Asks for a picker on the next frame.
    pub fn request(&mut self) {
        self.requested = true;
        self.requested_row = None;
    }

    /// Asks for a picker on the next frame whose one file becomes the source of the staged row at `row`.
    pub fn request_source(&mut self, row: usize) {
        self.requested = true;
        self.requested_row = Some(row);
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
    let scan = scan_content(&root);
    for row in &mut rows {
        stage_row(row, &scan);
    }
    validate_rows(&mut rows, &root);
    staging.rows = rows;
    staging.visible = !staging.rows.is_empty();
    staging.scan = Some(scan);
}

fn accept_picked_source(
    picked: Option<Vec<PathBuf>>,
    index: usize,
    staging: &mut ImportStaging,
    state: &ProjectState,
) {
    let (Some(source), Some(root)) = (
        picked.and_then(|picked| picked.into_iter().next()),
        state.project.as_ref().map(|project| &project.root),
    ) else {
        return;
    };
    let ImportStaging { rows, scan, .. } = staging;
    let Some(row) = rows.get_mut(index) else {
        return;
    };
    row.source = source;
    stage_row(row, scan.get_or_insert_with(|| scan_content(root)));
    validate_rows(rows, root);
}

pub(crate) fn keep_project_sources_in_place(rows: &mut [StagedImport], project_root: &Path) {
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
        match (finished, picker.job_row.take()) {
            (Ok(picked), None) => accept_picked(picked, &mut staging, &mut state),
            (Ok(picked), Some(row)) => accept_picked_source(picked, row, &mut staging, &state),
            (Err(_), _) => state.status = "The file picker failed.".into(),
        }
    }
    if !std::mem::take(&mut picker.requested) || picker.job.is_some() || state.project.is_none() {
        return;
    }
    let row = picker.requested_row.take();
    let extensions: Vec<String> = match row.and_then(|index| staging.rows.get(index)) {
        Some(staged) if staging.visible => vec![extension_of(Path::new(&staged.destination))],
        None if row.is_none() && !staging.visible => importers()
            .iter()
            .flat_map(|importer| importer.supported_extensions())
            .map(|extension| extension.to_string())
            .collect(),
        _ => return,
    };
    picker.job_row = row;
    picker.job = Some(std::thread::spawn(move || {
        let dialog = rfd::FileDialog::new().add_filter("Assets", &extensions);
        match row {
            Some(_) => dialog.pick_file().map(|picked| vec![picked]),
            None => dialog.pick_files(),
        }
    }));
}

/// Runs queued imports one at a time and adds each one's assets to the open project's registry.
pub struct ImportPlugin;

impl Plugin for ImportPlugin {
    fn build(&self, app: &mut App) {
        let types = app.main_mut().world_mut().type_registry();
        for importer in importers() {
            importer.register_types(&mut types.write());
        }
        app.insert_resource(ImportQueue::default());
        app.insert_resource(ImportStaging::default());
        app.insert_resource(ImportPicker::default());
        app.register_event::<ImportFinished>();
        if app.get_resource::<AssetRowMenu>().is_none() {
            app.insert_resource(AssetRowMenu::default());
        }
        app.get_resource_mut::<AssetRowMenu>()
            .expect("inserted above")
            .register(
                "Re-import…",
                |entry| entry.provenance.is_some(),
                |asset| StageReimport { asset },
            );
        app.add_listener(reimport::stage_reimport)
            .add_listener(reimport::queue_reimport);
        app.register_plugin(dialog::DialogPlugin);
        app.add_system(Update, (finish_import, drive_imports).chain());
        app.add_system(Update, drive_picker);
    }

    fn finish(&self, app: &mut App) {
        if app.get_resource::<ToolRegistry>().is_none() {
            return;
        }
        app.add_tool::<ReimportArgs, _>(
            McpTool::new("reimport").needs(Needs::Project),
            reimport::call_reimport,
        )
        .add_system(LateUpdate, reimport::answer_reimports);
    }
}

fn drive_imports(
    mut queue: ResMut<ImportQueue>,
    mut state: ResMut<ProjectState>,
    types: Res<AppTypeRegistry>,
) {
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
    queue.current_destination = Some(row.destination.clone());
    let types = types.clone();
    queue.job = Some(std::thread::spawn(move || {
        let used = settings_used(&row, &types);
        let settings = ImportSettings {
            settings: row.settings,
            asset_settings: row.asset_settings,
        };
        let result = match row.reimport {
            Some(asset) => {
                let picked = (row.source != root.join(&row.destination)).then_some(&row.source);
                reimport_into(asset, picked.map(PathBuf::as_path), &root, settings)
            }
            None => import_source_into(&row.source, &row.destination, &root, settings),
        }
        .map_err(|error| {
            let error = format!("{error:#}");
            log::error!(
                "importing '{}' to '{}' in '{}' failed: {error}",
                row.source.display(),
                row.destination,
                root.display()
            );
            failure_summary(&error, &root)
        });
        FinishedJob {
            root,
            settings: used,
            result,
        }
    }));
}

fn settings_used(row: &StagedImport, types: &AppTypeRegistry) -> String {
    let staged = row.staged.as_ref().and_then(|staged| staged.as_ref().ok());
    let settings = row
        .settings
        .as_deref()
        .or(staged.map(|staged| staged.settings.as_ref()));
    let asset_settings = row
        .asset_settings
        .as_deref()
        .or(staged.and_then(|staged| staged.asset_settings.as_deref()));
    settings_text(settings, asset_settings, &types.read())
}

fn finish_import(
    mut queue: ResMut<ImportQueue>,
    mut state: ResMut<ProjectState>,
    mut registry: ResMut<AssetRegistry>,
    mut written: EventWriter<AssetsWritten>,
    mut finished: EventWriter<ImportFinished>,
) {
    if !queue.job.as_ref().is_some_and(|job| job.is_finished()) {
        return;
    }
    let name = queue.current.take().unwrap_or_default();
    let destination = queue.current_destination.take().unwrap_or_default();
    let (root, settings, result) = match queue
        .job
        .take()
        .expect("the job was just observed as finished")
        .join()
    {
        Ok(job) => (Some(job.root), job.settings, job.result),
        Err(_) => {
            log::error!("the import worker panicked while importing '{name}'");
            (
                None,
                String::new(),
                Err("the import worker panicked".into()),
            )
        }
    };
    queue.done += 1;
    finished.write(ImportFinished {
        destination,
        settings,
        result: result.clone(),
    });
    let imported = match result {
        Ok(imported) => {
            if let Some(root) = &root
                && state
                    .project
                    .as_ref()
                    .is_some_and(|project| &project.root == root)
            {
                apply_to_registry(&mut registry, &imported);
                written.write(AssetsWritten {
                    ids: imported.assets.iter().map(|entry| entry.id).collect(),
                });
            }
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

fn apply_to_registry(registry: &mut AssetRegistry, imported: &ImportedSource) {
    let source_of = |entry: &AssetEntry| {
        entry
            .provenance
            .as_ref()
            .map(|provenance| provenance.source.clone())
    };
    let sources: HashSet<String> = imported.assets.iter().filter_map(source_of).collect();
    let stale: Vec<AssetId> = registry
        .iter()
        .filter(|entry| source_of(entry).is_some_and(|source| sources.contains(&source)))
        .map(|entry| entry.id)
        .collect();
    for id in stale {
        registry.remove(id);
    }
    for entry in &imported.assets {
        registry.insert(entry.clone());
    }
    for id in &imported.removed {
        registry.remove(*id);
    }
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
        for separator in ['/', '\\'] {
            summary = summary.replace(&format!("{}{separator}", root.display()), "");
        }
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
    use concerto_foundation::assets::{content::ImportProvenance, registry::EmbeddedIn};

    #[test]
    fn the_project_content_config_decides_where_assets_are_written() {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../obj/tests/fixtures");
        let source = tempfile::tempdir().expect("tempdir");
        for name in ["square.obj", "square.mtl"] {
            std::fs::copy(fixtures.join(name), source.path().join(name)).expect("copy");
        }
        let project = tempfile::tempdir().expect("tempdir");
        std::fs::write(project.path().join("content.toml"), "extension = \"bin\"\n")
            .expect("write config");
        let imported = import_source_into(
            &source.path().join("square.obj"),
            "assets/square.obj",
            project.path(),
            ImportSettings::default(),
        )
        .expect("import succeeds");
        assert!(!imported.assets.is_empty());
        for asset in &imported.assets {
            assert!(
                asset.address.starts_with("content/") && asset.address.ends_with(".bin"),
                "got {}",
                asset.address
            );
        }
    }

    struct Row {
        source: &'static str,
        destination: &'static str,
        importable: bool,
        replaces: bool,
    }

    #[test]
    fn each_row_is_new_a_replacement_or_rejected_for_where_it_would_land() {
        let project = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(project.path().join("assets")).expect("assets");
        std::fs::write(project.path().join("assets/hero.glb"), b"old").expect("write");
        let row = |source, destination, importable, replaces| Row {
            source,
            destination,
            importable,
            replaces,
        };
        let table = [
            row("/tmp/a.obj", "assets/a.obj", true, false),
            row("/tmp/hero.glb", "assets/hero.glb", true, true),
            row("/tmp/b.obj", "", false, false),
            row("/tmp/b.obj", "assets/", false, false),
            row("/tmp/c.obj", "../c.obj", false, false),
            row("/tmp/c.obj", "/etc/c.obj", false, false),
            row("/one/d.obj", "assets/d.obj", false, false),
            row("/two/d.obj", "assets//d.obj", false, false),
            row("/tmp/e.glb", "assets/e.png", false, false),
            row("/tmp/f.txt", "assets/f.txt", false, false),
        ];
        let mut staged: Vec<StagedImport> = table
            .iter()
            .map(|row| StagedImport {
                source: PathBuf::from(row.source),
                destination: row.destination.to_string(),
                state: RowState::New,
                staged: None,
                settings: None,
                asset_settings: None,
                reimport: None,
            })
            .collect();

        validate_rows(&mut staged, project.path());

        for (row, staged) in table.iter().zip(&staged) {
            assert_eq!(staged.importable(), row.importable, "{}", row.destination);
            assert_eq!(
                staged.state == RowState::Replaces,
                row.replaces,
                "{}",
                row.destination
            );
        }
    }

    #[test]
    fn an_imports_entries_replace_the_registrys_entries_for_its_source() {
        let entry = |address: &str, source: Option<&str>| AssetEntry {
            embedded_in: None,
            id: AssetId::new(),
            address: address.into(),
            kind: "Scene".into(),
            kind_version: None,
            provenance: source.map(|source| ImportProvenance {
                source: source.into(),
                sub_asset: address.into(),
            }),
        };
        let kept = entry("content/a/scene.gasset", Some("assets/a.glb"));
        let dropped = entry("content/a/old.gasset", None);
        let stale = entry("content/a/stale.gasset", Some("assets/a.glb"));
        let loose = entry("content/a/mesh_0.gasset", Some("assets/a.glb"));
        let other = entry("content/b/scene.gasset", Some("assets/b.glb"));
        let authored = entry("content/level.gasset", None);
        let mut registry = AssetRegistry::new();
        for entry in [&kept, &dropped, &stale, &loose, &other, &authored] {
            registry.insert(entry.clone());
        }
        let embedded = AssetEntry {
            embedded_in: Some(EmbeddedIn {
                host: kept.id,
                name: "mesh/0".into(),
            }),
            address: kept.address.clone(),
            ..loose.clone()
        };
        let imported = ImportedSource {
            assets: vec![kept.clone(), embedded.clone()],
            siblings: Vec::new(),
            removed: vec![dropped.id],
        };
        apply_to_registry(&mut registry, &imported);
        let mut left: Vec<String> = registry.iter().map(AssetEntry::location).collect();
        left.sort();
        assert_eq!(
            left,
            [
                "content/a/scene.gasset",
                "content/a/scene.gasset#mesh/0",
                "content/b/scene.gasset",
                "content/level.gasset",
            ]
        );
        assert_eq!(registry.get(loose.id), Some(&embedded));
    }
}
