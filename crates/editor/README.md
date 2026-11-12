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

### Extension crates

An editor crate extends the editor with a public `fn(&mut App)`, conventionally
`install`, which registers its asset editors and anything else it needs. A game
never depends on such a crate: it lists it under
`[package.metadata.concerto.editor]` in its `Cargo.toml`, and `cargo concerto
editor <dir> --update` regenerates `editor/` to call
`concerto_editor::run_with(recipe, project_root, plugins)`, as the project
tool's [README](../project/README.md#editor-plugins) describes. `run_with` is
`run` with `plugins` added to the editor's `App` in order, after `EditorPlugin`
and everything else the editor registers. Under `--mcp` they come after
`McpPlugin` and before the editor's own tools. `run` is `run_with` with an
empty list.

An extension brings its MCP tools with it: its plugin depends on
`concerto-mcp` and calls `add_tool` like the editor does, with no check for
`--mcp`.

```rust
pub fn install(app: &mut App) {
    app.add_tool::<GraphNodes, _>(
        McpTool::read_only("graph_nodes").needs(Needs::Project),
        graph_nodes,
    );
}
```

`add_tool` does nothing in an `App` without `McpPlugin`, so the tool and its
system exist only when the editor serves an agent. `describe_need` still
panics without `McpPlugin`; it is the editor's to call.

An extension crate's tests can use the `testing` feature, which exposes
`hosted::testing` and its `SceneFixture` and `forwarding`, the system that
re-writes a hosted event for a fixture's delivery, `inspector::value_systems`, the
systems that show a document's values and send their edits, and
`inspector::commit_number`, which queues a number as a float row's whole value;
take it as a dev-dependency:

```toml
[dev-dependencies]
concerto-editor = { workspace = true, features = ["testing"] }
```

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

