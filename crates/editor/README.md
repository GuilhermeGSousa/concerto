# Concerto editor

The engine-native editor uses the engine's ECS, renderer, assets, scene, and UI
crates. It opens project content, edits scenes through the inspector, and saves
them. It also imports source files into the open project; see Importing.

The editor is a library with no binary of its own. It is built once per game,
by the game's `editor/` crate, which links the game's library so that every
scene is edited with the game's own systems and components. A game without an
`editor/` crate has no editor. `cargo concerto editor <dir>` generates one.

```sh
cargo run -p tech-demo-editor -- --project <scratch copy of examples/tech-demo>
```

Add `--decorated` to use the window manager's title bar.

A game's `editor/` crate starts the editor with
`concerto_editor::run(recipe, project_root)`. `recipe` is the game's
`fn(&mut App)`, which builds the `App` of each scene being edited. `project_root`
is the project the editor opens; `--project <directory>` on the command line
overrides it. `--decorated` and `--help`/`-h` work as above, and a bad argument
prints the usage and exits with status 2. `run` sets up logging with
`env_logger`, ignoring a logger that is already installed.

`--mcp` opens no window: the whole editor runs headless and is served over MCP
on stdin and stdout, drawing into a 1600×900 picture unless
`--mcp-size <width>x<height>` names another size. `--decorated` is ignored
there, and `--mcp-size` without `--mcp` is a bad argument. Frames run only
while a call needs one or the editor is busy: a project or scene opening, a
game starting or playing, a save under way. When the client disconnects the
editor quits and discards unsaved edits. Without a GPU adapter it exits with
status 1 and a message naming lavapipe.

The tools so far: `status` reports the project and its status line, the open
documents, the active one, the selection in the edited scene, the viewport's
size, and whether the editor is busy. `list_assets` lists the project's assets
as `address · kind · id` lines, filtered by `query`, `kind` and `folder`, 50 at
a time unless `limit` says otherwise, from `offset`; a longer list ends with the
call that shows the rest. `scene_tree` shows the edited scene, or while a game
plays its top-level entities as the hierarchy does, one `id name [components]`
line per entity, indented, from `root` if given, `depth` levels deep (3) and at
most `limit` entities (200); a cut subtree or a cut list ends with the call that
continues it. `find_entities` lists the entities whose name contains `name`,
ignoring case, and that carry `component`, by short name or full path, each with
its path of names from the top, 50 unless `limit` says otherwise. Ids are scene
entities written as `42v3`; a `root` that names no live entity answers
`unknown_entity`, and one whose slot holds a newer entity `stale_entity`. Both
need an open scene. All four answer between frames, without running one.

## How the crate is laid out

The editor is one `App`, the one the window host drives. It holds the window,
the UI, the panels, and the documents, and no scene entity. A scene being
edited lives in an `App` of its own, the edit `App`, which the editor builds,
steps, and drops. The crate has three parts:

- `src/hosted/` builds an `App` from a recipe, holds it on its document as a
  `HostedApp`, steps it, feeds it the viewport's input, and carries messages
  both ways.
- `src/tools/` is everything that runs inside an edit `App`: the editor camera,
  the grid, picking, the selection box, the gizmo, and the listeners that apply
  each edit. `EditToolsPlugin` installs it. The half that edits and captures a
  scene needs no renderer; the viewport half is added when the recipe has one.
- The panels (hierarchy, inspector, viewport, tabs, content, dock) run in the
  editor's `App`.

The recipe is the `recipe: fn(&mut App)` field of `EditorPlugin`.
`concerto_editor::run` passes the game's recipe, such as `tech_demo::game`, the
same function the game's own `main` hands to `concerto::run`. Each scene's edit
`App` is created with `App::for_editing()` and built from it when the scene file
has loaded, and dropped when its tab closes or another scene replaces it. In an
edit `App` only the main-world systems marked `.at_edit_time()`
(`concerto_app::edit_time::AtEditTime`) run; the others keep their place in
the schedule and do nothing, and render-world systems all run. An unmarked
listener passed to `App::add_listener` is not added. A listener added through
`World::add_listener` or inserted on an entity is never filtered, including
one added that way from a plugin's `build`. Everything registered inside an
edit `App` through `add_system` or `App::add_listener`, the tools in
`EditToolsPlugin` and the systems and listeners of a test recipe included,
must be marked.

