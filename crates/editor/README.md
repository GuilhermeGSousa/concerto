# Rabbithole editor

The engine-native editor uses the engine's ECS, renderer, assets, scene, and UI
crates. It opens imported content and allows temporary inspector edits. It does
not import or save assets; importing belongs to the `import` CLI.

```sh
cargo run -p editor -- --project examples/render-test
```

Add `--decorated` to use the window manager's title bar.

Selecting a supported asset in Curiosities opens its editor tab. Each asset type
has one editor: another Scene reuses the Scene tab. The old preview, camera,
selection, and temporary edits remain until its replacement loads successfully.
A failed replacement preserves them. Selecting the displayed asset cancels a
pending replacement. Unsupported assets remain selectable without opening a tab.

Closing a tab discards its temporary state. With no tabs open, the editor shows
an empty viewport. Switching projects closes all editors. Tabs scroll
horizontally with the wheel or trackpad, and the active tab is revealed.

The scene hierarchy supports selection, filtering, expansion, and keyboard
navigation. The inspector uses typed property adapters. Right-drag looks around;
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
cargo run -p editor --example custom_property
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
cargo run -p editor --example custom_asset
```

The example uses a synchronous ECS loading system for clarity. Asynchronous
systems capture `EditorDocument.request_generation` and
`project_generation`; before mutating domain state, use
`asset_request_is_current` to reject canceled, closed, or superseded results.
Call `finish_asset_request` with a mutable document borrow, the captured request
generation, the live `ProjectState::generation`, and success or an error to update the tab's current asset and status. Failed loads must leave
the previous presentation intact.

Register custom interaction systems in `Update` after `EditorPlugin`; lifecycle
processing runs before viewport and transform propagation. Workspace visibility
uses resource change detection and host component changes; tab switches release
transient input without resetting the camera pose. Loading and
temporary state belong to the custom editor.
Only the Scene editor currently owns a 3D preview; multiple independent 3D
editor worlds and render suppression are not implemented.
