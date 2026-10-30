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

## Agents (MCP)

`--mcp` runs the editor for an agent: the whole editor, panels included, with
no window, served over [MCP](https://modelcontextprotocol.io) on stdin and
stdout. Logs stay on stderr.

```sh
cargo run -p tech-demo-editor -- --mcp --project <scratch copy of examples/tech-demo>
```

`.mcp.json` at the repository root registers it as the `concerto` server, so an
agent working in the repository can start its own editor. That entry opens the
game's own project; an agent that will change anything opens a scratch copy
with `open_project` first. The entry runs `cargo run`, which compiles the editor
before it starts and can outlast an MCP client's connect timeout, so build it
first with `cargo build -p tech-demo-editor`.

The editor draws into a 1600×900 picture unless `--mcp-size <width>x<height>`
names another size. `--decorated` is ignored there, and `--mcp-size` without
`--mcp` is a bad argument. Without a GPU adapter it exits with status 1 and a
message naming lavapipe.

Frames run on demand, only while a call needs one or the editor is busy: a
project or scene opening, the edited scene's assets still loading, a game
starting or playing, a save under way, or text in the panels changed in the
last frame and may not be drawn yet. An idle editor runs none, so once
`status` reports `busy: no` the viewport shows the scene with everything it
loads and the panels show what they hold. The tools that look answer between frames, without running one; the
tools that act run in frames, one at a time, and a second one waits until the
first has answered.

When the client disconnects, the editor quits and discards unsaved edits,
answering its own unsaved-changes prompt with Discard. An agent that wants its
edits keeps them by saving first, with `save`.

### Tools

`status` reports the project and its status line, the open documents, the
active one, the selection in the edited scene, the viewport's size, and whether
the editor is busy. `list_assets` lists the project's assets as
`address · kind · id` lines, filtered by `query`, `kind` and `folder`, 50 at a
time unless `limit` says otherwise, from `offset`; a longer list ends with the
call that shows the rest. `scene_tree` shows the edited scene, or while a game
plays its top-level entities as the hierarchy does, one `id name [components]`
line per entity, indented, from `root` if given, `depth` levels deep (3) and at
most `limit` entities (200); a cut subtree or a cut list ends with the call that
continues it. `find_entities` lists the entities whose name contains `name`,
ignoring case, and that carry `component`, by short name or full path, each with
its path of names from the top, 50 unless `limit` says otherwise. `inspect`
answers `entity` as JSON: its id, its name, and each component the inspector
lists, or only those `components` names by short name, ignoring case, or full
path. Each component has its short `name`, its full `type`, its `value`, and
its `paths`: every property path, dot-separated, with `adapter` saying whether
an inspector widget edits it. A path that names an enum or an `Option`, such
as `light_type` above `light_type.cone_angle`, has `"enum": true` and no
adapter, even where the inspector shows a variant picker for it, because
`set_property` writes it whole (below). A component without reflection data has
`"value": null, "unavailable": true` and no paths. A list longer than 16
elements shows its first 16 and ends with a `"… N more, M in all"` string, so a
cut list is not a value to write back. Ids are scene entities written as
`42v3`. `screenshot` answers a PNG: with `target` `viewport`, the picture the
viewport draws, the edited scene or the game while it plays, followed by the
camera's pose while editing; with `editor`, the whole editor, panels included,
at its window size. A picture longer than `max_edge` pixels (1024) on either
side is scaled down to it, keeping its shape; a smaller one keeps its size. It
is the last frame drawn: the viewport's picture changes only when the edit
`App` steps. `scene_tree`, `find_entities`, `inspect` and `screenshot`'s
viewport need an open scene. These six look.

`open_project` opens `path` and answers once the catalogue is read.
`open_asset` opens `asset`, an address or an id from `list_assets`, in its
editor: a scene answers once it has loaded, and at once if it is already shown;
another kind answers once its document exists, and a kind with no editor
answers `open_failed`. `close_document` closes the document `status` lists as
`document`. When one of these three would raise the unsaved-changes prompt,
the call cancels the prompt and answers `unsaved_changes`, naming the
documents, once the prompt has closed, unless it says `unsaved`: `"discard"`
drops those changes and `"save"` saves them first, and the call carries on as
if nothing had been unsaved. A save that fails answers `save_failed` with the
editor's message, and the documents keep their changes. A call that would lose
nothing ignores `unsaved`.
`select` selects `entity`, or clears the selection without one. `frame` selects
`entity` and frames it, or frames the whole scene without one. `set_camera`
places the camera at `position`, `[x, y, z]`, facing the point `look_at`, or
turned by `yaw` and `pitch` given together: radians, where yaw 0 faces -Z and a
positive yaw turns left, and pitch is up from level, kept within ±1.55. Giving
both forms, neither, a `look_at` at the position, or a number that is not
finite in an `f32` (beyond about ±3.4e38) answers `invalid_arguments`. `frame` and `set_camera` answer the camera's pose, its
position, yaw, pitch and the direction it looks along, once the edit `App` has
stepped and the camera shows it; the edit `App` steps only while the viewport
shows the scene. While a game plays both answer `refused`, since the game has
no editor camera. All three need an open scene.

The edit tools change the edited scene, or the running game while it plays, by
triggering the signal the panels trigger, and answer once the edit `App`
reports the outcome; an edit the tools refuse answers `refused` with their
status text, the same text the scene's status line shows. `create_entity` adds
an entity named Entity as the last child of `parent`, or of the scene's top
level without one, and answers `Created <id> Entity in <parent>.`.
`delete_entity` deletes `entity` and its descendants and answers what is
selected afterwards. `duplicate_entity` copies `entity` beside it and answers
the copy's id. `rename_entity` renames `entity` to `name`, trimmed; an empty
name is refused. `add_component` adds `component` at its default value and
answers it in `inspect`'s form; `remove_component` removes it. A component is
named by its short name, ignoring case, or by its full path when the short name
is ambiguous; `add_component` looks among the components registered in the
scene's `App`, the others among those the entity carries. `set_property` writes
`value`, JSON in `inspect`'s form, into the property at `path`, one of the
paths `inspect` lists for `component`, and answers the component in `inspect`'s
form. A struct value may leave fields out to keep them. Where `inspect` says an
adapter edits the path, the value goes through the adapter's
`PropertyEditor::edit_to` and `apply`, as an edit in the inspector does, so the
adapter's validation holds: a value it refuses answers `refused` and leaves the
component as it was. A path with no adapter is written from the JSON directly.
On either route a value that does not decode to the property's type answers
`invalid_value` and leaves the component as it was, an entity reference is a
live id such as `42v3`, and an asset handle is an asset id that loads through
the edit `App`'s `AssetServer`, as a spawned scene's handles do. A handle field
is edited by the asset picker, so an id that is not one of the project's assets
of the field's kind answers `refused`.
A path `inspect` marks `enum` takes the whole enum, its variant's name for a
variant without fields (`"Point"`) or an object with the variant's name as its
one key (`{"Spot": {"cone_angle": 40}}`), so it can switch variants; a field
left out of the new variant takes its default, and an unknown variant answers
`invalid_value`, naming the variants. A path that names a struct above a property is
not one `set_property` takes, so an adapter's validation holds for the fields
it edits.
A component that `inspect` lists as one value, a marker or an opaque type, has
the path `""`. The scene root is the editor's wrapper and none of these change
it. All seven need an open scene.

`save` writes `document`, or the active document without one, to its file as
Ctrl+S does, and answers `Saved <address>.` once it is written. A document with
no unsaved changes answers `Nothing to save in document <id> <address>.`, which
is not an error. A save that fails answers `save_failed` with the editor's
message, as does a save while the game plays (`Stop to save`), and a scene still
loading answers `busy`. `play` starts the active scene's game in the viewport,
as the play button does, and answers once the game has built, plays, has
spawned every scene it loaded, and has no asset loads in flight: `Playing
<address> in document <id>; it loaded in <seconds> s.` Like the play button it first saves the scene's unsaved changes,
and the answer then says so. If the game is still loading when the call's 30
second wait runs out, `play` answers anyway, not as an error, saying `it is
still loading`; `scene_tree` and `screenshot` then show the game as it arrives.
A game that has not started by then answers `timeout`. A game that does not
start, a save before it that fails, or a `play` while the game is on answers
`play_failed` with the editor's message. `stop` stops the game, starting or
playing, and answers once the edited scene is back as it was before play;
edits made to the running game are gone. `save` and `play` need an open scene,
and `stop` a game starting or playing.

`send_input` sends `events` to the game playing in the viewport as the editor
window passes on a person's input, one window event per frame in order, and
answers once the game has had the last: `Sent <n> window events to the game in
document <id>.`, naming the keys still held in the game and those held in the
editor, which a later call should release. An event is
`{ "key", "pressed", "text"? }`, a key named as the window crate's `KeyCode`
names it (`KeyW`, `Space`, `ArrowUp`, `ShiftLeft`, `F1`; the logo keys, Super
or Windows or Command, are `MetaLeft` and `MetaRight`), ignoring case, with the
text a press types; `{ "button", "pressed", "at"? }`, one of `left`,
`right`, `middle`, `back` and `forward`, moving the pointer to `at` first;
`{ "move": [x, y] }`, which moves the pointer there and, as a mouse does, adds
the mouse motion that takes it there, a second window event; or
`{ "wheel": [x, y] }`, in lines. Points are pixels of the viewport's picture
from its top-left corner at its full size, the `viewport` size `status`
reports; on a screenshot scaled down by `max_edge`, multiply a point by the
full size over the PNG's size. A point off the picture, or a key name
`KeyCode` lacks, answers `invalid_arguments`, the latter with close names, and
sends nothing. The events then take the path real input takes: Ctrl+P stops
the game, and Shift+F1 or a press off the viewport's picture gives input to the
editor. A key left pressed stays held in the game, across calls, until a later
call releases it. `send_input` needs a game playing. While the editor has
input, a list that starts with a press `at` a point on the picture goes
through: that press gives the game input back, as a person's click on the
viewport does, without reaching the game as a click, and the rest of the list
follows. In any other list only key and button releases go through, to the
editor, so a key pressed before the editor took input is not left held; a list
that holds anything else answers `input_released` once its releases are sent.
After an event that gives the editor input, or stops the game, the same holds
for the rest of the list: its releases are sent, and if anything else was left
out the call answers `input_released` or `not_playing`, saying how many were
sent; a call whose events all went through says so in its answer.

### Errors

A failed call is a tool result marked `isError` whose text is
`{"code": …, "message": …}`. The message says what to do next.

| Code | Meaning |
| --- | --- |
| `invalid_arguments` | The arguments do not match the tool's input schema |
| `no_project`, `no_scene`, `not_playing` | The tool needs an open project, an open scene, or a game playing |
| `busy` | `open_project` while a project is opening, `save` of a scene still loading, or `screenshot` of a picture not drawn yet |
| `unknown_entity`, `stale_entity` | The id names no live entity of the open scene, such as the editor's camera, or of the game while it plays; or a slot that now holds a newer one |
| `unknown_component` | The entity does not carry the component, or for `add_component` no registered component has that name; the message lists those it could be |
| `ambiguous_component` | The short name names several components; the message lists their full paths |
| `unknown_path` | The component has no such property path; the message lists those `inspect` shows |
| `invalid_value` | The JSON does not decode to the property's type, whether an adapter edits the path or not; the message says why |
| `not_editable` | The adapter that edits the path offers no value edit |
| `unknown_asset` | The asset is not in the catalogue |
| `open_failed` | The project or asset did not open; the message is the editor's status |
| `unsaved_changes` | The call would discard unsaved edits; retry with `unsaved` |
| `save_failed` | `save`, or a save asked for by `unsaved: "save"`, failed; the message is the editor's status |
| `play_failed` | The game did not start; the message is the editor's status |
| `input_released` | `send_input` while the editor, not the game, has input, with a list that does not start with a click on the picture and holds more than key and button releases; the releases were sent |
| `refused` | The call cannot act in the current state, or the editor refused the edit; the message says why |
| `timeout` | What the call waited for did not happen within 30 seconds |
| `internal` | No system answered the call: a bug |

A tool system that panics takes the editor down, as any system does.

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
`is_changed()` and `Changed<T>` through `EditWorld<T>` mean "since this system
last read the target", in the target's own clock: a scene system's write, a
signal sent through `EditWorldSignals`, or a direct write by the host is seen
on the next run. The first run against a newly targeted world sees everything
stamped within `MAX_CHANGE_AGE` as added and changed.

An `Entity` of the edit `App`'s world is not an entity of the editor's. A field
in the editor's world that holds one is named for it: `scene_entity`,
`scene_target`, `scene_parent`.

The Asset Browser, the content browser, is a `UITree` over the editor world's
`AssetRegistry`, which opening a project fills by scanning the content headers
and which is empty while no project is open. The editor scans only then; content
that git or a file explorer changes while the project is open shows after it is
reopened. The tree mirrors the
project's `content/` folder: the folders and
assets directly inside it are the top-level rows, a folder shows a folder mark
and holds its subfolders and assets, and an asset shows its kind mark and name.
A folder appears once it holds an asset at any depth; empty directories are not
shown. In each folder, subfolders come first, then assets, each alphabetically
ignoring case, and folders start collapsed. When the registry changes, after an
import, a New or a save, the tree keeps the rows of folders and assets that still exist, so
expansion and scroll survive. Clicking a row selects it, folder or asset, but
only an asset becomes the catalogue's selection; activating a folder, by
double-clicking it or pressing Enter, expands or collapses it. A click while a
project is opening is ignored and the highlight returns to the selected asset. The search
field and the kind chips (`Scene`, `Mesh`, `Texture`, `StandardMaterial`)
hide every asset that does not match and show the
folders leading to each match, open; clearing them restores the expansion as it
was. The footer counts the matching assets. The navigation keys are the UI
crate's list and tree actions, as in the hierarchy.

**New** beside Import… lists one row, `New <kind>`, per creatable asset kind
(see [Custom asset editors](#custom-asset-editors)); Scene is the only one the
editor ships. It creates in the selected folder, or in the content root when
nothing or an asset is selected. Right-clicking a folder opens a menu with the
same rows, which create in that folder; right-clicking the tree's empty space
opens it for the content root. Right-clicking an asset opens no menu. Picking a
row clears the search and the kind
chip, and puts a row with a focused name field in that folder, ahead of its
assets, expanding the folders above it and scrolling to it. The field starts as a free name: `New
<kind>`, then `New <kind> 1`, `New <kind> 2`, and so on up to `New <kind> 999`;
when all of those are taken it shows `New <kind>`, which Enter refuses. Enter, or clicking
away, writes `<folder>/<name>.gasset` with a fresh `AssetId`, adds it to the
registry, selects it, scrolls to it, says `Created <address>` in the status line
and opens it in its editor; Escape cancels and writes
nothing. A name follows `valid_asset_name`, and one already used in that folder,
by a file on disk or a registry entry, ignoring case, is taken. Enter on a
refused name keeps the field, outlines it in the error colour and puts the
reason in the footer until the next keystroke; clicking away from a refused
name cancels. New and the menu rows are disabled while no project is open or one is opening. The
file is always a `.gasset`, even in a project whose `content.toml` sets another
extension; there the scan looks only for that extension, so the created asset
is not listed after the project is reopened. If an unsaved Scene tab holds the open, the asset is already written
and the unsaved-changes prompt decides when it opens.

Activating a supported asset in the Asset Browser, by double-clicking it or pressing
Enter on its row, opens its editor tab. Each asset type
has one editor: another Scene reuses the Scene tab. The old preview, camera,
selection, and edits remain until its replacement loads successfully.
A failed replacement preserves them. Opening the displayed asset cancels a
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

**Import…** beside the Asset Browser search opens a file picker for glTF, OBJ, BVH, and
image sources; several files can be picked at once. A dialog then lists each
file with the project-relative destination it will be copied to, which you can
edit before confirming. Enter confirms and Escape cancels. Files are imported
one at a time. Each import's assets are added to the registry in memory as it
finishes, without a rescan and without closing open tabs. No registry file is
written.

Importing over an existing destination replaces it and keeps its asset IDs, so
everything that refers to those assets stays wired. A file that is already
inside the project is imported where it is, not copied.

The files a source refers to come along with it: an OBJ's `.mtl` and the
textures it names, a glTF's `.bin` buffers and external images. Each keeps its
position relative to the source, and replaces a file already there. If one would
land outside the project, the import fails and nothing is copied; pick a
destination deeper inside the project. Chatter names the first failure of a
batch; the log has every one in full.

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
scene stays as it was. The game uses the OS clipboard, so copy and paste work
between it, the editor and other programs.

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
question about unsaved changes. Play is refused while that question is open.
A game that asks to quit is stopped.

Code learns how a Play ended from events in the editor's world. It ends in
`PlayStarted { document }` once the game plays, or in `PlayFailed { document,
message }`. Where the tab shows a status, `message` is that status: "Wait for
the scene to load", "No scene is open", the failed save's "Save failed: …", or
"Could not start the game: …". A Play the tab says nothing about fails too:
"The game is already playing", "Answer the question about unsaved changes
first", "Wait for the project to open", a Play on an entity that is not a
document ("No scene is open"), and "The game was stopped before it started"
when the game is stopped while it builds. Every stop of a starting or playing
game writes `PlayStopped { document }`, so a stop while the game builds writes
both `PlayFailed` and `PlayStopped`; a stop with no game on writes nothing.

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
leaves it alone and re-imports the source's other outputs. A save updates the
scene's entry in the registry in memory and writes no registry file. A failed
save reports why in Chatter and leaves the document dirty.

Each save, whether asked for with `SaveRequested` or made by Play, ends in an
event in the editor's world: `SceneSaved { document, asset }`, where `asset` is
the scene the document now shows, or `SaveFailed { document, message }`, where
`message` is the status the tab shows, "Stop to save" included. Ctrl+S asks for
a save only when the document has unsaved changes; otherwise it writes neither,
even while it shows "Stop to save".

Replacing the open scene, closing a dirty tab, switching project, and closing
the window all ask first when there are unsaved changes: Save, Discard, or
Cancel. Escape cancels. Discard marks the scene clean, so its replacement
loads; if the replacement then fails to load, the old scene stays on screen
with the discarded edits, marked clean.

The scene root shown in the hierarchy is the editor's wrapper around the scene,
not part of it, so the inspector shows nothing for it.

The scene hierarchy is a `UITree` with one item per listed entity, nested as the
scene is. It supports selection, filtering, expansion, and keyboard navigation.
The navigation keys are the UI crate's list and tree actions and work while
focus is on a row: Up and Down move the selection, Home and End jump to the
first and last row, Right expands a row or steps into it, Left collapses it or
steps out to its parent. While the search field holds text, every row whose
name does not contain it, ignoring case, is hidden, the path to each match stays
open, and clearing the field restores the expansion as it was. When an edit or
a click in the viewport selects an entity, the hierarchy expands its ancestors
and scrolls its row into view, also for an entity created that frame. The title
counts every listed entity, collapsed and hidden ones included, and the footer
reads "Loading…" while a scene loads.
Right-click a row to add a child, duplicate, rename, or delete it;
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
`RevealEntity`, which the hierarchy scrolls to; and one outcome per applied
edit signal: `EntityEdited { edit, result }`, whose result is the entity a
create or duplicate made; `PropertyCommitted { row, result }`, whose refusal
is a `PropertyRefusal`, `Undecodable` for a JSON value that does not decode and
`Refused` for any other reason; and
`ComponentEdited { edit, result }`. A refused edit's result is the text its
`SceneStatus` carries, so a refused property or component edit also shows in
Chatter. `SetValue { entity, component, path, value }` writes a JSON value, in
the shape `inspect` reports, into the field at `path` of a component, or into
the whole component when `path` is empty, with no inspector adapter involved,
and reports `PropertyCommitted` (its row has no editor) and `SceneEdited` as a
property commit does. Fields the JSON leaves out keep their values. Entity
references are live entity ids such as `42v3`, asset handles are asset ids and
load through the edit `App`'s `AssetServer` as a spawned scene's do, and colors
are their linear `[r, g, b, a]`. It refuses the scene root, an unknown path, a
value that does not decode, and an entity or component that is not there, and
a refused set leaves the component as it was. An outcome written by a signal reaches the editor a frame later, as
any forwarded event does. `HostedAppEvents<E>` is the
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
widgets, and validated edits. A registered editor can handle a whole struct or
an opaque type, and can replace any built-in editor, including the numeric
fields and the `Color` row. Snapshots are cached per component, with snapshots
owned by property-row entities. Component cards carry
`InspectedComponent` and can be accessed through ordinary ECS queries.
Unchanged components reuse their snapshots;
component changes, selection changes, and editor registration changes refresh
them. Widget input and focused edit buffers continue updating every frame.

Registration is split between the two `App`s. A component states what it
supports on its derive, `#[derive(Reflect)] #[reflect(Component, Default)]`,
and is registered in the scene's `App`, by the plugin that owns it or by the
recipe, with `App::register_type::<T>()`: `TransformPlugin` registers
`Transform`, and `RenderPlugin` registers `Light` and `Camera`. Registering a
type also registers the types its fields hold, so a component's
`AssetHandle<A>` fields need no call of their own. Registration fills the
world's `AppTypeRegistry`, which is part of the engine and needs no editor. `ReflectComponent` and `ReflectDefault` must be in scope where the
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

An editor chooses the layout of the row that holds its widgets by overriding
`PropertyEditor::row_node(&self, &Snapshot, &UITheme) -> UINode`. The default
returns `standard_row_node(theme)`, which an override can start from and adjust,
as the numeric editor does to wrap a color row's fields. `row_node` runs once,
when the row is spawned, and a refresh that keeps the row does not call it
again. An editor whose node depends on the snapshot must return `false` from
`follows_snapshot`, so a value change rebuilds the row.

A widget queues its edit on `PropertyCommits`. The inspector checks it against
the widget registry and sends it to the scene, where the tools write the value.
`PropertyEditor::apply` runs there, and its `EditContext` holds the scene
world's `AssetServer` and `AssetRegistry`, which `AssetManagerPlugin` provides.
A scene `App` without either refuses every edit with `EditError::MissingAssets`.

### Enums and options

A reflected enum is edited with nothing registered on the editor's `App`.
`#[derive(Reflect)]` on an enum registers its variant names as
`ReflectEnumInfo`, and the row shows a button naming the active variant that
opens a menu of all of them, the active one selected. Picking another switches
to it, built from defaults;
picking the active variant changes nothing and leaves the scene saved. An
`Option<T>` field is a checkbox instead: checking it sets `Some(T::default())`
and unchecking it sets `None`. The active variant's fields, or the value's when
an `Option` is `Some`, follow as rows of their own, so a switch that changes
them rebuilds the card. A row under a tuple field such as `Some`'s `0` takes
the label of the nearest named field above it, so `CameraEnvironment.ambient`
shows an "Ambient" checkbox followed by an "Ambient" value row. An editor
registered for the exact enum or `Option` type with `register_property_editor`
still takes precedence, and its fields are not shown.

### Colors

A `Color` field is edited with nothing registered on the editor's `App`. Its
row shows a swatch beside the label, and below them, across the card, four
labelled fields in the value's own space: `R G B A` for a linear or sRGB color,
`H S L A` for an HSL one, with hue shown and typed in degrees. Editing a field
keeps the space; the inspector never converts a color to another one. Linear red, green and blue accept any value
from 0 up, so HDR colors can be typed; sRGB channels, saturation, lightness
and alpha accept 0 to 1. Text outside those ranges is refused and the field
shows the value again. Hue accepts any number of degrees and wraps, so `450`
is stored as 90°. The swatch always shows the color at full alpha, and an HDR
color clips at full brightness. An editor registered for `Color` with
`register_property_editor::<Color, _>` still takes precedence.

### Asset handles

`AssetPicker` edits every `AssetHandle<A>` the scene's `AppTypeRegistry` holds,
with nothing registered on the editor's `App`. The handle's registration
carries `ReflectAssetHandle`, which names the asset kind and loads an id without
the editor knowing `A`. Registering the component that holds the handle is
enough, so a game's own material gets a picker once `MaterialPlugin::<M>::new()`
registers `MaterialComponent<M>`. An editor registered for the exact handle type
with `register_property_editor` still takes precedence.

The row names the asset the handle holds, "None" for an empty handle or
"Missing" for an id the registry does not know, and opens a menu of the
project's assets of that kind. Picking one writes a handle loaded through the
scene's `AssetServer`, so the viewport shows the new asset at once. The edit is
refused if the id is not an asset of that kind, or if the scene `App` never
registered the asset type with `register_asset`. `AssetPickerPlugin` keeps the
row's text current; `EditorPlugin` adds it. The picker's `edit_to` takes the
handle's id, so MCP's `set_property` on a handle field picks that asset through
the same checks.

An edit is the widget's own type, so a value alone cannot pass through it.
`PropertyEditor::edit_to(&value)` returns the edit that sets the field to
`value`; it still goes through `apply`, with its `EditContext`, so validation
holds. The built-in editors for `f32`, `f64`, `Vec3`, `Quat` and `bool` and the
asset picker implement it; `Quat` goes
through the widget's XYZ degrees, so it comes back normalized and equal up to
rounding. It defaults to `None`, which says the editor offers no value edit. A
custom editor opts in by implementing it; MCP's `set_property` writes a field
an adapter claims only through it, and answers `not_editable` for an adapter
that returns `None`:

```rust,ignore
fn edit_to(&self, value: &Setting) -> Option<SettingEdit> {
    Some(SettingEdit::Set {
        title: value.title.clone(),
        enabled: value.enabled,
    })
}
```

See the [complete custom widget example](examples/custom_property.rs). It
includes both registrations, a non-Clone domain type, click handling, snapshot
refresh, validation, and `edit_to`. Run its headless demonstration with:

```sh
cargo run -p concerto-editor --example custom_property
```

## Custom asset editors

Asset editors are separate from property editors. Register the asset with
`app.register_asset::<T>()`, then use `AssetEditorAppExt`:

```rust,ignore
app.register_asset::<Dialogue>();
app.register_asset_editor::<Dialogue, DialogueEditor>()?;
```

The editor type implements `Default`, which is how the registry builds it.
Register after `EditorPlugin` installs its registry. Dispatch uses the asset's
serialized kind (`Asset::name()`), which is what catalogue entries carry.
Duplicate editor registrations are rejected.

A type that has no source file can be made creatable instead. Implement
`CreatableAsset`, which lives in `concerto-foundation` so a game crate can
implement it, to give the value a new asset starts from, and register with
`register_creatable_asset_editor`:

```rust,ignore
impl CreatableAsset for Dialogue {
    fn new_asset() -> Self {
        Dialogue {
            text: "Hello".into(),
        }
    }
}

app.register_creatable_asset_editor::<Dialogue, DialogueEditor>()?;
```

`AssetEditorRegistry::creatable()` lists the creatable kinds in name order,
each with a `Creator`. `Creator::create(project_root, address, registry)`
writes a new `.gasset` at the address with a fresh `AssetId` and no import
provenance, adds it to the registry, and returns its id. Scenes are creatable,
and a new scene is empty. Registration is refused when `New <kind> 999`, the
longest name the Asset Browser prefills, is not a valid asset name. Each
creatable kind gets a row in the Asset Browser's New
dropdown and in its right-click menu.

`valid_asset_name` holds the name rules: 1 to 64 ASCII letters, digits, spaces,
`-` and `_`, with no leading or trailing space. The names Windows reserves,
`CON`, `PRN`, `AUX`, `NUL`, `COM1` to `COM9` and `LPT1` to `LPT9`, are refused
in any case.

Implement `AssetEditor::build` using its `EntityCommandQueue` to queue the
editor's components and children. The
editor entity carries `EditorDocument` and `EditorHosts`; custom systems use
ordinary `Query` and `ProjectState` access to load assets and update their
components. Content browser and Chatter remain shared. Tag additional owned
roots with `EditorOwned(document)` so closing the editor cleans them up
recursively. Headless hosts have no UI containers.

The [complete custom asset editor example](examples/custom_asset.rs) loads a
non-Clone Dialogue asset, builds its own text widget, replaces its contents,
rejects invalid input without mutation, hosts a preview of a cube that it sets
spinning with a signal, and registers through the public API. Run its headless
smoke test with:

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

### Previews

The Scene editor hosts an `App` for each open scene, with its own camera,
selection, and tools. Any other asset editor can host one too, a preview, by
returning a `PreviewSpec` from `AssetEditor::preview`, which is `None` unless
it says otherwise:

```rust,ignore
impl AssetEditor for DialogueEditor {
    fn preview(&self) -> Option<PreviewSpec> {
        Some(PreviewSpec { build: dialogue_preview })
    }
}

fn dialogue_preview(app: &mut App) {
    app.add_system(Update, spin_cubes.at_preview_time())
        .add_listener(set_spin.at_preview_time());
}
```

When a document of that kind opens, the editor begins a preview `App`,
`App::for_previewing()`, built from the recipe passed to `EditorPlugin`, then
`PreviewPlugin`, then `build`. It runs only the systems and listeners marked
`.at_edit_time()` or `.at_preview_time()`, so it plays animation but spawns no
startup scene and runs none of the game's unmarked systems. It is built
asynchronously, like a scene's, and gets the project's content and registry.
A build that panics sets the document's status to `Preview failed: <message>`,
and the preview is not built again until the tab is closed and reopened.
Closing the tab drops the preview. Opening another asset of the same kind
reuses the document and keeps its preview; reset it through signals when the
asset changes.

`PreviewPlugin` gives every preview an orbit camera around the origin
(left-drag orbits, the wheel zooms, middle-drag pans), the world grid when the
recipe has a renderer, one directional light, and the default
`CameraEnvironment`. Only its camera draws, through `ExclusiveCamera`, so a
camera the preview spawns, inside a character scene for instance, does not.
None of the scene tools are added.

The preview shows in the shared viewport, which stands in for the document's
`canvas` host: a previewing document gets `left` and `inspector` hosts and no
`canvas`. Build the editor's own controls in those two hosts; the hierarchy and
inspector panels are hidden on a preview tab, as on any custom editor's tab. A
preview carries only `ViewportTarget`, which the viewport shows, routes input
to, and steps, and never `EditTarget`, which `EditWorld<T>` and
`EditWorldSignals` read. A preview is stepped only while its tab is in front
and the viewport is shown; a preview in the background is frozen.

Drive the preview with the `PreviewSignals` system parameter:
`previews.trigger(document, signal)` queues the signal, and it is triggered in
the preview's world in `First`, before the preview's next step. A signal sent
before the preview is built waits for it; one for a closed document, a failed
preview, or a document with no preview is dropped. Read back through the
preview's events: call `forward_hosted_event::<E>()` before the editor `App`
finishes building, and read `EventReader<Hosted<E>>`, whose `document` says
which preview wrote it.