A panic while the edit `App` is built does not take the editor down. That
covers the recipe and every plugin's `build`, each plugin's `ready` and
`finish`, the `Startup` schedule, and the ready step that publishes the content
and opens the scene. The half-built `App` is dropped once every system it was
running has returned, the panic is logged, and the Scene tab shows "Could not
open scene: <panic message>", as a failed load does.

A panel reaches the scene in two ways and no other:

- `EditWorld<T>` reads the edit `App`'s world: `EditWorld<Query<&Name>>`,
  `EditWorld<Res<Selection>>`. `T` is any read-only system parameter.
  `EditWorld<Res<T>>` skips its system with a warning on every frame the scene's
  world has no `T`, so name only resources the tools always insert, such as
  `Selection` and `AppTypeRegistry`.
- `EditWorldSignals` sends a signal to it: `scene.trigger(Select(Some(entity)))`.
  The listeners run when the system's commands are applied.

Both reach the document marked `EditTarget`: the front tab's scene, once it
has an `App`. A system that takes either is skipped while no document has the
marker, which includes while its game is starting. While the game plays, both
reach the running game's world instead of the scene's. Code changes the target
only by triggering `SetEditTarget`; its listener moves the marker. A panel
therefore keeps what it shows in a resource of its own, fills it in a system
that takes `EditWorld<T>`, and empties it in an `On<Remove<EditTarget>>` listener.
`has_changed()` and `Changed<T>` mean nothing through `EditWorld<T>`; the two
worlds keep separate clocks.

An `Entity` of the edit `App`'s world is not an entity of the editor's. A field
in the editor's world that holds one is named for it: `scene_entity`,
`scene_target`, `scene_parent`.

Selecting a supported asset in Curiosities opens its editor tab. Each asset type
has one editor: another Scene reuses the Scene tab. The old preview, camera,
selection, and edits remain until its replacement loads successfully.
A failed replacement preserves them. Selecting the displayed asset cancels a
pending replacement. Unsupported assets remain selectable without opening a tab.

Code learns how an open ended from events in the editor's world, written in the
frame the status text changes. Opening a project ends in `ProjectOpened { root }`
or `ProjectOpenFailed { root, message }`; a cancelled folder pick and a project
whose content cannot be activated are failures. A Scene document's request ends
in `SceneOpened { document, asset }` or `SceneOpenFailed { document, asset,
message }`: a scene that cannot be read, a build that panics, a replacement
refused over unsaved changes, and a scene asked for with no project open each
fail, and `message` is the status the tab shows. A request that is cancelled or
superseded writes neither, and neither does selecting the scene already shown.

With no tabs open, the editor shows an empty viewport. Switching projects
closes all editors. There is only one Scene tab, from the first scene opened
until the project changes or the editor quits; it has a Play button where other
tabs have a close button. An asset editor says whether its tabs can be closed
with `AssetEditor::closable`, which is true unless it says otherwise; the Scene
editor's is false. `AssetEditorCommand::Close` still closes a Scene document.
Tabs scroll horizontally with the wheel or trackpad, and the active tab is
revealed.

## Importing

**Import…** beside the Curiosities search opens a file picker for glTF, OBJ, and
image sources; several files can be picked at once. A dialog then lists each
file with the project-relative destination it will be copied to, which you can
edit before confirming. Enter confirms and Escape cancels. Files are imported
one at a time, and the catalogue refreshes after each without closing open tabs.

Importing over an existing destination replaces it and keeps its asset IDs, so
everything that refers to those assets stays wired. A file that is already
inside the project is imported where it is, not copied.

The files a source refers to come along with it: an OBJ's `.mtl` and the
textures it names, a glTF's `.bin` buffers and external images. Each keeps its
position relative to the source, and replaces a file already there. If one would
land outside the project, the import fails and nothing is copied; pick a
destination deeper inside the project. Chatter names the first failure of a
batch; the log has every one in full.

The `import` CLI does the same work from a terminal; `--to <destination>` brings
in a source from outside the project.

## Playing

The button on the Scene tab plays the open scene; while the game plays it reads
"Stop" and pressing it stops the game. Ctrl+P does the same. The game is built
inside the Play press, so the editor pauses briefly and the button goes
straight from "Play" to "Stop". Play saves the scene first if it has unsaved
changes; if that save fails, the game does not start and the tab says why.
While a replacement scene is loading, Play is refused with "Wait for the scene
to load".

