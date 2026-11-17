//! Importing source files into the open project.

pub mod dialog;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Component, Path, PathBuf};
use std::thread::JoinHandle;

use concerto_app::{App, Plugin, schedule_groups::Update};
use concerto_ecs::{Event, IntoSystemConfig, ResMut, Resource, events::event_writer::EventWriter};
use concerto_foundation::assets::{
    AssetId,
    registry::{AssetEntry, AssetRegistry},
};
use concerto_import::ImportedSource;
use concerto_import::config::ContentConfig;
use concerto_render::assets::texture::{TextureSettings, texture_settings_options};

use crate::project::{AssetsWritten, ProjectState};

/// Imports a source into the project at `destination`, bringing the files it refers to along; the textures it creates take `texture_settings`, or the defaults.
pub fn import_source_into(
    source: &Path,
    destination: &str,
    project_root: &Path,
    texture_settings: Option<&TextureSettings>,
) -> anyhow::Result<ImportedSource> {
    let config = ContentConfig::load_or_default(project_root)?;
    concerto_import::import_source_into(
        source,
        destination,
        project_root,
        &config,
        texture_settings
            .map(texture_settings_options)
            .unwrap_or_default(),
    )
}

/// Whether importing `source` can create textures: a glTF or an image.
pub fn emits_textures(source: &Path) -> bool {
    matches!(
        extension_of(source).as_str(),
        "gltf" | "glb" | "png" | "jpg" | "jpeg"
    )
}

/// A file chosen for import, and where it will be copied to.
#[derive(Debug, Clone)]
pub struct StagedImport {
    pub source: PathBuf,
    /// Project-relative, e.g. `assets/hero.glb`.
    pub destination: String,
    pub state: RowState,
    /// Settings for the textures this import creates; `None` gives them the defaults.
    pub texture_settings: Option<TextureSettings>,
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
                texture_settings: None,
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

/// A queued import ended: what it wrote to `destination`, or why it failed.
#[derive(Event, Clone, Debug, PartialEq)]
pub struct ImportFinished {
    pub destination: String,
    pub result: Result<ImportedSource, String>,
}

/// Queued imports and the single worker running one of them.
#[derive(Resource, Default)]
pub struct ImportQueue {
    pending: VecDeque<QueuedImport>,
    job: Option<JoinHandle<(PathBuf, Result<ImportedSource, String>)>>,
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

const SUPPORTED_EXTENSIONS: &[&str] = &[
    "gltf", "glb", "obj", "bvh", "png", "jpg", "jpeg", "ogg", "wav",
];

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

/// Runs queued imports one at a time and adds each one's assets to the open project's registry.
pub struct ImportPlugin;

impl Plugin for ImportPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ImportQueue::default());
        app.insert_resource(ImportStaging::default());
        app.insert_resource(ImportPicker::default());
        app.register_event::<ImportFinished>();
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
    queue.current_destination = Some(row.destination.clone());
    queue.job = Some(std::thread::spawn(move || {
        let result = import_source_into(
            &row.source,
            &row.destination,
            &root,
            row.texture_settings.as_ref(),
        )
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
        (root, result)
    }));
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
    let (root, result) = match queue
        .job
        .take()
        .expect("the job was just observed as finished")
        .join()
    {
        Ok((root, result)) => (Some(root), result),
        Err(_) => {
            log::error!("the import worker panicked while importing '{name}'");
            (None, Err("the import worker panicked".into()))
        }
    };
    queue.done += 1;
    finished.write(ImportFinished {
        destination,
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
    fn the_picker_filter_lists_exactly_the_common_extensions_an_importer_handles() {
        let candidates = [
            "png", "jpg", "jpeg", "gltf", "glb", "obj", "bmp", "tga", "gif", "webp", "tiff", "tif",
            "hdr", "exr", "dds", "ktx2", "fbx", "dae", "ply", "stl", "usd", "usdz", "blend", "3ds",
            "mtl", "bvh", "ogg", "wav",
        ];
        for extension in candidates {
            assert_eq!(
                SUPPORTED_EXTENSIONS.contains(&extension),
                concerto_import::supported_extension(extension),
                "{extension}"
            );
        }
        for extension in SUPPORTED_EXTENSIONS {
            assert!(candidates.contains(extension), "{extension}");
        }
    }

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
            None,
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
                texture_settings: None,
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
