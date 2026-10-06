# Concerto editor

The engine-native editor uses the engine's ECS, renderer, assets, scene, and UI
crates. It opens project content, edits scenes through the inspector, and saves
them. It does not import assets; importing belongs to the `import` CLI.

```sh
cargo run -p concerto-editor -- --project examples/render-test
```

Add `--decorated` to use the window manager's title bar.

Selecting a supported asset in Curiosities opens its editor tab. Each asset type
has one editor: another Scene reuses the Scene tab. The old preview, camera,
selection, and edits remain until its replacement loads successfully.
A failed replacement preserves them. Selecting the displayed asset cancels a
pending replacement. Unsupported assets remain selectable without opening a tab.

With no tabs open, the editor shows an empty viewport. Switching projects closes
all editors. Tabs scroll horizontally with the wheel or trackpad, and the active
tab is revealed.

## Saving

An edited document is dirty until it is saved, and its tab shows a dot. Ctrl+S
saves the active document. A scene that came from `import` is never overwritten:
its first save writes an authored copy beside it, named `<name>-level`, and the
tab switches to that copy; later saves overwrite the copy. Re-importing the
source leaves the copy alone. A failed save reports why in Chatter and leaves
the document dirty.

Replacing the open scene, closing a dirty tab, switching project, and closing
the window all ask first when there are unsaved changes: Save, Discard, or
Cancel. Escape cancels.

The scene root shown in the hierarchy is the editor's wrapper around the scene,
not part of it, so the inspector shows nothing for it.

The scene hierarchy supports selection, filtering, expansion, and keyboard
navigation. The inspector uses typed property adapters. Its "add component" row
adds any `register_editable` component the entity lacks, at its `Default` value;
a card's ⋯ menu removes one. `register_inspectable` components, such as `Camera`,
are edited and removed but never offered. Right-drag looks around;
WASD and Q/E move while looking, Shift boosts speed, middle-drag pans, and the
wheel dollies or adjusts flight speed while looking. F frames the selection;
Shift-F frames the scene.

## Custom property editors

The inspector edits live component values through typed `PropertyEditor<T>`
adapters. `editable` provides structural access; adapters own snapshots, widgets,
and validated edits. A registered editor can handle a whole struct or an opaque
type, and can replace any built-in numeric editor. Snapshots are cached per
component, with snapshots owned by property-row entities. Component cards carry
`InspectedComponent` and can be accessed through ordinary ECS queries.
Unchanged components reuse their snapshots;
component changes, selection changes, and editor registration changes refresh
them. Widget input and focused edit buffers continue updating every frame.

See the [adapter guide](src/inspector/custom_editors.md) and the
[complete custom widget example](examples/custom_property.rs). The example
includes registration, a non-Clone domain type, click handling, snapshot refresh,
and validation. Run its headless demonstration with:

```sh
cargo run -p concerto-editor --example custom_property
```

## Custom asset editors

Asset editors are separate from property editors. Register the asset with
`app.register_asset::<T>()`, then use `AssetEditorAppExt`:

```rust,ignore
app.register_asset::<Dialogue>();
app.register_asset_editor::<Dialogue>(DialogueEditor)?;
```

Register after `EditorPlugin` installs its registry. Dispatch uses the asset's
serialized kind (`Asset::name()`), which is what catalogue entries carry.
Duplicate editor registrations are rejected.

Implement `AssetEditor::build` using its `EntityCommandQueue` to queue the
editor's components and children. The
editor entity carries `EditorDocument` and `EditorHosts`; custom systems use
ordinary `Query` and `ProjectState` access to load assets and update their
components. Content browser and Chatter remain shared. Tag additional owned
roots with `EditorOwned(document)` so closing the editor cleans them up
recursively. Headless hosts have no UI containers.

The [complete custom asset editor example](examples/custom_asset.rs) loads a
non-Clone Dialogue asset, builds its own text widget, replaces its contents,
rejects invalid input without mutation, and registers through the public API.
Run its headless smoke test with:

```sh
cargo run -p concerto-editor --example custom_asset
```

The example uses a synchronous ECS loading system for clarity. Asynchronous
systems capture `EditorDocument.request_generation` and
`project_generation`; before mutating domain state, use
`asset_request_is_current` to reject canceled, closed, or superseded results.
Call `finish_asset_request` with a mutable document borrow, the captured request
generation, the live `ProjectState::generation`, and success or an error to update the tab's current asset and status. Failed loads must leave
the previous presentation intact.

To take part in saving, call `EditorDocument::mark_edited` when the user changes
something, or `mark_entity_edited` for an entity under an `EditorOwned` root.
Ctrl+S and the unsaved-changes prompt add `SaveRequested` to the document: write
the asset, call `mark_saved` with the revision you captured, and remove the
marker. On failure remove the marker and leave the document dirty.

Register custom interaction systems in `Update` after `EditorPlugin`; lifecycle
processing runs before viewport and transform propagation. Workspace visibility
uses resource change detection and host component changes; tab switches release
transient input without resetting the camera pose. Loading and
saving belong to the custom editor.
Only the Scene editor currently owns a 3D preview; multiple independent 3D
editor worlds and render suppression are not implemented.