The tools below are the editor's. An extension crate adds its own with
`add_tool` from its plugin, as [Extension crates](#extension-crates) shows, and
they are listed and called like these.

Frames run on demand, only while a call needs one or the editor is busy: a
project or scene opening, the assets of the edited scene or the preview in
front still loading, a game starting or playing, a save under way, a document
holding `BackgroundWork`, or text in the panels changed in the last frame and
may not be drawn yet. A timeline's
readout does not count, so a clip that is playing does not keep the editor busy
and advances only while frames run. An idle editor runs none, so once
`status` reports `busy: no` the viewport shows the scene with everything it
loads and the panels show what they hold. The tools that look answer between frames, without running one; the
tools that act run in frames, one at a time, and a second one waits until the
first has answered.

When the client disconnects, the editor quits and discards unsaved edits,
answering its own unsaved-changes prompt with Discard. An agent that wants its
edits keeps them by saving first, with `save`.

### Tools

`status` reports the project and its status line, the open documents, the
active one, the selection in the edited scene, the panels' sizes, the
viewport's size, whether the editor is busy, and under `values:` the names of
the front document's inspected values, or `none`. The `layout:` line reads
`left 260 · right 320 · bottom 220 · browser fill`: the size each panel shows,
in logical pixels, or `hidden` or `fill` for one the front tab collapses or
lets take the space that is left. The `viewport:` line is the picture's size
in physical pixels; under `--mcp` the scale is 1, so the two agree. `list_assets` lists the project's assets as
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
`set_property` writes it whole (below). A list has `adapter` false for the same
reason, and each element follows it, by index from 0: `values.2`, or for an
element with fields only those fields, `strides.0.length`. A component without
reflection data has `"value": null, "unavailable": true` and no paths. A list
longer than 16 elements shows its first 16 in `value` and ends with a
`"… N more, M in all"` string, so a cut list is not a value to write back.
`paths` shows the same 16 elements and then, among its entries, a
`"… N more of values, M in all"` string naming the list; a cut element is still
addressed by its index. Ids are scene entities written as
`42v3`. `screenshot` answers a PNG: with `target` `viewport`, the picture the
viewport draws, the edited scene or the game while it plays, followed by the
camera's pose while editing; with `editor`, the whole editor, panels included,
at its window size. A picture longer than `max_edge` pixels (1024) on either
side is scaled down to it, keeping its shape; a smaller one keeps its size. It
is the last frame drawn: the viewport's picture changes only when the edit
`App` steps. `scene_tree`, `find_entities`, `inspect` of an entity and
`screenshot`'s viewport need an open scene. These six look.

`open_project` opens `path` and answers once the catalogue is read; the
project's default or first scene then opens, as it does when the editor opens
a project.
`open_asset` opens `asset`, an address or an id from `list_assets`, in its
editor: it answers once the asset has loaded, and at once if it is already
shown. A scene that fails to load answers `open_failed`; another kind answers
once its editor calls `finish_asset_request`, or `open_failed` with the
document's status if the load failed. A kind with no editor answers
`open_failed`. `create_asset` creates an empty asset of `kind`, one of the kinds
the Asset Browser's New lists, ignoring case, named `name` in `folder`, or in
the content root without one, by triggering the `CreateAsset` signal that New's
name field triggers. It answers `Created <address> · <kind> · <id>. Open it
with open_asset.` and does not open the asset. A kind that cannot be created
answers `unknown_kind`, listing the ones that can; a name New would refuse, a
name taken in that folder, or a folder outside the content root answers
`create_failed` with the reason. It needs an open project. `import_source`
imports the file at `source` through the queue the Import… dialog fills: it
copies the source to `destination` in the project, or to `assets/<file name>`
without one, or leaves it where it is when it already lies inside the project,
and answers once that import has finished, with `Imported <destination>: N
assets, M sibling files.` and one `address · kind · id` line per asset written.
A path with no file answers `unknown_source`; a destination outside the
project, without a file name, with another extension than the source's, or a
source no importer handles answers `import_refused` with the dialog's reason;
an import that fails answers `import_failed` with the status line's reason.
Each queued import ends with an `ImportFinished` event, which is what the tool
waits for. It needs an open project. `close_document`
closes the document `status` lists as
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

`set_panel_size` sets the size of `panel`, one of `left`, `right`, `bottom`
and `browser`, to `size` logical pixels: a width, or for `bottom` the strip's
height. It triggers `ResizePanel`, the signal a splitter's drag triggers, and
answers `status`'s `layout:` and `viewport:` lines once the panels are laid
out and the viewport has its new size, or the `layout:` line alone on a tab
that shows no viewport. A size outside what the window allows is clamped, and
the answer shows the size that was set. A panel that is `hidden` or `fill` on
the front tab answers `refused`, naming the kind of tab that sizes it.

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
of the field's kind answers `refused`. A `SceneEntityRef` field is edited by the
entity picker, which refuses a node index other than the unset value; one inside
an `Option` or a list is written whole with the rest of it, unchecked (#377).
A path `inspect` marks `enum` takes the whole enum, its variant's name for a
variant without fields (`"Point"`) or an object with the variant's name as its
one key (`{"Spot": {"cone_angle": 40}}`), so it can switch variants; a field
left out of the new variant takes its default, and an unknown variant answers
`invalid_value`, naming the variants. A path that names a struct above a property is
not one `set_property` takes, so an adapter's validation holds for the fields
it edits, and that includes a list element with fields.
A list's own path takes the whole array, and an element's path takes that
element. Under a list, a property whose type has no JSON form and no registered
`Default` answers `not_editable`, naming the type; a nested list and an `Option`
are such types, and their own elements and fields can still be written.
`list_push` appends a default element to the list at `path`, and `list_remove`
removes the element at `index`, as the inspector's add and remove buttons do.
Each answers the list's new length: `Added an element to Samples.values; its
length is 4.` A path that is not a list, or a `Vec` with an editor registered
for its exact type, answers `not_a_list`, and an index past the end answers
`index_out_of_range`, naming the length.
A component that `inspect` lists as one value, a marker or an opaque type, has
the path `""`. The scene root is the editor's wrapper and none of these change
it. All nine need an open scene, unless a call names a `target`.

`inspect`, `set_property`, `list_push` and `list_remove` also reach a document's
values, the ones its asset editor publishes through `InspectedValues` (see
[Document values](#document-values)). `target` names a value of the front
document, one of the names `status` lists under `values:`, and takes the place
of `entity` and `component`, or for `inspect` of `entity` and `components`. A
name matches exactly, unlike a component's. These calls need no open scene and
answer straight away, with no scene as the edit target. `target` with any of
those arguments, neither `target` nor `entity`, and `entity` without a
`component` the tool requires each answer `invalid_arguments`, a name the front
document does not have answers `unknown_target`, listing the names it has, and a
name two of its values share answers `ambiguous_target`.

`inspect` with `target` answers in a component's form with no entity: `name`,
`type`, `value` and `paths`. A value that is gone, such as one its document has
not loaded, answers `{name, value: null, unavailable: true}`. This is the one
read that uses commands, because its answer needs the whole world and a
read-only tool's system must only read: its system is added with `add_system`
to `McpServe` and `McpRead`, after `receive_read_only_calls`, not with
`add_tool`.
`set_property` takes a value as it takes a component: a path with an adapter
goes through it, any other path is written from the JSON, and the answer is
`inspect`'s form. A JSON write refuses a value whose root type is not registered
in the registry its target names. `list_push` and `list_remove` answer the new
length. An edit reports `Hosted<PropertyCommitted>`, as one from the inspector
does, and what it means to the document is up to its asset editor.

`save` writes `document`, or the active document without one, to its file as
Ctrl+S does, and answers `Saved <address>.` once it is written. A document with
no unsaved changes answers `Nothing to save in document <id> <address>.`, which
is not an error. A save that fails answers `save_failed` with the editor's
message, as does a save while the game plays (`Stop to save`), and a scene still
loading answers `busy`. `play` starts the active scene's game in the viewport,
as the play button does, and answers once the game has built, plays, has
spawned every scene it loaded, and has no asset loads in flight: `Playing
<address> in document <id>; it loaded in <seconds> s.` The time runs from
`PlayStarted`, which comes after the play `App` has warmed up hidden, so it
covers only what the game still loads once it is shown. Like the play button it first saves the scene's unsaved changes,
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
editor, which a later call should release, then the pads connected, those
`send_input` connected marked `from send_input`, and the pad buttons held. An
event is
`{ "key", "pressed", "text"? }`, a key named as the window crate's `KeyCode`
names it (`KeyW`, `Space`, `ArrowUp`, `ShiftLeft`, `F1`; the logo keys, Super
or Windows or Command, are `MetaLeft` and `MetaRight`), ignoring case, with the
text a press types; `{ "button", "pressed", "at"? }`, one of `left`,
`right`, `middle`, `back` and `forward`, moving the pointer to `at` first;
`{ "move": [x, y] }`, which moves the pointer there and, as a mouse does, adds
the mouse motion that takes it there, a second window event; or
`{ "wheel": [x, y] }`, in lines; or a gamepad event: `{ "pad": 0, "connect":
true, "name"? }` plugs pad 0 in, named `Pad 0` when `name` is left out, and
`"connect": false` unplugs it; `{ "pad": 0, "button": "south", "value": 1.0 }`
sets a button, 0 up to 1 fully down, the triggers in between; and
`{ "pad": 0, "axis": "left_stick_x", "value": 0.5 }` sets a stick axis, -1 to 1
with Y up. Buttons and axes are named in snake case, ignoring case: `south`,
`east`, `north`, `west`, `left_bumper`, `right_bumper`, `left_trigger`,
`right_trigger`, `select`, `start`, `mode`, `left_thumb`, `right_thumb`,
`d_pad_up`, `d_pad_down`, `d_pad_left`, `d_pad_right`, and `left_stick_x`,
`left_stick_y`, `right_stick_x`, `right_stick_y`. A pad's buttons and axes need
it connected first, by `send_input` or as a real gamepad; the editor's window
sees the pads `send_input` connects as it sees real ones, and they stay
connected, across calls, until a later call unplugs them. Points are pixels
of the viewport's picture from its top-left corner at its full size, the
`viewport` size `status` reports; on a screenshot scaled down by `max_edge`, multiply a point by the
full size over the PNG's size. These answer `invalid_arguments` and send
nothing: a point off the picture; a key name `KeyCode` lacks, or an unknown pad
button or axis, with close names; a button, axis or `"connect": false` for a
pad not connected, saying to send `{ "pad": n, "connect": true }` first;
`"connect": false` for a real gamepad, which is unplugged by hand; `"connect": true` for a
number a real gamepad has, listing the numbers in use, so pick another (a pad
`send_input` already connected is left as it is); and a pad event with none, or
more than one, of `connect`, `button` and `axis`, or a button or axis without
`value`, showing the three shapes. The events then take the path real input
takes: Ctrl+P stops the game, and Shift+F1 or a press off the viewport's picture gives input to the
editor. A key left pressed stays held in the game, across calls, until a later
call releases it. `send_input` needs a game playing. While the editor has
input, a list that starts with a press `at` a point on the picture goes
through: that press gives the game input back, as a person's click on the
viewport does, without reaching the game as a click, and the rest of the list
follows. Pad events go through whoever has input, to the game. In any other
list only key and button releases and pad events go through, the releases to
the editor, so a key pressed before the editor took input is not left held; a
list that holds anything else answers `input_released` once those are sent.
After an event that gives the editor input, or stops the game, the same holds
for the rest of the list: its releases are sent, and if anything else was left
out the call answers `input_released` or `not_playing`, saying how many were
sent; a call whose events all went through says so in its answer.

### Errors

A failed call is a tool result marked `isError` whose text is
`{"code": …, "message": …}`. The message says what to do next.

| Code | Meaning |
| --- | --- |
| `invalid_arguments` | The arguments do not match the tool's input schema, or `target`, `entity`, `component` and `components` are given in a combination the tool does not take |
| `no_project`, `no_scene`, `not_playing` | The tool needs an open project, an open scene, or a game playing |
| `busy` | `open_project` while a project is opening, `save` of a scene still loading, or `screenshot` of a picture not drawn yet |
| `unknown_entity`, `stale_entity` | The id names no live entity of the open scene, such as the editor's camera, or of the game while it plays; or a slot that now holds a newer one |
| `unknown_component` | The entity does not carry the component, or for `add_component` no registered component has that name; the message lists those it could be |
| `ambiguous_component` | The short name names several components; the message lists their full paths |
| `unknown_target` | The front document has no value of that name; the message lists those it has |
| `ambiguous_target` | The front document has more than one value of that name |
| `unknown_path` | The component or value has no such property path; the message lists those `inspect` shows |
| `invalid_value` | The JSON does not decode to the property's type, whether an adapter edits the path or not; the message says why |
| `not_editable` | The adapter that edits the path offers no value edit, or the path is under a list and its type has no JSON form and no registered `Default` |
| `not_a_list` | `list_push` or `list_remove` on a path that is not a list, or on a `Vec` with an editor registered for its exact type |
| `index_out_of_range` | `list_remove` with an index past the end; the message names the length |
| `unknown_asset` | The asset is not in the catalogue |
| `open_failed` | The project or asset did not open; the message is the editor's status |
| `unsaved_changes` | The call would discard unsaved edits; retry with `unsaved` |
| `save_failed` | `save`, or a save asked for by `unsaved: "save"`, failed; the message is the editor's status |
| `play_failed` | The game did not start; the message is the editor's status |
| `input_released` | `send_input` while the editor, not the game, has input, with a list that does not start with a click on the picture and holds more than key and button releases and pad events; the releases and pad events, which always go through, were sent |
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
  both ways. A `PendingApp` is building while the editor polls its plugins'
  `ready` each `First`, then finishing: `finish_plugin_build` and the ready
  step run on a `hosted-build` thread, and the editor joins it in a later
  `First` and hands the `App` the current asset registry. Dropping the
  `PendingApp`, however it is removed, detaches the thread and sets a flag the
  thread reads after `finish_plugin_build`: it then cancels the `App`'s loads,
  skips the ready step, and drops the `App`.
  An edit or play `App` then warms up: the editor steps it hidden once per
  `First`, whatever tab is in front and with no input, until a step leaves its
  `AssetServer` not loading, no `SceneSpawnerComponent` whose scene is in its
  `AssetStore<Scene>`, and `render_assets_settled` true, or until
  `WARM_UP_LIMIT`, 10 s after the join. A `WarmUpLimit` resource in the
  editor's world, when present, replaces `WARM_UP_LIMIT` for the edit and play
  `App`s it starts.
  Only then does it become the document's `HostedApp`; a panic in a hidden
  step is the build's failure, in the step `warming up`. A preview `App` skips
  the warm-up. `finish_blocking` and `PendingApp::finish_now` finish on the
  calling thread and skip it too.
  Finishing on the thread and the warm-up happen only when `register_editor`
  inserts `FinishBuildsOffThread`. Without it, as in tests that build the
  editor's world themselves, `First` finishes a ready `App` on the editor's
  thread and makes it the `HostedApp` at once, with no warm-up.
  Every hosted `App` uploads at most `HOSTED_UPLOAD_BYTES_PER_FRAME`, 32 MiB,
  of render assets per frame, set after the recipe and again after
  `finish_plugin_build`, so a game's own `RenderAssetBytesPerFrame` does not
  apply in the editor.
- `src/tools/` is everything that runs inside an edit `App`: the editor camera,
  the grid, picking, the selection box, the gizmo, and the listeners that apply
  each edit. `EditToolsPlugin` installs it. The half that edits and captures a
  scene needs no renderer; the viewport half is added when the recipe has one.
- The panels (hierarchy, inspector, viewport, tabs, content, dock) run in the
  editor's `App`.

The dock (`src/dock.rs`) tiles the window; nothing is drawn over anything
else. From the top: a bar with the brand, the tabs, the stats and the window
buttons; a row of the `Left`, `Centre` and `Right` regions; the `Bottom` strip
across the full width; and the `Status` line. The hierarchy is in `Left`, the
viewport in `Centre`, the inspector in `Right`, the Asset Browser in `Bottom`
and Chatter in `Status`. The viewport, and so the game while it plays, is
exactly the space the panels leave. A panel declares its region in its
`PanelDescriptor`, and `PanelRegistry::region` gives a region's node.

Each panel in `Left`, `Right` and `Bottom` is a card, `dock::card`: the
theme's panel with its large corner radius and a faint outline, on the opaque
canvas backdrop. The viewport's picture has the same corners. The cards stand
8 px apart and 8 px from the window's left and right edges; the top bar and
the status line have no fill of their own. A tab's hosts are cards too, and
two cards in one side region stand 8 px apart. The Play button on the Scene
tab is a pill filled with the accent that shows `play` and "Play"; while the
game runs it is outlined in the error colour and shows `stop` and "Stop".

Four splitters resize the panels: between `Left` and `Centre`, between
`Centre` and `Right`, above the `Bottom` strip, and, on a tab with a bottom
host, between the Asset Browser and that host. A splitter is the gap between
two cards: it draws nothing at rest, and a line in the accent colour while it
is hovered or dragged. The sizes are the
`EditorLayout` resource, in logical pixels: `left` 260, `right` 320, `bottom`
220 and `browser` 420 to begin with. They are the same on every tab and last
for the session; no file holds them. `ResizePanel { panel, size }`, with
`panel` a `LayoutPanel` (`Left`, `Right`, `Bottom` or `Browser`), is the one
signal that changes them; a splitter's drag triggers it, and so does the MCP
tool `set_panel_size`. Its listener
clamps the size for the window as it is: `Left` no less than 180 and `Right`
no less than 220, each no more than leaves the `Centre` 320 wide; `Bottom`
from 120 to what leaves the `Centre` 240 high; `Browser` from 200 to what
leaves its bottom host 240 wide. In a window too small for the stored sizes
the dock shows smaller ones, `Right` giving way before `Left`, and the stored
sizes come back when the window grows; `EditorLayout::shown` gives the sizes
on screen.

`PanelStates` says what each of the four is on the front tab: `Sized`, which
shows its stored size and can be resized; `Hidden`, collapsed with its
splitter; or `Fill`, taking the space that is left, with no splitter. The
workspace (`src/workspace.rs`) decides it from what the tab shows: a side
region with no visible panel and no host is `Hidden`, and the Asset Browser is
`Fill` unless the tab has a bottom host. On a [form](#inspected-assets) tab
`Left` is `Hidden`, the `Centre` is hidden and `Right` is `Fill`.
`ResizePanel` for a panel that is not `Sized` changes nothing.

The brand in the top bar is the Concerto mark, `fonts::brand_mark`: the one
glyph of the bundled `Concerto Mark` face, which
[`docs/logo/build.py`](../../docs/logo/README.md) generates with the logo's
SVGs.

Every icon is a glyph of the bundled Phosphor face, drawn through
`fonts::icon` from a constant in `fonts::glyph`, which carries the glyph's
upstream name. There are two sizes. A row's icon is a 16 px glyph in a 20 px
column: a tree row's is `fonts::row_icon`, and a tab builds its own to the same
measure. An icon button is `fonts::icon_button` with `fonts::button_icon`: a
16 px glyph in a square of the theme's control height. A tree row's icon is the
muted text colour at rest and the text colour while its row is hovered or
selected, and a tab's is the text colour on the active tab and muted on the
others, whether hovered or not; the glyph, not a colour, tells kinds apart. The hierarchy shows `stack` for the scene root, `camera`,
`lightbulb` and `cube` for an entity with a camera, a light or a mesh, and
`circle` for any other entity, with or without children.

The `AssetIcons` resource maps an asset kind's `Asset::name()` to its glyph,
and the Asset Browser's rows and the tab strip both read it. `ContentPlugin`,
`TabsPlugin` and the first `register_asset_icon` each insert it when it is
missing, so either plugin works without `FontsPlugin`. `FontsPlugin` registers
`Scene`, `Mesh`, `Texture`, `StandardMaterial` and `Skeleton`; a kind nothing
registered shows `file`. An extension crate registers its own kinds, and a
later registration of a kind replaces the earlier one:

```rust,ignore
use concerto_editor::fonts::{AssetIconsAppExt, glyph};

app.register_asset_icon(AnimationClip::name(), glyph::WAVEFORM);
```

`concerto-animation-editor` registers `AnimationClip`, `AnimationGraph`,
`BoneMask` and `RetargetRig`, and `concerto-motion-matching-editor` registers
`MotionDatabase`.

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
`finish`, the `Startup` schedule, the ready step that publishes the content
and opens the scene, and every hidden step while it warms up. The half-built
`App` is dropped once every system it was running has returned, the panic is
logged, and the Scene tab shows "Could not open scene: <panic message>", as a
failed load does.

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
assets directly inside it are the top-level rows, a folder shows the `folder`
glyph, `folder-open` while expanded, and holds its subfolders and assets, and
an asset shows its kind's glyph from `AssetIcons` and its name.
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
(see [Custom asset editors](#custom-asset-editors) and
[Inspected assets](#inspected-assets)); Scene is the only one the editor
ships. The rows follow the registry, so the game's inspected kinds join them
once a scene's edit `App` has built. It creates in the selected folder, or in the content root when
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
name cancels. Enter and clicking away trigger `CreateAsset { kind, folder, name, open }`,
the signal an extension or the `create_asset` MCP tool triggers to create an
asset without the row; its listener does the checks and the write, makes a
missing folder, and reports each request as an `AssetCreation` event with the
new id or the reason. New and the menu rows are disabled while no project is open or one is opening. The
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

Opening a project opens a scene in the Scene tab: the one `content.toml`'s
`default_scene` names, or else the project's first scene, by folder and then
name. A `default_scene` that is missing or is not a Scene logs a warning and
the first scene opens instead; a project with no scene opens nothing. A scene
that fails to open keeps its tab with the failure as its status, and no other
scene is tried.

With no tabs open, the editor shows an empty viewport. Switching projects
closes all editors. There is only one Scene tab, from the scene that opens with
the project until the project changes or the editor quits; it has a Play button where other
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
position relative to the source, and replaces a file already there. A texture
the `.mtl` names that cannot be read is skipped with a warning naming it in the
log, and the import goes on without it; a missing `.mtl`, `.bin` or glTF image
fails the import. If one that can be read would land outside the project, the
import fails and nothing is copied; pick a destination deeper inside the
project. Chatter names the first failure of a
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
then lands on whatever is under the pointer, the tab strip, the panels and the
splitters beside the viewport included; Shift+F1; and the editor window losing focus. The
game is told it lost focus, so nothing stays held in it, and any cursor grab it
asked for is let go. The border turns muted and the viewport says "Click to
play"; the game keeps running but gets no input, and the editor works as usual.
A click on the viewport's picture gives input back to the game without reaching
it as a click. Regaining window focus does not.

Ctrl+P stops the game whichever side has input, and Shift+F1 works while the
game has it; the host reads both from the window's own events while the game
has input. `hosted::input::ReleaseInput(document)` gives input to the editor
from code.

Gamepads are the exception: their events reach the playing game whichever side
has input, and the editor's panels read none. Giving input to the editor tells
the game it lost focus and then sends it the buttons and sticks held now, so a
held stick keeps driving it. The editor window losing focus zeroes the game's
pads whichever side has input. When the game starts playing, or its viewport
is shown again, it is first told about the pads connected now, with their
buttons and sticks, and about any that went away while it was hidden.

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

Every write of a content asset in the editor also ends in
`AssetsWritten { ids }` in the editor's world, naming the assets written: a
finished import sends every asset it wrote, unless its project was replaced
while it ran; a save by any asset editor sends the saved asset, taken from its
`SceneSaved`, so an asset editor's save reports it by writing `SceneSaved`; and
a New in the Asset Browser sends the created asset. Code that keeps data
derived from other assets reads it to know when to derive again.

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
create or duplicate made; `PropertyCommitted { address, result }`, whose refusal
is a `PropertyRefusal`, `Undecodable` for a JSON value that does not decode and
`Refused` for any other reason; and
`ComponentEdited { edit, result }`. A refused edit's result is the text its
`SceneStatus` carries, so a refused property or component edit also shows in
Chatter. `SetValue { entity, component, path, value }` writes a JSON value, in
the shape `inspect` reports, into the field at `path` of a component, or into
the whole component when `path` is empty, with no inspector adapter involved,
and reports `PropertyCommitted` and `SceneEdited` as a
property commit does. Fields the JSON leaves out keep their values. Entity
references are live entity ids such as `42v3`, asset handles are asset ids and
load through the edit `App`'s `AssetServer` as a spawned scene's do, and colors
are their linear `[r, g, b, a]`. It refuses the scene root, an unknown path, a
value that does not decode, and an entity or component that is not there, and
a refused set leaves the component as it was. An outcome written by a signal reaches the editor a frame later, as
any forwarded event does. An edit of a document's value does not pass through a
hosted `App`: its `Hosted<PropertyCommitted>` is written straight into the
editor's world, naming the value's document, with no frame's lag.
`HostedAppEvents<E>` is the
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

The inspector edits live reflected values, a scene's components and the
[values a document holds](#document-values), through typed `PropertyEditor<T>`
adapters. `concerto-reflect` provides structural access; adapters own snapshots,
widgets, and validated edits. A registered editor can handle a whole struct or
an opaque type, and can replace any built-in editor, including the numeric
fields and the `Color` row. Snapshots are cached per component, with snapshots
owned by property-row entities. Component cards carry
`InspectedComponent` and can be accessed through ordinary ECS queries.
Unchanged components reuse their snapshots;
component changes, selection changes, and editor registration changes refresh
them. Widget input and focused edit buffers continue updating every frame.

The inspector's cards sit in a column at most 760 px wide, centred in the
panel, so a panel wider than that, as on a form tab, leaves margins beside
them. A card's label column is 38% of the card's width, no less than 96 px and
no more than 260 px, so a wider panel gives labels more room.

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
the widget registry and sends it to the world the value lives in. A component's
edit goes to the scene, where the tools write the value.
`PropertyEditor::apply` runs there, and its `EditContext` holds the scene
world's `AssetServer` and `AssetRegistry`, which `AssetManagerPlugin` provides.
A scene `App` without either refuses every edit with `EditError::MissingAssets`.
A document's value is edited in the editor's world, as the next section says.

### Document values

The inspector shows and edits anything that implements `PropertyTarget`: a
reflected value with a name, a `TargetWorld` saying where it lives, and ways to
read and edit it. Entity cards use `ComponentTarget`. An asset editor shows a
value its document holds with `DocumentValue<C>`, which reaches into `C`, a
component on the document in the editor's world, and lists its targets in the
`InspectedValues` component on the document:

```rust,ignore
fn build(&self, editor: &mut EntityCommandQueue) {
    let document = editor.entity();
    editor.insert((
        PaletteDocument { palette: None },
        InspectedValues(vec![Arc::new(DocumentValue::<PaletteDocument> {
            name: "Palette".into(),
            document,
            read: Box::new(|held| held.palette.as_ref().map(|p| p as &dyn Reflect)),
            edit: Box::new(|held| held.palette.as_mut().map(|p| p as &mut dyn Reflect)),
        })]),
    ));
}
```

While that document is the active one, the panel shows one card per target,
titled by its name, with no name field, add-component row or ⋯ menu, since those
belong to entity cards. `read` or `edit` returning `None` means the value is
gone: its card has no rows and an edit is refused with
`EditError::MissingTarget`. The workspace shows the inspector with the document's
`inspector` host above it, and the host neither grows nor shrinks while
`InspectedValues` is there, so the two do not split the card in half and what
the host holds is not clipped. `InspectedValues` is replaced, never removed,
because a removal is not noticed; an empty list shows no cards.

A document that publishes several values can show one at a time by adding a
`ShownValue` holding one of its targets: the panel then shows only that value's
card, and inserting another `ShownValue` swaps it. Every value in
`InspectedValues` stays reachable by name over MCP. `ShownValue` is replaced,
never removed, for the same reason.

A value's types are the ones registered in the document's preview `App` when it
has one, because a game's types are registered there, and otherwise in the
editor's `App`. `HostedApp::type_registry` gives the preview's. Register the
value's type with `register_type` in the `App`s it is read through: what the
registry does not hold reads "Unsupported type", and MCP refuses to write JSON
into a value whose root type is missing.

A value has no change detection and is not polled. It is read when an edit lands,
when `ActiveEditor`, `InspectedValues`, `ShownValue` or the editor registrations
change, when a preview arrives, and on the public signal `RefreshInspector`.
Anything else that writes the value, such as a load replacing it, triggers
`RefreshInspector` itself; the cost of forgetting is a panel that stays stale
until the next edit.
Targets that are the same `Arc`s keep their cards and their rows, so a focused
field keeps its text. Different targets rebuild the cards and reset the scroll.

A value's edit runs in the editor's world, and its `EditContext` holds the
editor's `AssetServer` and `AssetRegistry`; without either every edit is refused
with `EditError::MissingAssets`. That is a limit on handles: the editor's server
knows only what its own `App` registered, which is `RenderPlugin`'s `Mesh`,
`Texture` and `Skeleton` and whatever an extension adds with `register_asset`.
So a value holding another handle type cannot take a picker edit, even when its
types come from a preview that knows the asset. A commit queued before the
editor registrations changed is refused and reported, where a component's is
only logged. The outcome is one `Hosted<PropertyCommitted>`, naming the value's
document and not the active one,
so a tab switch cannot credit an edit to the wrong tab. A refusal also sets that
document's `EditorDocument::status`, which only a scene's chrome shows by itself,
so an asset editor puts it in one of its own panels. Nothing else happens by
itself: an asset editor reads the event to insert `Unsaved` and to tell its
preview.

`PropertyTarget::saved` says whether an edit to the value changes what a save
of its document writes; it defaults to `true`. A value that only steers what a
tab shows, such as a preview setting, returns `false`: its edits still report
`Hosted<PropertyCommitted>`, but do not mark an inspected asset's tab unsaved.

A panel that writes a value by path, rather than through an inspector row,
triggers the public signal `WriteValue { target, path, json }`. Its listener
runs the code behind MCP `set_property` on a value, with `path` as `inspect`
shows it and `json` as `set_property` takes it, so the panel and MCP write
through one code path. The outcome is reported as `Hosted<PropertyCommitted>`,
and a refusal, an unknown path included, sets the document's
`EditorDocument::status`.

A custom editor on a value row must not use `EditWorld`, which skips its system
while no scene is the edit target. It reaches its document through
`Res<ActiveEditor>`, because value rows exist only for the front document, and
closes its menus on a change of `ActiveEditor`, because `Remove<EditTarget>`
does not fire between value tabs.

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
the label of the nearest named field above it. `CameraEnvironment.fog` shows a
"Fog" checkbox followed by rows for its fields, Color, Density and Start. An
editor registered for the exact enum or `Option` type with
`register_property_editor` still takes precedence, and its fields are not
shown.

### Lists

A `Vec<T>` field is edited with nothing registered on the editor's `App`.
`Vec<T>` registers `ReflectList`, and the field's row shows its length and an
add button, which appends `T::default()`. Each element follows, labelled by its
index from 0, in one of three shapes. An element with an editor gets that
editor's row, with a remove button at the end. An element with fields, such as a
struct, gets a heading with its index and the remove button, followed by a row
per field. Anything else gets an "Unsupported type" row with a remove button, so
whatever add creates can be removed; `Vec<u64>` falls here, since there is no
`u64` editor. Elements cannot be reordered.

Add and remove are property commits, so they mark the scene unsaved. The editor
has no undo, so a removal stands until the scene is reloaded. A list in a list,
or in an `Option`, works the same way. Every element is shown, and a list the
game rewrites stays visible: a change of length rebuilds the card, and new
values refresh in their rows. An editor registered for the exact `Vec` type with
`register_property_editor` still takes precedence, and its elements are not
shown.

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

### Names

A `concerto_name::Name` field is a text field holding its text, with nothing
registered on the editor's `App`. Submitting the field commits the typed text
as a `Name`; Escape or moving focus away puts the shown text back without an
edit. This is the interned name type, not the entity `Name` component, which
the field above the cards edits.

### Text and numbers

A `String` field is a text field that behaves like a `Name` field: submitting
commits the typed text, and Escape or moving focus away puts the shown text
back without an edit. `f32` and `f64` fields are one number field each, `Vec2`
two and `Vec3` three, each edited on its own. `usize` and `u32` fields take a
whole number from 0 up, and `i32` fields a whole number of either sign. A
fraction, a negative count, or a number the field's type cannot hold is
refused, and the field shows the value again. Nothing is registered on the
editor's `App` for any of them.

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

### Joint names

`JointNameEditor` edits every `concerto_mesh::JointName` field, on a field and
in a list, with nothing registered on the editor's `App`:
`InspectorRegistry::default()` registers it and `InspectorPlugin` adds its
systems. The row is a button naming the joint, which opens a menu of the bones
in the front document's `DocumentBones`, so it needs no edit target. Without
bones the button is disabled and reads the stored name in the muted colour. A
name the skeleton lacks uses the warning colour, an empty name reads `None`,
and the menu closes when another tab comes to the front. Picking the name
already set makes no edit. The picker's `edit_to` takes the value.

`DocumentBones(Option<Vec<Name>>)` is a component any document may carry: the
bones of the skeleton its value names, or `None` while it names none that can
be read. An asset type names its skeleton by implementing
`concerto_mesh::SkeletonSource` and writing `#[reflect(SkeletonSource)]`; the
editor reads the id through `ReflectSkeletonSource` without naming the type.
The generic tab of an [inspected asset](#inspected-assets) fills its
`DocumentBones`. An editor of its own fills it however it reads the skeleton,
as the motion database editor does.

The menu is `name_menu`'s `NameMenu`, opened under a button with
`open_name_menu`, or at a point in the window with `open_name_menu_at`, as a
right-click menu is, and closed with `close_menu`; `close_name_menu` closes it
when another tab comes to the front. An extension crate opens it for names of its own, as the motion database
editor's channel-type picker does.

### Entity references

`EntityPicker` edits every `SceneEntityRef` field, which is how a component
names another entity of its scene, as `CharacterLink` names a capsule and a
skeleton. The row names the referenced entity: its `Name`, or `Entity 42` as the
hierarchy writes an entity with no name or an empty one. It reads
`(not in this scene)` when the value is unset or names an entity the menu below
doesn't offer, the scene root included. The button beside it selects that
entity and reveals it in the hierarchy.

The row opens a menu with a filter field, `None`, and the entities the
hierarchy lists for the edit target, indented by depth, without the scene root:
a scene saves only its root's children, so a reference to the root couldn't be
saved. Typing keeps the entities whose label contains the text, and their
ancestors. `None` writes the unset value. A field whose entity was deleted keeps
naming it and makes saving fail until it is cleared or picked again. While a
game plays the menu lists the running game's entities, and an edit is lost on
Stop like any other. `EntityPickerPlugin`, added by `EditorPlugin`, keeps the
row's text current and closes the menu when its edit target goes. The picker's
`edit_to` takes the value, so MCP's `set_property` writes an entity reference
as a live id such as `42v3`, or the unset value, and refuses any other node
index. The unset value is the node index `18446744073709551615`, `u64::MAX`, as
`inspect` shows it; a client that reads JSON numbers as doubles must pass it
through unchanged.

An edit is the widget's own type, so a value alone cannot pass through it.
`PropertyEditor::edit_to(&value)` returns the edit that sets the field to
`value`; it still goes through `apply`, with its `EditContext`, so validation
holds. The built-in editors for `f32`, `f64`, `usize`, `u32`, `i32`, `Vec2`, `Vec3`, `Quat`, `bool`, `Name` and `String`, the
asset picker, the entity picker and the joint picker implement it; `Quat` goes
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
app.register_asset::<Palette>();
app.register_asset_editor::<Palette, PaletteEditor>()?;
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
impl CreatableAsset for Palette {
    fn new_asset() -> Self {
        Palette {
            colors: Vec::new(),
        }
    }
}

app.register_creatable_asset_editor::<Palette, PaletteEditor>()?;
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

An editor that returns `true` from `AssetEditor::bottom_host`, which is `false`
unless it says otherwise, also gets `EditorHosts::bottom`: a panel in the
`Bottom` strip, to the right of the Asset Browser, shown while its document is
active. It is as high as the strip, and a splitter between it and the Asset
Browser sets the browser's width, `EditorLayout::browser`; on a tab with no
bottom host the Asset Browser takes the whole strip. The `left` host stands in
`Left` in place of the hierarchy, `inspector` in `Right` above the inspector
panel, and `canvas` in `Centre`. Chatter stays visible on every tab. A Scene
document gets no `bottom` host.

An editor that returns `true` from `AssetEditor::form`, which is `false` unless
it says otherwise, shows its `InspectedValues` as the tab's main content. Its
documents get no `canvas` and no `left` host, only `inspector`, and `bottom` if
the editor asks for one. On its tabs the `Centre` is hidden, `Left` collapses,
and `Right` fills the row with no splitter, so `status` reports `left hidden`
and `right fill` and `set_panel_size` refuses both; `EditorLayout::right` is
left as it was for the other tabs. A preview wins: an editor with a preview
keeps the viewport and its hosts whatever `form` says.

The [custom asset example](examples/custom_asset.rs) writes no editor of its
own. Its game marks a non-Clone `Dialogue { text: String, spin_speed: f32 }`
as an [inspected asset](#inspected-assets), so the generic tab opens, edits,
saves and discards it: `text` is a text field and `spin_speed` a number field.
`cargo run -p concerto-editor --example custom_asset` opens the editor over a
temporary project holding one empty scene; New lists ExampleDialogue once that
scene's edit `App` has built. Its smoke test, `tests/editor/asset_editor.rs`, drives a
hand-built editor `App` with `ProjectPlugin`, `ScenePlugin`,
`InspectedAssetPlugin` and `ContentPlugin` through the same steps without a
window: open the project, wait for ExampleDialogue to be creatable, create and
open one, edit both fields in the inspector, check it is unsaved, save, and
decode the file.

A custom editor may load in a synchronous ECS system, as the generic tab does.
Asynchronous systems capture the `AssetEntry` in `EditorDocument.pending` when they start;
before mutating domain state, use `asset_request_is_current` with it to reject
cancelled or superseded results. Call `finish_asset_request` with a mutable
document borrow, the captured `AssetEntry`, and success or an error to update
the tab's current asset and status. A request still pending in its tab needs
that call: MCP's `open_asset` waits for it, and times out without it. A call
waiting on a request that was superseded or whose tab closed times out too. A
result for a closed tab has no document to land on, and switching project
closes every tab. An `AssetEditorCommand::Open`
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

A document that holds its own edits restores them on `DiscardEdits`, which the
prompt sends when the user chooses Discard. The tab can outlive that choice, since
the open that asked for it can fail.

A worker thread that a custom editor polls for its result needs frames to be
polled in, and an agent needs to know it is running: insert `BackgroundWork` on
the document when it starts and remove it when it ends. `status` reads
`busy: yes` while any document holds it.

Register custom interaction systems in `Update` after `EditorPlugin`. Workspace
visibility uses resource change detection and host component changes; a tab
switch, and a scene opening, being replaced, or closing, release transient
input. Loading and saving belong to the custom editor.

A custom editor can bind a key in a context of its own with
`ActionMap::bind`, and push that context while its keys apply. Delete, for
one, is bound to `DeleteEntity` in `ViewportContext` and `TreeContext`, and
`ActionMap::resolve` searches from the last pushed context. The systems that
push those two run in `LateUpdate` in the public `EditorContextSet`, each
frame, so a system that pushes a context over them runs in `LateUpdate`
`.after(EditorContextSet)`; its key then wins while the pointer is over the
viewport or focus is in the tree. Read the fired action with
`EventReader<ActionFired>` and `ActionFired::is`, as the editor reads `Save`.

### Inspected assets

A game marks a reflectable content asset type as inspected with
`app.register_inspected_asset::<T>()` from its own plugin, where `T` is a
`CreatableAsset`, a `LoadableAsset` (an empty `impl` when it has no load-time
work) and `Reflect`. That registers the asset and its type in the game, and
records what the editor needs to create, decode and save it without naming
`T`. The editor learns those kinds, and the game's type registry, from each
scene's edit `App` once it has built: on `SceneOpened`, every inspected kind
that has no editor yet gets the generic `InspectedAssetEditor` and a row in New.
A kind that already has an editor keeps it, and one whose `New <kind> 999` is
not a valid asset name is skipped with a warning. `ProjectTypes` holds the type
registry of the last scene's edit `App`.

Before the first scene's edit `App` has built, and all along in a project with
no scene, an inspected kind has no editor: double-clicking one in the Asset
Browser does nothing, and MCP `open_asset` answers `open_failed`, naming it a
kind with no editor.

The generic tab is a form: `InspectedAssetEditor` returns `true` from
`AssetEditor::form`, so the inspector fills the row above the Asset Browser in
place of the viewport, with no preview and no bottom host, and
it closes like any other tab. A custom editor that calls
`InspectedAssetEditor::build` from its own `build` is its own `AssetEditor` and
is a form only if it returns `true` itself. Its document carries `InspectedDocument`, which
holds the kind's record, the value being edited, and the file's bytes as last
read or saved, and publishes the value as one
[document value](#document-values) named after the kind. Opening reads the
file, decodes it and checks that its header carries the asset's id; a file
that cannot be read or decoded, or holds another asset's id, fails the open
with the error as the tab's status. The value's types come from
`DocumentTypes`, which the tab takes from `ProjectTypes`, so the inspector
walks the game's lists, options and enums. A document whose values are in a
registry other than its hosted `App`'s or the editor's carries
`DocumentTypes`, which `DocumentValue` reads first.

An edit through the inspector marks the tab unsaved. Ctrl+S, or the
unsaved-changes prompt's Save, writes the value over the file it was opened
from, keeps the file's id, reads the written file back as what was last
saved, and ends in `SceneSaved`, with "Saved <address>" as the status until
the next edit clears it. A failed save ends in `SaveFailed`, shows why, and leaves the tab unsaved.
Discard decodes the file as last read or saved back into the value. There is
no undo. A value whose type carries `ReflectSkeletonSource` gives the document
the bones of the skeleton it names in `DocumentBones`, read again whenever an
edit, a save or a discard changes the skeleton it names, so its `JointName`
fields pick from them; see [Joint names](#joint-names). MCP works on the tab as on any document value: `open_asset`,
`inspect`, `set_property`, `list_push`, `list_remove` and `save`.

`InspectedAssetPlugin` marks edits unsaved, saves, loads and reads the skeleton
in the `Update` set `InspectedAssetSystems`; a system that reads an inspected
document's state for the frame runs after it. A custom editor for an inspected
kind, registered with `register_creatable_asset_editor`, wins over the generic
one and builds on it: its `build` calls
`InspectedAssetEditor { record: InspectedAsset::of::<T>() }.build(editor)`,
then inserts what it adds and replaces what it extends, such as
`InspectedValues` with the kind's value followed by values of its own. The
plugin's systems then open, mark unsaved, save and discard the tab unchanged.

### Previews

The Scene editor hosts an `App` for each open scene, with its own camera,
selection, and tools. Any other asset editor can host one too, a preview, by
returning a `PreviewSpec` from `AssetEditor::preview`, which is `None` unless
it says otherwise:

```rust,ignore
impl AssetEditor for PaletteEditor {
    fn preview(&self) -> Option<PreviewSpec> {
        Some(PreviewSpec { build: palette_preview })
    }
}

fn palette_preview(app: &mut App) {
    app.add_system(Update, draw_swatches.at_preview_time())
        .add_listener(set_swatches.at_preview_time());
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
`canvas`. Build the editor's own controls in those two hosts, and in `bottom`
when the editor asks for it; the hierarchy panel is hidden on a preview tab, as
on any custom editor's tab, and so is the inspector panel unless the document
has `InspectedValues`. A preview carries only `ViewportTarget`, which the viewport shows, routes input
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

### The clip, rig and graph editors

The editors of an `AnimationClip`, of a `RetargetRig` and of an
`AnimationGraph` are in the crate
[`concerto-animation-editor`](../animation-editor/README.md), which a game lists
in its `Cargo.toml`. The clip tab plays a clip on its skeleton. The rig tab
inspects a rig beside a second rig and a clip, with both characters side by
side in its preview. Both play through the crate's transport widget, a play,
step, loop and speed row and a timeline over the document's `ClipTransport`,
public for any tab whose preview plays a clip. The graph tab shows a graph's
nodes on a [graph canvas](#the-graph-canvas), where they are added, wired,
inspected and saved. In a game that doesn't list the crate a clip or a graph
has no editor, so opening one opens no tab, and a rig opens in the generic
inspector.

### The timeline

`concerto_editor::timeline::Timeline` is a component on a UI node, which
`TimelinePlugin` turns into a readout over a ruler, a track, a marker lane and
track rows.
The node's owner sizes it and writes its `duration`, `rate`, `time`,
`selection`, `selectable`, `markers`, `selected_marker` and `tracks`; the
widget never writes them. `Timeline` has no builders: an owner compares what it
wants with what is shown each frame and writes only what differs, field by
field when a field such as `tracks` is costly to clone.

- Pressing or dragging on the ruler sends `TimelineScrubbed { time }` to the
  `Timeline` entity. The time snaps to a frame when there is a rate.
- With `selectable` set, a drag on the track sends
  `TimelineSelected { range: Some((first, last)) }`, in frames, inclusive,
  either way round, and a click sends `range: None`. Without a rate there are
  no frames, so the track selects nothing. With `selectable` false the track
  never sends `TimelineSelected` and the selection band stays hidden.
- The marker lane sits under the track and is hidden while `markers` is
  empty. Each `TimelineMarker { time, label, locked }` is a line with its label
  beside it, in the accent colour, the muted text colour when `locked`, and the
  selection colour when its index is `selected_marker`. Markers at the same
  time stack one row each, and the lane grows to the most rows.
- Pressing a marker sends `TimelineMarkerSelected { index: Some(index) }`. A
  click on the track or on the lane outside a marker sends `index: None`, along
  with `TimelineSelected { range: None }` when the track would send it.
- Dragging an unlocked marker sends `TimelineMarkerMoved { index, time }`, the
  time under the pointer snapped to a frame when there is a rate. A locked
  marker sends nothing when dragged.
- Each `TimelineTrack { label, warning, keys }` in `tracks` is a read-only row
  under the marker lane, in a scroll area that takes the wheel and fills the
  space left below the lane. A row draws a tick at each key, and keys within
  3 px of the one before join into a bar, so a clip keyed on every frame draws
  one bar. Rows take no clicks.
- While `tracks` is not empty, a label column runs down the left of every
  band: the marker lane's reads `Events` and each row's its `label`, in the
  warning colour when `warning` is set. The ruler, track, lane, rows and head
  all start after the column and stop short of the scrollbar, so times line
  up, and the track shrinks to one row's height. With no tracks there is no
  column and the track fills the space under the ruler.

The lane is rebuilt only when the markers' labels or lock flags change; a
moved or newly selected marker keeps its node. The rows are rebuilt only when
`tracks` or the ruler's width change. The head is drawn in an overlay
over every band, so it crosses the lane.

Put the listeners for these signals on the `Timeline` entity, and write back
what they decide: a `Timeline` nobody writes to does not move, and a marker
nobody moves stays where it was. `frame_at`,
`time_of_frame` and `frame_count` are the widget's conversions. Ticks keep at
least 64 px between labels. There is no zoom.

### The graph canvas

`concerto_editor::graph_canvas::GraphCanvas` is a component on a UI node, which
`GraphCanvasPlugin` turns into a canvas of boxes joined by links. The node's
owner sizes it and writes its `nodes`, `links`, `mode` and `selection`; the
widget never writes them. Every gesture is a signal sent to the `GraphCanvas`
entity, and a canvas nobody answers changes nothing.

- A `CanvasNode::new(id, title)` is a box, 176 px wide, with its title on the
  first row. `with_subtitle` adds a muted second row, `with_input(label)` adds
  one input pin with its label, `with_output()` gives it an output pin,
  `with_badge` puts a short readout at the right of the title row, and
  `with_emphasis` outlines it: `CanvasEmphasis::Active` in the theme's second
  accent, `Warning` in the warning colour. The `id` is a `CanvasNodeId`, a
  `u32` the owner chooses; links, the selection and the signals name a node by
  it.
- A `CanvasLink::new(from, to)` runs out of `from` and into `to`. `with_slot`
  names the input pin of `to` it ends on, `with_label` writes a label beside
  its vertical run, and `with_emphasis` colours it as it does a node. A link
  whose `from` or `to` is no node's id is not drawn. A link is named by its
  index in `links`.
- `mode` is a `CanvasMode`. In `Flow` a node shows its pins, an input per row
  down its left side and the output at the right of its title row, and a link
  runs from an output pin to the input pin of its slot. In `Diagram` no pin is
  drawn, `inputs` and `output` are ignored, and a link is an arrow from box to
  box.
- `selection` is an `Option<CanvasTarget>`, `Node(id)` or `Link(index)`, drawn
  in the accent colour.

`GraphCanvas::new(mode)`, `with_node` and `with_link` build one. An owner
compares what it wants with what is shown and writes only when they differ.

- Pressing a node or a link sends `CanvasSelected { target: Some(..) }`; a
  click on empty space sends `target: None`. Pressing a pin selects nothing.
- A double click on a node, or Enter with a node selected, sends
  `CanvasActivated { node }`.
- In `Flow`, a drag from an output pin that ends on an input pin of another
  node sends `CanvasConnected { from, to, slot: Some(slot) }`. In `Diagram`, a
  drag from one box that ends on another sends it with `slot: None`. A line
  follows the pointer during the drag. A drag that ends anywhere else sends
  nothing.
- Delete with something selected sends `CanvasDeleted { target }`.
- A right click sends `CanvasMenuRequested { at, on }`: `at` is the pointer's
  position in the window, and `on` is a `CanvasPart`, one of `Node(id)`,
  `Input { node, slot }`, `Output(id)`, `Link(index)` or `Empty`.

A pin is hit within 7 px of its centre and a link within 4 px of its line; a
pin wins over the node it sits on, and a node over a link behind it. Delete and
Enter are the actions `DeleteCanvasSelection` and `ActivateCanvasSelection`,
bound in `CanvasContext`, which is pushed after `EditorContextSet` while the
pointer is over a canvas or one has keyboard focus, and not while the
unsaved-changes prompt or the import dialog is open. They go to the canvas
under the pointer, or else the focused one.

The widget owns the view. A drag with the middle button pans it, without
limit, and sending `FrameCanvasNode { node }` to the canvas scrolls that node
into view, 24 px inside the edge, once the canvas has been laid out. There is
no zoom.

Where things sit is `graph_canvas::layout::layout(nodes, links, mode)`, a pure
function that returns a `CanvasLayout`: a `NodeBox` per node with its
rectangle and pin centres, a `LinkRoute` per link, and the content's size.
`CanvasLayout::part_at(point)` is the hit test.

- `Flow`: a node that is no link's `from` is a root, and each root is a tree
  growing leftward from it: a node's inputs are one column to its left,
  stacked top to bottom by `slot`, each with its own subtree, and the node is
  centred on them. Roots stack top to bottom in node order, with the deepest
  tree's leaves in the first column. A node a link would place twice, as a
  loop does, stays where it was first placed.
- `Diagram`: a node's column is its distance from the first node along links,
  in the direction they point. Nodes stack in node order within a column, and
  those no path reaches share a last column.

A link is three thin boxes: out of its start, up or down, and into its end. In
`Diagram` it ends in an arrowhead, the `caret-right` or `caret-left` glyph. A
link to a later column leaves the right side of its box a third of the way
down and enters the left side at the same height; a link to an earlier column
leaves the left side two thirds of the way down and enters the right side
there. A link within a column, or from a node to itself, leaves the right side
at the upper height and comes back into the right side at the lower one. Two
links between the same pair in opposite directions so differ in height at
both ends and in where they turn. Nodes are drawn over links, so a link that
spans several columns passes behind the boxes between.

The boxes are rebuilt only when the layout could change: the mode, a node's
id, title, pins or having a subtitle, or a link's ends, slot or label. A
changed selection, emphasis, badge or subtitle text is written to the nodes
that are there.
