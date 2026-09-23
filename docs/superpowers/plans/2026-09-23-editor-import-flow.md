# Editor Asset Import Flow Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a user import source files (glTF, OBJ, images) into an open project from inside the editor, via an Import button, a native file picker, and a modal dialog that confirms where each file lands.

**Architecture:** An `Import…` button in the Curiosities panel opens an `rfd` multi-select picker on a worker thread. The picked paths become editable staging rows in a modal dialog; confirming pushes them onto a strictly serial FIFO queue whose worker copies each source into the project and calls `import::import_source`. Completion re-runs `discover_project` and republishes the registry to the `AssetServer` without bumping `ProjectState::generation` or closing open documents.

**Tech Stack:** Rust, the workspace's own `ecs`/`app`/`ui` crates, `rfd` for the native file dialog, `anyhow`, `tempfile` (tests).

**Spec:** `docs/superpowers/specs/2026-09-23-editor-import-flow-design.md`

## Global Constraints

- The `import` crate gains exactly one new public function, `supported_extension`. Nothing else in `crates/import/` or `crates/asset-import/` changes.
- Imports run strictly one at a time. `import_source` rewrites `.registry.toml` wholesale and documents itself as not a concurrent-writer API.
- The import path must never touch `ProjectState::job`. That field backs `ProjectState::busy()`, which disables the Curiosities panel.
- The catalogue refresh after an import must not bump `ProjectState::generation` and must not push `AssetEditorCommand::CloseAll`.
- Content root and extension always come from `ContentConfig::load_or_default(project_root)`, never hardcoded.
- Destination paths are project-relative strings using `/` separators, never absolute and never containing `..`.
- Comments: only one-line `///` doc comments on public items. No narrative comments explaining what code does.
- New editor dependencies: `import = { path = "../import" }` and `rfd = "0.15"`.

## Review Focus

These are the conditions the spec implies but does not spell out. Each has a test in the task that owns the code.

1. **Picker cancelled** — `pick_files()` returns `None`; no dialog opens, the status line says so, and the button becomes usable again. (Task 7)
2. **Source deleted between picking and confirming** — the worker errors on that row, the queue continues with the remaining rows rather than abandoning the batch. (Task 5)
3. **Destination at the project root with no parent directory** — e.g. `hero.glb`; the copy must still succeed rather than failing on a missing parent. (Task 2)
4. **Empty or directory-shaped destination** — `""` or `assets/` has no file name; the row must be rejected rather than producing a path with no stem. (Task 3)
5. **Confirm pressed with zero importable rows** — every row rejected; Confirm must be inert and queue nothing. (Task 6)

---

### Task 1: Expose supported extensions from the import crate

The validator needs to know which extensions have an importer, but `registered_importers` is private.

**Files:**
- Modify: `crates/import/src/lib.rs:21-27` (add the function below `registered_importers`)
- Test: `crates/import/src/lib.rs` (new `#[cfg(test)] mod tests` at the end of the file)

**Interfaces:**
- Consumes: nothing.
- Produces: `pub fn import::supported_extension(extension: &str) -> bool` — takes an extension without a leading dot, case-insensitive.

- [ ] **Step 1: Write the failing test**

Append to `crates/import/src/lib.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_extensions_are_matched_without_regard_to_case() {
        assert!(supported_extension("obj"));
        assert!(supported_extension("GLB"), "matching is case-insensitive");
        assert!(supported_extension("gltf"));
        assert!(supported_extension("png"));
        assert!(!supported_extension("txt"));
        assert!(
            !supported_extension(".obj"),
            "the caller passes an extension, not a suffix with its dot"
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p import supported_extensions_are_matched -- --nocapture`
Expected: FAIL to compile with "cannot find function `supported_extension`".

- [ ] **Step 3: Write minimal implementation**

Add below `registered_importers` in `crates/import/src/lib.rs`:

```rust
/// Whether any registered importer handles this extension, given without a dot.
pub fn supported_extension(extension: &str) -> bool {
    let extension = extension.to_ascii_lowercase();
    registered_importers()
        .iter()
        .any(|importer| importer.supported_extensions().contains(&extension.as_str()))
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p import supported_extensions_are_matched`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/import/src/lib.rs
git commit -m "Expose which extensions the registered importers handle"
```

---

### Task 2: The import worker function

The one piece of work a queued row performs: copy the source to its destination inside the project, then import it.

**Files:**
- Create: `crates/editor/src/import/mod.rs`
- Modify: `crates/editor/src/lib.rs:5-21` (add `pub mod import;` in alphabetical order, after `pub mod hierarchy;`)
- Modify: `crates/editor/src/lib.rs:1-4` (module doc comment currently reads "Importing is the `import` CLI's job.")
- Modify: `crates/editor/Cargo.toml` (add the `import` dependency)

**Interfaces:**
- Consumes: `import::supported_extension` (Task 1), `import::import_source`, `import::config::ContentConfig`, `import::ImportedAsset`.
- Produces: `pub fn editor::import::import_source_into(source: &Path, destination: &str, project_root: &Path) -> anyhow::Result<Vec<ImportedAsset>>`.

- [ ] **Step 1: Add the dependency**

In `crates/editor/Cargo.toml`, under `[dependencies]`, after the `essential` line:

```toml
import = { path = "../import" }
```

- [ ] **Step 2: Write the failing tests**

Create `crates/editor/src/import/mod.rs`:

```rust
//! Importing source files into the open project.

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../obj-loader/tests/fixtures/square.obj")
            .canonicalize()
            .expect("the obj fixture exists")
    }

    #[test]
    fn an_external_source_is_copied_to_its_destination_and_imported() {
        let project = tempfile::tempdir().expect("tempdir");
        let written = import_source_into(&fixture(), "assets/square.obj", project.path())
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
        let project = tempfile::tempdir().expect("tempdir");
        import_source_into(&fixture(), "square.obj", project.path())
            .expect("a destination with no parent directory still imports");
        assert!(project.path().join("square.obj").is_file());
    }

    #[test]
    fn a_source_already_at_its_destination_is_left_in_place() {
        let project = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(project.path().join("assets")).expect("assets");
        let inside = project.path().join("assets/square.obj");
        std::fs::copy(fixture(), &inside).expect("seed the source");
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
        let project = tempfile::tempdir().expect("tempdir");
        let first = import_source_into(&fixture(), "assets/square.obj", project.path())
            .expect("first import");
        let second = import_source_into(&fixture(), "assets/square.obj", project.path())
            .expect("second import");
        let ids = |assets: &[import::ImportedAsset]| {
            assets.iter().map(|a| a.asset_id).collect::<Vec<_>>()
        };
        assert_eq!(
            ids(&first),
            ids(&second),
            "re-importing reuses the sidecar, so asset identities are stable"
        );
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p editor --lib import::`
Expected: FAIL to compile with "cannot find function `import_source_into`".

- [ ] **Step 4: Write the implementation**

Put this above the `#[cfg(test)]` module in `crates/editor/src/import/mod.rs`:

```rust
use std::path::Path;

use anyhow::{bail, Context};
use import::config::ContentConfig;
use import::ImportedAsset;

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
    if !import::supported_extension(extension) {
        bail!("no importer handles '.{extension}'");
    }
    let config = ContentConfig::load_or_default(project_root)?;
    let target = project_root.join(destination);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating '{}'", parent.display()))?;
    }
    let same = source
        .canonicalize()
        .ok()
        .zip(target.canonicalize().ok())
        .is_some_and(|(source, target)| source == target);
    if !same {
        std::fs::copy(source, &target).with_context(|| {
            format!("copying '{}' to '{}'", source.display(), target.display())
        })?;
    }
    import::import_source(&target, project_root, &config)
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p editor --lib import::`
Expected: PASS, all five tests.

- [ ] **Step 6: Register the module and correct the stale doc comment**

In `crates/editor/src/lib.rs`, add after `pub mod hierarchy;`:

```rust
pub mod import;
```

Then replace the fourth line of the module doc comment:

```rust
//! Asset editors allow temporary inspection and edits, but never save files.
//! Importing brings source files into the project's content tree.
```

- [ ] **Step 7: Run the editor test suite**

Run: `cargo test -p editor`
Expected: PASS

- [ ] **Step 8: Commit**

```bash
git add crates/editor/Cargo.toml crates/editor/src/lib.rs crates/editor/src/import/mod.rs Cargo.lock
git commit -m "Add the editor's import worker function"
```

---

### Task 3: Staging rows and their validation

Turning picked paths into editable rows, and deciding which rows may be imported.

**Files:**
- Modify: `crates/editor/src/import/mod.rs` (add the types and functions; tests go in the existing `mod tests`)

**Interfaces:**
- Consumes: `import::supported_extension` (Task 1).
- Produces:
  - `pub struct StagedImport { pub source: PathBuf, pub destination: String, pub state: RowState }`
  - `pub enum RowState { New, Replaces, Rejected(String) }`
  - `pub fn stage_sources(sources: Vec<PathBuf>) -> Vec<StagedImport>`
  - `pub fn validate_rows(rows: &mut [StagedImport], project_root: &Path)`
  - `impl StagedImport { pub fn importable(&self) -> bool }`

- [ ] **Step 1: Write the failing tests**

Add to the `mod tests` block in `crates/editor/src/import/mod.rs`:

```rust
fn rows(pairs: &[(&str, &str)]) -> Vec<StagedImport> {
    pairs
        .iter()
        .map(|(source, destination)| StagedImport {
            source: std::path::PathBuf::from(source),
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
    let staged = stage_sources(vec![std::path::PathBuf::from("/home/someone/hero.glb")]);
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p editor --lib import::`
Expected: FAIL to compile with "cannot find type `StagedImport`".

- [ ] **Step 3: Write the implementation**

Add to `crates/editor/src/import/mod.rs`, above the tests:

```rust
use std::collections::HashMap;
use std::path::{Component, PathBuf};

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
    let mut counts: HashMap<String, usize> = HashMap::new();
    for row in rows.iter() {
        *counts.entry(row.destination.clone()).or_default() += 1;
    }
    for row in rows.iter_mut() {
        row.state = row_state(row, project_root, counts[&row.destination]);
    }
}

fn row_state(row: &StagedImport, project_root: &Path, occurrences: usize) -> RowState {
    if occurrences > 1 {
        return RowState::Rejected("two files claim this destination twice".into());
    }
    let destination = Path::new(&row.destination);
    if destination.as_os_str().is_empty()
        || destination.is_absolute()
        || destination
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return RowState::Rejected("must stay inside the project, without '..'".into());
    }
    let Some(name) = destination.file_name().and_then(|value| value.to_str()) else {
        return RowState::Rejected("needs a file name".into());
    };
    if row.destination.ends_with('/') || name.is_empty() {
        return RowState::Rejected("needs a file name".into());
    }
    let source_extension = extension_of(&row.source);
    if !import::supported_extension(&source_extension) {
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p editor --lib import::`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/editor/src/import/mod.rs
git commit -m "Stage picked sources into validated import rows"
```

---

### Task 4: Refresh the catalogue without closing open documents

`EditorCommand::OpenProject` bumps `generation` and pushes `CloseAll`. An import must do neither.

**Files:**
- Modify: `crates/editor/src/project.rs:141-146` (add the command variant)
- Modify: `crates/editor/src/project.rs:172-247` (add the refresh handling)
- Test: `crates/editor/tests/catalogue_refresh.rs` (create)

**Interfaces:**
- Consumes: `discover_project`, `ProjectState`, `AssetServer::publish_project_content`.
- Produces: `EditorCommand::RefreshCatalogue`, handled synchronously in `process_commands`.

- [ ] **Step 1: Write the failing test**

Create `crates/editor/tests/catalogue_refresh.rs`:

```rust
//! Refreshing after an import must not disturb open documents.
use app::{schedule_groups::Update, App};
use editor::asset_editor::{AssetEditorCommand, AssetEditorCommands};
use editor::project::{discover_project, EditorCommand, EditorCommands, ProjectPlugin, ProjectState};
use essential::assets::asset_server::AssetServer;
use essential::assets::content::{write_content_asset, ContentAssetHeader, ImportProvenance, CONTENT_FORMAT_VERSION};
use essential::assets::AssetId;

fn asset(root: &std::path::Path, name: &str) {
    let header = ContentAssetHeader {
        format_version: CONTENT_FORMAT_VERSION,
        asset_id: AssetId::new(),
        references: vec![],
        kind: "Mesh".into(),
        provenance: Some(ImportProvenance {
            source: "assets/model.obj".into(),
            sub_asset: name.into(),
        }),
    };
    std::fs::create_dir_all(root.join("content")).expect("content");
    std::fs::write(
        root.join(format!("content/{name}.gasset")),
        write_content_asset(&header, &[]).expect("encode"),
    )
    .expect("write");
}

#[test]
fn refreshing_adds_assets_without_bumping_generation_or_closing_documents() {
    let project = tempfile::tempdir().expect("tempdir");
    asset(project.path(), "first");

    let mut app = App::new();
    app.insert_resource(EditorCommands::default());
    app.insert_resource(AssetEditorCommands::default());
    app.insert_resource(AssetServer::new());
    let mut state = ProjectState::default();
    state.project = Some(discover_project(project.path()).expect("discover"));
    state.generation = 7;
    app.insert_resource(state);
    app.register_plugin(ProjectPlugin);
    app.finish_plugin_build();

    asset(project.path(), "second");
    app.get_resource_mut::<EditorCommands>()
        .expect("commands")
        .0
        .push_back(EditorCommand::RefreshCatalogue);
    app.update();

    let state = app.get_resource::<ProjectState>().expect("state");
    assert_eq!(
        state.project.as_ref().expect("project").assets.len(),
        2,
        "the refresh picks up the newly written asset"
    );
    assert_eq!(state.generation, 7, "a refresh is the same project, so generation is untouched");
    let documents = app.get_resource::<AssetEditorCommands>().expect("documents");
    assert!(
        !documents.0.iter().any(|c| matches!(c, AssetEditorCommand::CloseAll)),
        "a refresh must not close open documents"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p editor --test catalogue_refresh`
Expected: FAIL to compile with "no variant `RefreshCatalogue`".

- [ ] **Step 3: Add the command variant**

In `crates/editor/src/project.rs`, extend the enum:

```rust
pub enum EditorCommand {
    OpenProject(PathBuf),
    OpenAsset(AssetId),
    RefreshCatalogue,
}
```

- [ ] **Step 4: Handle it in `process_commands`**

Inside the `match command` block in `process_commands`, add an arm alongside `OpenAsset` and `OpenProject`:

```rust
EditorCommand::RefreshCatalogue => {
    let Some(root) = state.project.as_ref().map(|project| project.root.clone()) else {
        continue;
    };
    match discover_project(&root) {
        Ok(project) => {
            if let Err(error) =
                asset_server.publish_project_content(&project.root, project.registry.clone())
            {
                state.status = format!("Could not refresh project assets: {error:#}");
                continue;
            }
            state.project = Some(project);
        }
        Err(error) => state.status = format!("Could not refresh catalogue: {error:#}"),
    }
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p editor --test catalogue_refresh`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add crates/editor/src/project.rs crates/editor/tests/catalogue_refresh.rs
git commit -m "Refresh the catalogue without closing open documents"
```

---

### Task 5: The import queue and its worker systems

**Files:**
- Modify: `crates/editor/src/import/mod.rs` (add the resources, systems and plugin)
- Test: `crates/editor/tests/import_queue.rs` (create)

**Interfaces:**
- Consumes: `import_source_into` (Task 2), `StagedImport`/`RowState` (Task 3), `EditorCommand::RefreshCatalogue` (Task 4).
- Produces:
  - `#[derive(Resource, Default)] pub struct ImportQueue` with `pub fn enqueue(&mut self, rows: Vec<StagedImport>)`, `pub fn is_running(&self) -> bool`, `pub fn remaining(&self) -> usize`
  - `pub struct ImportPlugin`
  - systems `drive_imports`, `finish_import`

- [ ] **Step 1: Write the failing test**

Create `crates/editor/tests/import_queue.rs`:

```rust
//! The queue runs imports one at a time and survives a failing row.
use app::App;
use editor::asset_editor::AssetEditorCommands;
use editor::import::{stage_sources, ImportPlugin, ImportQueue};
use editor::project::{discover_project, EditorCommands, ProjectPlugin, ProjectState};
use essential::assets::asset_server::AssetServer;

fn fixture() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../obj-loader/tests/fixtures/square.obj")
        .canonicalize()
        .expect("the obj fixture exists")
}

fn editor(root: &std::path::Path) -> App {
    std::fs::create_dir_all(root.join("content")).expect("content");
    let mut app = App::new();
    app.insert_resource(EditorCommands::default());
    app.insert_resource(AssetEditorCommands::default());
    app.insert_resource(AssetServer::new());
    let mut state = ProjectState::default();
    state.project = Some(discover_project(root).expect("discover"));
    app.insert_resource(state);
    app.register_plugin(ProjectPlugin);
    app.register_plugin(ImportPlugin);
    app.finish_plugin_build();
    app
}

fn drain(app: &mut App) {
    for _ in 0..200 {
        app.update();
        if app.get_resource::<ImportQueue>().expect("queue").remaining() == 0
            && !app.get_resource::<ImportQueue>().expect("queue").is_running()
        {
            app.update();
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the queue never drained");
}

#[test]
fn a_batch_imports_every_row_and_shows_up_in_the_catalogue() {
    let project = tempfile::tempdir().expect("tempdir");
    let mut app = editor(project.path());
    let mut rows = stage_sources(vec![fixture(), fixture()]);
    rows[1].destination = "assets/other.obj".into();
    app.get_resource_mut::<ImportQueue>()
        .expect("queue")
        .enqueue(rows);
    drain(&mut app);

    assert!(project.path().join("assets/square.obj").is_file());
    assert!(project.path().join("assets/other.obj").is_file());
    let state = app.get_resource::<ProjectState>().expect("state");
    assert!(
        state.project.as_ref().expect("project").assets.len() >= 2,
        "both imports reached the catalogue"
    );
}

#[test]
fn a_failing_row_does_not_abandon_the_rest_of_the_batch() {
    let project = tempfile::tempdir().expect("tempdir");
    let mut app = editor(project.path());
    let mut rows = stage_sources(vec![
        std::path::PathBuf::from("/nonexistent/missing.obj"),
        fixture(),
    ]);
    rows[1].destination = "assets/square.obj".into();
    app.get_resource_mut::<ImportQueue>()
        .expect("queue")
        .enqueue(rows);
    drain(&mut app);

    assert!(
        project.path().join("assets/square.obj").is_file(),
        "the row after the failing one still imported"
    );
    let state = app.get_resource::<ProjectState>().expect("state");
    assert!(
        state.status.contains("missing.obj"),
        "the failure is reported, got: {}",
        state.status
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p editor --test import_queue`
Expected: FAIL to compile with "cannot find type `ImportQueue`".

- [ ] **Step 3: Write the implementation**

Add to `crates/editor/src/import/mod.rs`:

```rust
use std::collections::VecDeque;
use std::thread::JoinHandle;

use app::{schedule_groups::Update, App, Plugin};
use ecs::{Res, ResMut, Resource};

use crate::project::{EditorCommand, EditorCommands, ProjectState};

/// Queued imports and the single worker running one of them.
#[derive(Resource, Default)]
pub struct ImportQueue {
    pending: VecDeque<StagedImport>,
    job: Option<JoinHandle<Result<Vec<ImportedAsset>, String>>>,
    current: Option<String>,
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
        }
        self.total += rows.len();
        self.pending.extend(rows);
    }

    pub fn is_running(&self) -> bool {
        self.job.is_some()
    }

    pub fn remaining(&self) -> usize {
        self.pending.len()
    }
}

pub struct ImportPlugin;

impl Plugin for ImportPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ImportQueue::default());
        app.add_system(Update, finish_import);
        app.add_system(Update, drive_imports);
    }
}

fn drive_imports(
    mut queue: ResMut<ImportQueue>,
    mut state: ResMut<ProjectState>,
    project: Res<ProjectState>,
) {
    let _ = &project;
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
        import_source_into(&row.source, &row.destination, &root).map_err(|error| format!("{error:#}"))
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
        .unwrap_or_else(|_| Err("Import worker panicked".into()));
    let name = queue.current.take().unwrap_or_default();
    queue.done += 1;
    match result {
        Ok(written) => {
            state.status = format!("Imported {name} · {} assets", written.len());
            commands.0.push_back(EditorCommand::RefreshCatalogue);
        }
        Err(error) => state.status = format!("Could not import {name}: {error}"),
    }
}
```

Remove the unused `project: Res<ProjectState>` parameter and the `let _ = &project;` line from `drive_imports` — a system cannot take both `Res` and `ResMut` of the same resource. The signature is:

```rust
fn drive_imports(mut queue: ResMut<ImportQueue>, mut state: ResMut<ProjectState>) {
```

- [ ] **Step 4: Register the plugin**

In `crates/editor/src/lib.rs`, in `EditorPlugin::build`, after `app.register_plugin(content::ContentPlugin);`:

```rust
app.register_plugin(import::ImportPlugin);
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p editor --test import_queue`
Expected: PASS

- [ ] **Step 6: Run the whole suite**

Run: `cargo test -p editor`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add crates/editor/src/import/mod.rs crates/editor/src/lib.rs crates/editor/tests/import_queue.rs
git commit -m "Run queued imports one at a time and refresh on completion"
```

---

### Task 6: The modal staging dialog

**Files:**
- Create: `crates/editor/src/import/dialog.rs`
- Modify: `crates/editor/src/import/mod.rs` (add `ImportStaging`, declare `pub mod dialog;`, register the dialog's systems in `ImportPlugin`)
- Test: `crates/editor/tests/import_dialog.rs` (create)

**Interfaces:**
- Consumes: `StagedImport`, `RowState`, `validate_rows`, `ImportQueue::enqueue`, `ProjectState`, `PanelRegistry::root`.
- Produces:
  - `#[derive(Resource, Default)] pub struct ImportStaging { pub rows: Vec<StagedImport>, pub visible: bool }`
  - `pub fn editor::import::dialog::DialogPlugin` registered by `ImportPlugin`
  - Components `DialogRoot`, `DialogRow(usize)`, `DialogField(usize)`, `DialogStatus(usize)`, `DialogAction`

- [ ] **Step 1: Write the failing test**

Create `crates/editor/tests/import_dialog.rs`:

```rust
//! Confirming the staging dialog queues exactly the importable rows.
use app::App;
use editor::import::{stage_sources, ImportPlugin, ImportQueue, ImportStaging, RowState};
use editor::project::{discover_project, EditorCommands, ProjectPlugin, ProjectState};
use editor::asset_editor::AssetEditorCommands;
use essential::assets::asset_server::AssetServer;
use ui::interaction::{HoveredNode, UIClick};
use ui::theme::UITheme;

fn editor(root: &std::path::Path) -> App {
    std::fs::create_dir_all(root.join("content")).expect("content");
    let mut app = App::new();
    app.insert_resource(EditorCommands::default());
    app.insert_resource(AssetEditorCommands::default());
    app.insert_resource(AssetServer::new());
    app.insert_resource(UITheme::default());
    app.insert_resource(HoveredNode::default());
    app.register_event::<UIClick>();
    let mut state = ProjectState::default();
    state.project = Some(discover_project(root).expect("discover"));
    app.insert_resource(state);
    app.register_plugin(ProjectPlugin);
    app.register_plugin(ImportPlugin);
    app.finish_plugin_build();
    app
}

#[test]
fn confirming_queues_only_the_importable_rows() {
    let project = tempfile::tempdir().expect("tempdir");
    let mut app = editor(project.path());
    let mut rows = stage_sources(vec![
        std::path::PathBuf::from("/tmp/hero.obj"),
        std::path::PathBuf::from("/tmp/notes.txt"),
    ]);
    rows[1].state = RowState::Rejected("no importer handles '.txt'".into());
    {
        let staging = app.get_resource_mut::<ImportStaging>().expect("staging");
        staging.rows = rows;
        staging.visible = true;
    }
    editor::import::dialog::confirm(
        app.get_resource_mut::<ImportStaging>().expect("staging"),
        app.get_resource_mut::<ImportQueue>().expect("queue"),
    );
    let queue = app.get_resource::<ImportQueue>().expect("queue");
    assert_eq!(queue.remaining(), 1, "the rejected row is not queued");
    let staging = app.get_resource::<ImportStaging>().expect("staging");
    assert!(!staging.visible, "confirming closes the dialog");
    assert!(staging.rows.is_empty(), "confirming clears the staged rows");
}

#[test]
fn confirming_with_no_importable_rows_queues_nothing() {
    let project = tempfile::tempdir().expect("tempdir");
    let mut app = editor(project.path());
    let mut rows = stage_sources(vec![std::path::PathBuf::from("/tmp/notes.txt")]);
    rows[0].state = RowState::Rejected("no importer handles '.txt'".into());
    {
        let staging = app.get_resource_mut::<ImportStaging>().expect("staging");
        staging.rows = rows;
        staging.visible = true;
    }
    editor::import::dialog::confirm(
        app.get_resource_mut::<ImportStaging>().expect("staging"),
        app.get_resource_mut::<ImportQueue>().expect("queue"),
    );
    let queue = app.get_resource::<ImportQueue>().expect("queue");
    assert_eq!(queue.remaining(), 0);
    assert!(!queue.is_running());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p editor --test import_dialog`
Expected: FAIL to compile with "cannot find type `ImportStaging`".

- [ ] **Step 3: Add the staging resource**

In `crates/editor/src/import/mod.rs`, add `pub mod dialog;` at the top of the file and:

```rust
/// Rows awaiting the user's confirmation, and whether the dialog is shown.
#[derive(Resource, Default)]
pub struct ImportStaging {
    pub rows: Vec<StagedImport>,
    pub visible: bool,
}
```

In `ImportPlugin::build`, before the systems:

```rust
app.insert_resource(ImportStaging::default());
app.register_plugin(dialog::DialogPlugin);
```

- [ ] **Step 4: Write the dialog module**

Create `crates/editor/src/import/dialog.rs`. Mirror the structure of `crates/editor/src/content.rs`: build the nodes once at `Startup`, then render them from the resource each frame.

```rust
//! The modal that confirms where each picked file lands.
use app::{
    schedule_groups::{LateUpdate, Startup},
    App, Plugin,
};
use ecs::{
    command::CommandQueue, events::event_reader::EventReader, Component, Query, Res, ResMut,
};
use taffy::FlexDirection;
use ui::{
    focus::UIFocusable,
    interaction::{Interactable, UIClick, UIInteractionStyle},
    material::UIMaterial,
    node::{UIInset, UINode, UIRect},
    text::TextComponent,
    text_input::{UITextInput, UITextInputChanged},
    theme::UITheme,
    transform::UIValue,
};

use crate::dock::PanelRegistry;
use crate::import::{validate_rows, ImportQueue, ImportStaging, RowState};
use crate::marks::TRANSPARENT;
use crate::project::ProjectState;

/// Above every editor layer, including the window chrome's resize grips.
const SCRIM_LAYER: i32 = 200;
const ROW_HEIGHT: f32 = 30.0;
/// Rows rendered at once; a longer selection scrolls.
const ROWS: usize = 10;

#[derive(Component)]
struct DialogRoot;

#[derive(Component)]
struct DialogRow(usize);

#[derive(Component)]
struct DialogField(usize);

#[derive(Component)]
struct DialogStatus(usize);

#[derive(Component, Clone, Copy)]
enum DialogAction {
    Confirm,
    Cancel,
}

pub struct DialogPlugin;

impl Plugin for DialogPlugin {
    fn build(&self, app: &mut App) {
        app.add_system(Startup, build_dialog);
        app.add_system(LateUpdate, edit_destinations);
        app.add_system(LateUpdate, handle_buttons);
        app.add_system(LateUpdate, render_dialog);
    }
}
```

`build_dialog` spawns, as a child of `PanelRegistry::root()`:
- a scrim `UINode` with `position: taffy::Position::Absolute`, `inset: UIInset` of `UIValue::Px(0.0)` on all four sides, `z_index: SCRIM_LAYER`, `visible: false`, plus `Interactable` and `DialogRoot`, with a `UIMaterial::flat` in a dimmed `theme.canvas`;
- inside it a centred card using `UIMaterial { corner_radius: theme.radius_md, ..UIMaterial::with_border(theme.surface, theme.border, 1.0) }`, `z_index: SCRIM_LAYER + 1`;
- a title `TextComponent` reading `Import assets`;
- a `UIScrollArea` + `UIVirtualList::new(0, ROW_HEIGHT)` viewport holding a pool of `ROWS` rows, each with a `DialogRow(slot)` marker, a source-name label, a `UITextInput::new("assets/…")` carrying `Interactable`, `UIFocusable` and `DialogField(slot)`, and a `DialogStatus(slot)` message label;
- a footer row with two `Interactable` buttons carrying `DialogAction::Cancel` and `DialogAction::Confirm`.

`edit_destinations` reads `UITextInputChanged`, matches the event entity against `DialogField`, writes the new value into `staging.rows[index].destination`, then calls `validate_rows` against the open project's root.

`render_dialog` sets the scrim's `visible` from `staging.visible`, fills each pooled row's labels from `staging.rows`, hides unused rows, and colours each status message: `theme.text_muted` for `RowState::New`, `theme.warning` with the text `replaces existing, keeps asset IDs` for `RowState::Replaces`, and `theme.error` with the reason for `RowState::Rejected`.

`handle_buttons` reads `UIClick`, matches `DialogAction`, and calls the two functions below.

```rust
/// Queues every importable row and closes the dialog.
pub fn confirm(mut staging: ResMut<ImportStaging>, mut queue: ResMut<ImportQueue>) {
    let rows = std::mem::take(&mut staging.rows);
    queue.enqueue(rows);
    staging.visible = false;
}

/// Discards the staged rows and closes the dialog.
pub fn cancel(mut staging: ResMut<ImportStaging>) {
    staging.rows.clear();
    staging.visible = false;
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p editor --test import_dialog`
Expected: PASS

- [ ] **Step 6: Run the whole suite**

Run: `cargo test -p editor`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add crates/editor/src/import/dialog.rs crates/editor/src/import/mod.rs crates/editor/tests/import_dialog.rs
git commit -m "Confirm import destinations in a modal staging dialog"
```

---

### Task 7: The Import button and the native file picker

**Files:**
- Modify: `crates/editor/Cargo.toml` (add `rfd`)
- Modify: `crates/editor/src/content.rs:63-67` (extend `Action`), `:130-150` (spawn the button), `:363-400` (`handle_actions`)
- Modify: `crates/editor/src/import/mod.rs` (add the picker resource and system)
- Test: `crates/editor/tests/import_picker.rs` (create)

**Interfaces:**
- Consumes: `stage_sources`, `validate_rows`, `ImportStaging`, `ProjectState`.
- Produces:
  - `#[derive(Resource, Default)] pub struct ImportPicker` with `pub fn request(&mut self)` and `pub fn is_picking(&self) -> bool`
  - `pub fn editor::import::accept_picked(sources: Option<Vec<PathBuf>>, staging: &mut ImportStaging, state: &mut ProjectState)`
  - `Action::Import` in `content.rs`

- [ ] **Step 1: Add the dependency**

In `crates/editor/Cargo.toml`, under `[dependencies]`:

```toml
rfd = "0.15"
```

- [ ] **Step 2: Write the failing test**

Create `crates/editor/tests/import_picker.rs`:

```rust
//! What the editor does with the picker's result, without opening a picker.
use editor::import::{accept_picked, ImportStaging};
use editor::project::{discover_project, ProjectState};

fn state(root: &std::path::Path) -> ProjectState {
    std::fs::create_dir_all(root.join("content")).expect("content");
    let mut state = ProjectState::default();
    state.project = Some(discover_project(root).expect("discover"));
    state
}

#[test]
fn a_cancelled_pick_opens_no_dialog_and_says_so() {
    let project = tempfile::tempdir().expect("tempdir");
    let mut state = state(project.path());
    let mut staging = ImportStaging::default();
    accept_picked(None, &mut staging, &mut state);
    assert!(!staging.visible, "a cancelled pick opens no dialog");
    assert!(staging.rows.is_empty());
    assert!(
        state.status.to_lowercase().contains("cancel"),
        "the cancellation is reported, got: {}",
        state.status
    );
}

#[test]
fn picked_files_open_the_dialog_already_validated() {
    let project = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(project.path().join("assets")).expect("assets");
    std::fs::write(project.path().join("assets/hero.obj"), b"old").expect("write");
    let mut state = state(project.path());
    let mut staging = ImportStaging::default();
    accept_picked(
        Some(vec![
            std::path::PathBuf::from("/elsewhere/hero.obj"),
            std::path::PathBuf::from("/elsewhere/notes.txt"),
        ]),
        &mut staging,
        &mut state,
    );
    assert!(staging.visible);
    assert_eq!(staging.rows[0].destination, "assets/hero.obj");
    assert!(
        matches!(staging.rows[0].state, editor::import::RowState::Replaces),
        "validation already ran, so the occupied destination is marked"
    );
    assert!(
        matches!(staging.rows[1].state, editor::import::RowState::Rejected(_)),
        "the unsupported file is rejected rather than silently discarded"
    );
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p editor --test import_picker`
Expected: FAIL to compile with "cannot find function `accept_picked`".

- [ ] **Step 4: Write the picker**

Add to `crates/editor/src/import/mod.rs`:

```rust
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
    validate_rows(&mut rows, &root);
    staging.rows = rows;
    staging.visible = !staging.rows.is_empty();
}

fn drive_picker(
    mut picker: ResMut<ImportPicker>,
    mut staging: ResMut<ImportStaging>,
    mut state: ResMut<ProjectState>,
) {
    if picker.job.as_ref().is_some_and(|job| job.is_finished()) {
        let picked = picker
            .job
            .take()
            .expect("the job was just observed as finished")
            .join()
            .unwrap_or(None);
        accept_picked(picked, &mut staging, &mut state);
    }
    if !std::mem::take(&mut picker.requested)
        || picker.job.is_some()
        || staging.visible
        || state.project.is_none()
    {
        return;
    }
    let extensions = SUPPORTED_EXTENSIONS;
    picker.job = Some(std::thread::spawn(move || {
        rfd::FileDialog::new()
            .add_filter("Assets", extensions)
            .pick_files()
    }));
}
```

Define the filter list next to it:

```rust
/// Extensions offered by the picker's filter.
const SUPPORTED_EXTENSIONS: &[&str] = &["gltf", "glb", "obj", "png", "jpg", "jpeg"];
```

In `ImportPlugin::build`, add `app.insert_resource(ImportPicker::default());` and `app.add_system(Update, drive_picker);`.

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p editor --test import_picker`
Expected: PASS

- [ ] **Step 6: Add the button to the Curiosities panel**

In `crates/editor/src/content.rs`, extend the `Action` enum:

```rust
#[derive(Component, Clone, Copy)]
enum Action {
    Asset(usize),
    /// One of the kind tags above the list.
    Kind(usize),
    Import,
}
```

In `build_panel`, after the `tags` loop and before the `view` node, spawn an import button as a child of `tags`, so it sits on the tag row:

```rust
let import = cmd
    .spawn((
        UINode {
            flex_shrink: 0.0,
            margin: UIRect {
                left: UIValue::Auto,
                ..Default::default()
            },
            padding: UIRect::axes(2.0, 7.0),
            ..Default::default()
        },
        UIMaterial {
            corner_radius: theme.radius_sm,
            ..UIMaterial::with_border(TRANSPARENT, theme.accent, 1.0)
        },
        TextComponent {
            color: theme.text,
            font_size: 10.0,
            line_height: theme.line_height(10.0),
            wrap: false,
            ..text(&theme, "Import…")
        },
        Interactable,
        UIInteractionStyle {
            normal: TRANSPARENT,
            hovered: theme.surface_hovered,
            pressed: selection_tint(&theme),
            disabled: TRANSPARENT,
        },
        Action::Import,
    ))
    .entity();
cmd.add_child(tags, import);
```

In `handle_actions`, add the arm and the picker parameter:

```rust
Action::Import => picker.request(),
```

with `mut picker: ResMut<crate::import::ImportPicker>` added to the system's parameters.

- [ ] **Step 7: Run the whole suite**

Run: `cargo test -p editor`
Expected: PASS

- [ ] **Step 8: Check the workspace still builds**

Run: `cargo build --workspace && cargo clippy -p editor -- -D warnings`
Expected: no errors, no warnings.

- [ ] **Step 9: Commit**

```bash
git add crates/editor/Cargo.toml crates/editor/src/content.rs crates/editor/src/import/mod.rs crates/editor/tests/import_picker.rs Cargo.lock
git commit -m "Open a native file picker from an Import button"
```

---

### Task 8: Verify the flow in the running editor

**Files:** none changed unless a defect turns up.

- [ ] **Step 1: Launch the editor against a scratch project**

```bash
mkdir -p /tmp/import-check/content
cargo run -p editor -- /tmp/import-check
```

- [ ] **Step 2: Walk the flow**

Click **Import…** in the Curiosities panel. Pick `crates/obj-loader/tests/fixtures/square.obj`. Confirm that:
- the dialog opens with `assets/square.obj` prefilled;
- clicking outside the card does not reach the panels beneath it;
- editing the path to `assets/props/square.obj` keeps the row importable;
- Confirm closes the dialog, the status line reports the import, and the new assets appear in the catalogue;
- `/tmp/import-check/assets/props/square.obj` and `/tmp/import-check/content/props/square/` both exist.

- [ ] **Step 3: Walk the re-import case**

Import the same fixture again to `assets/props/square.obj`. Confirm the row reads as a replacement, and that `.registry.toml` still lists the same UUIDs afterwards:

```bash
cat /tmp/import-check/.registry.toml
```

- [ ] **Step 4: Report**

Record what was observed. If anything diverges from the spec, fix it with a test first.
