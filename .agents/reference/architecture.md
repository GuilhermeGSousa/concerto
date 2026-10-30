# Architecture

## Crates

Every directory under `crates/` is the package `concerto-<dir>`.

| Layer | Crates | What they hold |
|---|---|---|
| Core | `ecs`, `tasks`, `app`, `foundation` | ECS, signals, schedules; task pools; `App`, plugins, sub-apps, extract; assets, transforms, time, math |
| Assets | `asset-format`, `asset-import`, `asset-build`, `import`, `gltf`, `obj`, `bvh` | `AssetId` and `.gasset` framing; the `Importer` trait; `asset_id!` build tracking and the registry a game embeds; the import library behind the editor's Import…; the glTF, OBJ and BVH importers |
| Rendering | `render`, `mesh`, `skybox`, `world-grid`, `debug-gizmos`, `color`, `terminal-renderer` | wgpu renderer and materials; mesh data, bounds, ray tests; extra passes; a renderer that draws to a terminal |
| Simulation | `physics`, `jolt-ffi`, `animation`, `scene`, `director`, `gameplay` | Jolt and Rapier backends behind one API; the Jolt C++ bindings; animation graphs; scene spawn and capture; the virtual-camera stack; player and movement helpers |
| Interface | `window`, `winit`, `ui` | the window, input, and their events as plain data; the winit host; the engine's own UI |
| Tooling | `editor`, `reflect`, `project`, `mcp` | the editor; structural, path-addressed access to a value's fields and the `TypeRegistry` of what each type supports, which the ECS holds per world as `AppTypeRegistry`, a shared `Arc<RwLock<TypeRegistry>>` that the inspector, scenes and the `AssetServer`'s asset loads read; `cargo concerto`, which creates game projects and their editor crates; an MCP server whose tools are systems answering `McpCall` events |

`ecs`, `foundation`, `render`, and `reflect` each have a `macros` sub-crate.
`jolt-ffi` builds Jolt from the submodule at `crates/jolt-ffi/vendor/JoltPhysics`.

## Plugins

`DefaultPlugins` in [src/lib.rs](../../src/lib.rs) registers the standard
plugins in a fixed order. Whether an `App` has a window is its host's business,
not a plugin's. Plugin order is also system registration order, which decides
how unordered systems run.

## The frame

One `App` holds two worlds, `main` and `render`, as fixed sub-apps
([crates/app/src/subapp.rs](../../crates/app/src/subapp.rs)). Each frame:

1. The main world runs `Main`: `First`, then zero or more
   `FixedUpdate` + `LateFixedUpdate` steps, then `Update`, then `LateUpdate`,
   then `world.tick()` marks the frame boundary for change detection read
   outside any system.
2. Extract runs on the main thread with both worlds. It links each
   `SyncWithRenderWorld` entity (required by the camera, light, mesh, material and
   UI layout components) to a render-world mirror, despawns mirrors whose
   main entity is gone, then runs the render world's `Extract` schedule, where
   systems read the main world through `Extracted<T>`.
3. The render world runs `RenderMain`: `Render`, then `LateRender`.

`Startup` runs once in each world after every plugin has finished. Rendering is
not on its own thread: `RenderThreadPlugin` is empty.

Draw work in the render world is ordered with `RenderSet`
([crates/render/src/sets.rs](../../crates/render/src/sets.rs)).

## Hosts

An `App` does not run itself. A host builds it, feeds it, and steps it:

1. Create the `App` and insert a `Gpu`, a `DrawTarget`, and a `Window`.
2. Apply the recipe, a function that registers plugins and systems.
3. Poll `plugin_state()` until `Ready`, then call `finish_plugin_build()`.
4. Each frame, push `WindowEvent`s into the main world, call `update()`, and
   read `Window` for what the game asked for.

A host may also trigger signals on an `App` it holds, and read back the events
that `App` wrote. It never borrows the `App`'s world to change it. Between
frames it may call `serve(label)`, which runs one main-world schedule without
marking a frame boundary for change detection, extracting, or touching the
render world.