The game is built from the same recipe as the standalone game, in an `App` of
its own, and starts from the open scene's saved file, which replaces any
startup scene the recipe declared. Its content comes from the open project.
While it builds, the viewport keeps showing the scene and the hierarchy and
inspector change nothing. Once it is ready it takes the scene's place: the
viewport shows the game, and the hierarchy and inspector act on the running
game. The scene's own `App` waits, untouched, and is not stepped. If the game
cannot be built, the tab shows "Could not start the game: <message>" and the
scene stays as it was.

While playing, the hierarchy lists every top-level entity of the game's world
with its children nested: the scene's root and whatever the game spawned, such
as its character, data entities included. Only the entities that
`World::add_listener` spawns to hold a listener, which carry `GlobalListener`,
are left out. Any listed entity can be renamed, duplicated, or deleted. The `+`
adds a child to the selection, or an entity at the top level when nothing is
selected or the game has despawned the selection.

Stop drops the game and returns to the scene exactly as it was before Play.
Changes made while playing are lost and never mark the scene unsaved. Saving is
refused while the game starts or plays, with "Stop to save". Opening another
scene, opening another project, and quitting stop the game first, before any
question about unsaved changes. Play does nothing while that question is open.
A game that asks to quit is stopped.

### Who has input

The game has the editor's input from the moment it starts playing: every key,
mouse button, wheel turn, and mouse motion goes to it wherever the pointer is,
Esc included, and the editor's panels and shortcuts see none of them. The
viewport has an accent border and says "Shift+F1 to release". Three things give
input back to the editor: a click anywhere but the viewport's picture, which
then lands on whatever is under the pointer, the tab strip and the panels drawn
over the viewport included; Shift+F1; and the editor window losing focus. The
game is told it lost focus, so nothing stays held in it, and any cursor grab it
asked for is let go. The border turns muted and the viewport says "Click to
play"; the game keeps running but gets no input, and the editor works as usual.
A click on the viewport's picture gives input back to the game without reaching
it as a click. Regaining window focus does not.

Ctrl+P stops the game whichever side has input, and Shift+F1 works while the
game has it; the host reads both from the window's own events while the game
has input. `hosted::input::ReleaseInput(document)` gives input to the editor
from code.

## Saving

An edited document is dirty until it is saved, and its tab shows a dot. Ctrl+S
saves the active document over the scene it was opened from, imported or not.
A saved scene belongs to the editor from then on: re-importing its source
leaves it alone and re-imports the source's other outputs. A failed save
reports why in Chatter and leaves the document dirty.

Replacing the open scene, closing a dirty tab, switching project, and closing
the window all ask first when there are unsaved changes: Save, Discard, or
Cancel. Escape cancels.

The scene root shown in the hierarchy is the editor's wrapper around the scene,
not part of it, so the inspector shows nothing for it.

The scene hierarchy supports selection, filtering, expansion, and keyboard
navigation. Right-click a row to add a child, duplicate, rename, or delete it;
the `+` in the panel header adds a child to the selection, or to the scene when
nothing is selected. Delete removes the selected entity and its descendants,
Ctrl+D duplicates it beside itself, and F2 edits its name in the field at the
top of the inspector. New and duplicated entities go last among their siblings.
The scene root can be given children but not deleted, duplicated, or renamed.
A duplicate fails when the entity refers to something outside itself, such as a
skinned mesh copied without its bones; Chatter says why.

Panels change the scene only by triggering signals through `EditWorldSignals`,
for example `scene.trigger(EntityEdit::Delete(entity))` or
`scene.trigger(Select(Some(entity)))`. The scene tools in `src/tools/` own the
listeners: one applies each kind of edit and moves the selection. The tools
know nothing about documents or panels. They report back through events that
the editor re-writes in its own world as `Hosted<E>` events carrying the
document, read with `EventReader<Hosted<E>>`: `SceneEdited`, which says the
scene changed; `SceneStatus`, which carries why an edit was refused to Chatter;
and `RevealEntity`, which the hierarchy scrolls to. `HostedAppEvents<E>` is the
system input that reads `E` from every hosted `App`, keeping one cursor per
hosted world, and `forward_hosted_event::<E>()` forwards a type of your own.
Whether the scene has unsaved edits is state kept in the edit `App`
(`SceneDirty`); the document's dot follows it. A save asks the edit `App` for the
scene directly and writes it in the same step. The saved file lists every asset
the scene's components name, including ones added in the editor.

The inspector uses typed property adapters. Its "add component" row
adds any registered type declaring `#[reflect(Component, Default)]` that the
entity lacks, at its `Default` value; a card's ⋯ menu removes one. A type that
declares `Component` without `Default`, such as `Camera`, is edited and removed
but never offered.

