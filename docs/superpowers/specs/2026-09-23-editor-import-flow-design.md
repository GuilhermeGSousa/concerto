# Editor Asset Import Flow

## Intent

Bring source files (glTF, OBJ, images) into an open project from inside the
editor, without dropping to the `import` CLI. The user picks files, confirms
where they land, and the catalogue shows the resulting assets.

Success: a user with a project open clicks **Import…**, selects `hero.glb`,
accepts the proposed path, and sees the mesh, material and texture assets
appear in the Curiosities panel, loadable by the running editor.

## Flow

```
[Import…] button (Curiosities panel header)
        |
        v
  rfd file picker, multi-select, filtered to supported extensions
        |  Option<Vec<PathBuf>>
        v
  stage_sources() -> ImportStaging { rows, visible: true }
        |  row = { source, destination: String, state }
        |  destination prefilled "assets/<file_name>"
        v
  [modal staging dialog]  scrim + card, one editable row per file
        |  Confirm / Cancel, Enter / Esc
        v  confirm
  ImportQueue (FIFO) -> one worker thread at a time
        |
        v
  import_source_into() -> copy to destination -> import_source()
        |
        v
  refresh catalogue: discover_project -> publish_project_content -> status
```

## Decisions

**Sources are copied into the project.** The picker can select a file anywhere
on disk, but `import_source` writes an `.import.toml` sidecar next to the
source, and that sidecar owns the output asset UUIDs. A sidecar outside the
project is not committed, so re-importing after the file moves mints fresh
UUIDs and orphans every reference to the old ones. Copying first keeps the
sidecar inside the project, beside its source.

**The dialog edits the source path, not the output path.** `content_address`
already derives `content/props/hero/mesh_0.gasset` from a project-relative
source of `assets/props/hero.glb`, so steering the source steers the output.
The `import` crate needs no changes.

**Re-importing over an existing destination is the intended path.** Overwriting
`assets/hero.glb` reuses the existing sidecar, so every sub-asset keeps its
UUID and everything referencing those assets stays wired. This is the "I
changed the model in Blender, bring it back in" workflow. The dialog labels
such a row rather than blocking it.

**Imports run one at a time.** `import_source` rewrites `.registry.toml`
wholesale and documents itself as not a concurrent-writer API, so the queue is
strictly serial.

**Import does not use `ProjectState.job`.** That field backs `ProjectState::busy()`,
which disables the whole Curiosities panel. Browsing stays live during an
import, so the queue owns its own worker handle.

## Components

### `crates/editor/src/import/mod.rs`

`ImportPlugin`, the queue, and the worker entry point.

```rust
/// A file chosen for import and where it will be copied to.
pub struct StagedImport {
    pub source: PathBuf,
    /// Project-relative, e.g. "assets/hero.glb".
    pub destination: String,
    pub state: RowState,
}

pub enum RowState {
    New,
    /// The destination exists; importing replaces it and keeps its asset IDs.
    Replaces,
    Rejected(String),
}

#[derive(Resource, Default)]
pub struct ImportStaging {
    pub rows: Vec<StagedImport>,
    pub visible: bool,
}

#[derive(Resource, Default)]
pub struct ImportQueue {
    pending: VecDeque<StagedImport>,
    job: Option<JoinHandle<Result<Vec<ImportedAsset>, String>>>,
    current: Option<String>,
    done: usize,
    total: usize,
}
```

Systems, all on `Update`:

- `open_picker` — on the button's `UIClick`, spawns a thread running
  `rfd::FileDialog::new().add_filter("Assets", &exts).pick_files()`. Polled via
  `JoinHandle::is_finished()`, mirroring how `process_commands` polls the
  project worker. Gated off when no project is open, when the dialog is already
  visible, or when a pick is in flight.
- `stage_picked` — turns the picker's result into `ImportStaging` rows and shows
  the dialog. A cancelled pick sets the status line and shows nothing.
- `drive_imports` — when no job is running and `pending` is non-empty, pops one
  row and spawns the worker.
- `finish_import` — joins the completed worker and triggers the catalogue
  refresh.

The worker body is a plain function with no ECS in it:

```rust
pub fn import_source_into(
    source: &Path,
    destination: &str,
    project_root: &Path,
) -> anyhow::Result<Vec<ImportedAsset>>
```

1. `ContentConfig::load_or_default(project_root)`.
2. Resolve `project_root.join(destination)`; skip the copy when it is already
   the source (an in-project file kept at its own path).
3. Copy via stage-to-temp then `persist`, matching the `stage`/`replace`
   discipline in `import/src/lib.rs`.
4. `import_source(&destination_path, project_root, &config)`.

### `crates/editor/src/import/dialog.rs`