`concerto::run` is the winit host: it owns the event loop, the OS window, the
surface, and the device. `concerto::headless` builds an `App` with a device and
no window, for tests. `concerto_terminal_renderer::run` drives one from a
terminal. `concerto_mcp::host::run` serves one over MCP on stdin and stdout
with no window: it runs one frame before it starts the server, so no call is
served before that frame ends, then runs frames only while a call or the `App`
asks for them,
answers read-only calls between frames with `serve(McpRead)`, and quits when
stdin closes or the `App` closes its window; the editor's `--mcp` uses it. A `DrawTarget::Texture` makes an
`App` draw its window into a texture the host can sample.

A host never touches the render world once frames are running.

## ECS

- Systems are plain functions with up to twelve parameters.
- A system that uses `CommandQueue` gets its own sync point, where its
  commands are applied with exclusive access to the world. The sync point sits
  at the system's registration position, not after it in the run order; see
  [gotchas.md](gotchas.md).
- A `Signal` is dispatched to listeners. `World::add_listener` registers a
  global one; an entity carries at most one `Listener<S>` per signal type.
  `cmd.trigger(signal)` queues the dispatch for the next sync point.
- Ordering is by reference, never by registration: see
  [docs/scheduling.md](../../docs/scheduling.md).

## Assets

Source files (`.glb`, `.gltf`, `.obj`, `.bvh`, images) are imported once, ahead of
time. The running game never reads them.

- The editor's Import… writes one `.gasset` per sub-asset under
  `content/<stem>/` and writes `<source>.import.toml` beside the source to pin
  each sub-asset's UUID. It returns an `AssetEntry` for each output, and the
  editor adds them to its registry. There is no import command line.
- Code names an asset with `asset_id!("content/hero/mesh_0.gasset")`, resolved
  at compile time. The runtime finds files by UUID through the registry.
- No registry file exists, committed or at runtime. A game's `build.rs` calls
  `concerto_asset_build::track_assets`, which scans `content/` and writes the
  registry to `OUT_DIR`. `concerto::content_registry!()` embeds it, and
  `concerto::run` installs it. A `.gasset` moved or added in a file explorer
  or by git is picked up at the next build.
- `AssetRegistry` is the one catalogue of content on disk: id, address, kind.
  It is a resource; `registry.of_type::<T>()` lists assets without loading
  them. `AssetServer` resolves loads against a snapshot of it, refreshed by
  `sync_asset_registry` in `First` when the resource changes. The editor builds
  it by scanning headers once, when a project opens. An import inserts its
  entries and a scene save upserts the scene's, both in memory, with no rescan.
  The editor gives each hosted `App` the project root and a copy when the
  `App` is created, and the new registry whenever it changes.
- A scene is a tree of nodes whose components are stored by type name as
  reflected values (`concerto_reflect::ReflectedValue`), with field and variant
  names. A component takes part by deriving `Reflect` and `Default` with
  `#[reflect(SceneComponent)]` and being registered with `App::register_type`.
  Asset handles are stored as their `AssetId` and entity references as node
  indices, through `ReflectSceneLeaf` type data; capture lists every asset a
  handle names in `referenced_assets`.
- Re-importing a source keeps its asset IDs. An output whose header no longer
  carries import provenance was saved by the editor and is not rewritten.

Each example is a self-contained project: its `build.rs` copies `content/` to
`<exe-dir>/<package>-content/content/`, so examples sharing one `target/` do not
overwrite each other's content. `content_root_for_exe` picks that directory as
the native content root when it holds a `content/` directory, and the
executable's directory otherwise.

## Editor

A game that has an editor is a library plus a thin binary; `tech-demo` is the
one example built that way, and the other four are binaries only. The library
exposes one recipe, `pub fn game(app: &mut App)`, which registers the game's
plugins, components, systems, and startup scene; `main` is
`concerto::run(<game>::game, concerto::content_registry!())`. The editor is a
library too, built per game: the game's `editor/` crate, named `<game>-editor`
and generated by `cargo concerto editor`, depends on the game's library and calls
`concerto_editor::run(<game>::game, <project root>)`. A game without an
`editor/` crate has no editor:

```sh
cargo run -p tech-demo-editor -- --project <scratch copy of examples/tech-demo>
```

`--project <dir>` opens that project's `content/` tree instead of the game's
own.