Left-clicking a mesh in the viewport selects its entity, scrolls the hierarchy
to it, and draws a box around it; clicking empty space clears the selection. A
press that moves more than a few pixels is not a click. Picking tests the
mesh's triangles, so a mesh seen through a gap in another is the one selected.
Limits: a skinned mesh is picked and boxed in its bind pose; a click selects
the mesh entity itself, which in an imported model is often a child of the node
you think of as the object; transparent texels still count as a hit; and
entities without a mesh are selected from the hierarchy.

The selected entity shows a transform gizmo: red, green, and blue handles for
the world X, Y, and Z axes, with the one under the pointer in yellow. W shows
arrows that move the entity along an axis; E shows rings that rotate it about
one. Drag a handle with the left button. A ring seen nearly edge-on cannot be
dragged; orbit a little first. The gizmo keeps its size on screen, is hidden
while the camera is being flown, and is not shown for the scene root. A drag
marks the scene dirty when it ends.

Right-drag looks around;
WASD and Q/E move while looking, Shift boosts speed, middle-drag pans, and the
wheel dollies or adjusts flight speed while looking. F frames the selection;
Shift-F frames the scene.

## Custom property editors

The inspector edits live component values through typed `PropertyEditor<T>`
adapters. `concerto-reflect` provides structural access; adapters own snapshots,
widgets, and validated edits. A registered editor can handle a whole struct or an
opaque type, and can replace any built-in numeric editor. Snapshots are cached per
component, with snapshots owned by property-row entities. Component cards carry
`InspectedComponent` and can be accessed through ordinary ECS queries.
Unchanged components reuse their snapshots;
component changes, selection changes, and editor registration changes refresh
them. Widget input and focused edit buffers continue updating every frame.

Registration is split between the two `App`s. A component states what it
supports on its derive, `#[derive(Reflect)] #[reflect(Component, Default)]`,
and is registered in the scene's `App`, by the plugin that owns it or by the
recipe, with `App::register_type::<T>()`: `TransformPlugin` registers
`Transform`, and `RenderPlugin` registers `Light` and `Camera`. Registration
fills the world's `AppTypeRegistry`, which is part of the engine and needs no
editor. `ReflectComponent` and `ReflectDefault` must be in scope where the
derive names them. The widget is registered on the editor's own
`App`, through `EditableApp::register_property_editor`, which needs
`InspectorPlugin` first. A recipe that adds a component of its own looks like
this:

```rust,ignore
fn recipe(app: &mut App) {
    app.register_plugin(concerto::DefaultPlugins);
    app.register_type::<Setting>();
}
```

A widget queues its edit on `PropertyCommits`. The inspector checks it against
the widget registry and sends it to the scene, where the tools write the value.

See the [complete custom widget example](examples/custom_property.rs). It
includes both registrations, a non-Clone domain type, click handling, snapshot
refresh, and validation. Run its headless demonstration with:

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
systems capture the `AssetEntry` in `EditorDocument.pending` when they start;
before mutating domain state, use `asset_request_is_current` with it to reject
cancelled or superseded results. Call `finish_asset_request` with a mutable
document borrow, the captured `AssetEntry`, and success or an error to update
the tab's current asset and status. A result for a closed tab has no document to
land on, and switching project closes every tab. An `AssetEditorCommand::Open`
for an asset that is not in the current project's registry is ignored. Failed
loads must leave the previous presentation intact. Finishing a replacement does not clear
`Unsaved`: refuse a replacement that finishes loading while the document is
`Unsaved`, as the Scene editor does, or remove the marker yourself.

To take part in saving, insert `Unsaved` on your document when the user changes
something. Ctrl+S and the unsaved-changes prompt add `SaveRequested` to the
document: remove `Unsaved` at the moment you capture what you will write, write
the asset, and remove `SaveRequested`. If the write fails, insert `Unsaved`
again and leave an error in the document's status. Removing the marker at
capture means an edit made while a slow save is in flight marks the document
again and is not lost.

Register custom interaction systems in `Update` after `EditorPlugin`. Workspace
visibility uses resource change detection and host component changes; a tab
switch, and a scene opening, being replaced, or closing, release transient
input. Loading and saving belong to the custom editor.
Only the Scene editor hosts an `App`. Each open scene has one, with its own
camera, selection, and tools; only the one on the active tab is stepped, and
only while the viewport is visible. Hosting is not part of the `AssetEditor`
contract.