Builds the modal once at `Startup`, hidden. Rendered from `ImportStaging` each
frame, the way `content.rs` renders rows from `ContentState`; the dialog owns no
state of its own.

- A full-bleed scrim: `Position::Absolute`, inset 0 on all sides, high
  `z_index`, `Interactable`. UI hit-testing picks the single topmost
  `Interactable` by `paint_order`, so the scrim blocks every click beneath it.
- A centred card holding a recycled row pool, sized like `content.rs`'s pool.
  Each row: the source's file name as a label, a `UITextInput` bound to that
  row's `destination`, and a state message.
- Confirm and Cancel buttons. `Enter` confirms, `Esc` cancels, both read from
  the `Input` resource.
- Confirm moves every non-`Rejected` row onto `ImportQueue`, clears the staging
  rows and hides the dialog. Cancel discards them.

### Validation

A pure function over the rows, re-run on every edit:

- Destination must be project-relative with no `..` and not absolute — the same
  rules `validate_config` enforces for the content root.
- The destination's extension must equal the source's. `import_source` selects
  its importer from the destination extension, so renaming `.glb` to `.png`
  would silently pick the image importer.
- An unsupported source extension rejects the row with its reason, shown
  disabled rather than silently discarded. Needs a new
  `import::supported_extension(ext: &str) -> bool`, since `registered_importers`
  is private.
- A destination that exists on disk marks the row `Replaces`.
- Two rows resolving to the same destination reject both and block Confirm.

### Catalogue refresh

A new outcome in `project.rs` alongside the existing open/cancel/error arms.
`EditorCommand::OpenProject` currently bumps `ProjectState::generation` and
pushes `AssetEditorCommand::CloseAll`; an import must do neither, or every open
tab closes when a texture is imported.

`generation` exists to invalidate async results aimed at a project that has been
swapped out — `process_editor_commands` drops `Open` commands whose generation
no longer matches, as does `asset_request_is_current`. An import yields the same
project with assets added, so the refresh re-runs `discover_project`, calls
`publish_project_content`, replaces `state.project` and updates `status`, while
leaving `generation` and the open documents untouched. Addresses of existing
UUIDs survive a re-import — `import_source` consults `registry.get(asset_id)`
before generating an address — so `AssetEntry`s cached in open documents stay
valid.

### Status reporting

`ProjectState::status` carries progress: `Importing hero.glb (2 of 5)…`, then
the outcome. A failed import reports its error and the queue continues with the
next row; one bad file does not abandon the batch.

## Out of scope

**Re-imported assets already loaded do not refresh in the running editor.**
`AssetServer` keeps a `loaded_assets: HashSet<AssetId>` and `load()` skips
anything already in it. Re-importing a changed `hero.glb` rewrites the `.gasset`
and the catalogue, but a viewport already displaying that mesh keeps the old
bytes until the project is reopened. Hot-reload is a separate feature.

**Multi-file glTF is the user's responsibility.** A bare `.gltf` references
`.bin` buffers and textures by relative URI. The picker is multi-select, so the
user selects those files alongside it and gives them destinations that preserve
the relative layout. The editor does not parse URIs or follow references.

**Drag and drop.** `WindowEvent::DroppedFile` already reaches the ECS event
channel for free, but it carries no cursor position and `CursorMoved` is not
delivered during drags on X11/Wayland, so a drop cannot be attributed to a
panel. The button flow supersedes it; drops can be added later as a second
entry point onto the same staging dialog.

**Choosing the output path independently of the source path.** Would require a
destination override parameter on `import_source`.

**macOS.** `rfd`'s blocking `FileDialog` requires the main thread on macOS,
while this design calls it from a worker so the editor keeps rendering. The fix
is `rfd::AsyncFileDialog` driven by the `tasks` crate; it is not built now.

## Testing

Pure functions, no ECS:

- `stage_sources` prefills `assets/<file_name>` from an arbitrary source path.
- Validation: `..` and absolute paths rejected; extension mismatch rejected;
  unsupported source extension rejected with a reason; an existing destination
  marked `Replaces`; two rows sharing a destination both rejected.
- `import_source_into` copies an external source to its destination, leaves an
  in-project source at its own path when the destination matches, and errors
  before copying anything when the extension is unsupported.

Integration, over a tempdir project using `crates/obj-loader/tests/fixtures/square.obj`:

- Import writes the `.gasset` files, `.registry.toml` gains the new IDs, and
  `discover_project` surfaces them.
- A second import of the same source reuses the sidecar and every asset UUID is
  unchanged.

Resource-level, without the picker:

- A batch of N rows runs serially in FIFO order.
- A failed row leaves the queue running and the remaining rows complete.
- Refreshing after an import leaves `ProjectState::generation` unchanged and
  pushes no `CloseAll`.