The editor is a host as well as a hosted program. Four kinds of `App` exist
while it runs:

- **The editor `App`,** driven by the winit host, or by the MCP host under
  `--mcp`. It owns the window, the UI, the panels, and the documents. It
  draws no 3D and holds no scene entity.
- **An edit `App`,** one per open scene, owned by the editor. It is created
  with the edit role, `App::for_editing()`, and built from the recipe passed to
  `EditorPlugin` plus `EditToolsPlugin`; the recipe is the game's, so the
  edit `App` has the game's components and edit-time systems. Only the
  main-world systems marked `.at_edit_time()` run in it; the others stay as inert stand-ins. An unmarked
  listener passed to `App::add_listener` is not added; a listener added through
  `World::add_listener` or inserted on an entity, including from a plugin's
  `build`, is never filtered. Its render world runs everything. It is
  held as a `HostedApp` component on the scene's document, and dropped with
  it. It holds the scene's entities and the scene tools: the editor camera, the
  grid, picking, and the gizmo. It draws its window into a texture that the
  viewport panel shows.
- **A play `App`,** while the Scene tab plays its scene. Play (the tab's
  button, or Ctrl+P) saves the scene if needed, then builds an `App` with the
  game role, `App::new()`, from the same recipe plus `EditToolsCorePlugin`
  (selection and edits, no viewport tools), with the project's content and the
  open scene's saved file as its startup scene. While it builds it is a
  `PendingPlayApp` beside the edit `App`. Once ready, the edit `App` moves to
  `ParkedEditApp`, where it is not stepped, and the play `App` becomes the
  document's `HostedApp`, so the stepping, the viewport, input, and the panels
  follow it. Stop drops it and puts the parked edit `App` back. The document's
  `PlayState` (`Editing`, `Starting`, `Playing`) says which is which, and
  `InputOwner` says whether the game or the editor has the window's input.
- **A preview `App`,** one per open document whose asset editor returns a
  `PreviewSpec` from `AssetEditor::preview`. `PreviewHostPlugin`
  (`crates/editor/src/preview.rs`) builds it with the preview role,
  `App::for_previewing()`, from the same recipe plus `PreviewPlugin` (an orbit
  camera, the grid, a light) and the spec's `build`, and holds it as the
  document's `HostedApp` with `HostRole::Preview`. It runs the main-world
  systems and listeners marked `.at_edit_time()` or `.at_preview_time()` (the
  `Preview` tag), so animation plays in it while the startup scene and the
  game's unmarked systems stay out. It has no scene file and no scene tools.
  The asset editor drives it with signals through `PreviewSignals` and reads
  its events as `Hosted<E>`.

`crates/editor/src/hosted/` is the host. Each frame, in `First`, it steps the
document's `HostedApp` on the active tab, the edit `App`, the play `App` while
playing, or the preview, while the viewport is visible, after pushing the
viewport's size and input into it. That document carries `ViewportTarget`;
every role but a preview also carries `EditTarget`, which the panels read. An
edit `App` that is not stepped still takes signals, which is how a tab that is
not in front is saved; a preview in the background is frozen.

Panels reach the edit `App`, or the play `App` while playing, through two
system parameters: `EditWorld<T>` reads its world, and `EditWorldSignals` triggers
signals such as `EntityEdit` and `Select` in it. The scene tools in
`crates/editor/src/tools/` apply each change and report back through the
`SceneEdited`, `SceneStatus`, and `RevealEntity` events. A `forward_events::<E>`
system per type, registered with `forward_hosted_event::<E>()`, reads them
through `HostedAppEvents<E>` in `First` and writes `Hosted<E>` events in the
editor's world, which systems read with `EventReader<Hosted<E>>`. Whether the
scene has unsaved edits is state, `SceneDirty`, kept in the edit `App`: edits set
it and a successful save clears it, and the editor's `Unsaved` follows it. A save
asks the edit `App` for the scene directly with `HostedApp::capture_scene`.
The inspector edits components through `PropertyEditor<T>` adapters over
`reflect`. Each asset type has one editor, registered with
`register_asset_editor`. [crates/editor/README.md](../../crates/editor/README.md)
covers all of it.
