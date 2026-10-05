# Editor MCP Server — Design

**Status:** M1 done; M2–M5 not started
**Touches:** new `crates/mcp`; `crates/editor` (headless split, viewport, tools),
`crates/editable`, `crates/ecs` (entity ids), `crates/render` (readback)

## Problem

An agent working on Concerto today reads code and runs tests, but it cannot
see or use the editor. Every bug in the editor's design history that a person
found by looking at it, an agent would have missed: the grid and light that
never reached the render world (M11), the viewport rendering at 800×600 and
stretching (M13), tab titles collapsing to their padding (M2). Each of those
compiled, passed its tests and looked wrong.

The editor already has every verb an agent needs: open a project, open an
asset, walk the live entity tree, read component values through
`TypeInfo::read`, edit properties through `PropertyEditor`, move the camera,
frame a selection. They are just only reachable through a mouse.

An [MCP](https://modelcontextprotocol.io) server inside the editor exposes those
verbs as tools, plus the one thing a mouse never needed: a screenshot.

## Goals

- An agent can start its own editor, with no window and no display, and drive
  it through MCP — on a workstation, in CI, or in a cloud session.
- An agent can see what the viewport renders, as an image.
- Edits made through MCP take the inspector's path: the same registry,
  validation and mutation boundary. There is no second way to change the world.
- Tool output is compact and bounded. A 5,000-entity scene must not produce a
  5,000-line answer.
- Later, an agent can attach to the editor a person is using, and the two share
  one selection.

## Non-goals

- **Saving.** The editor never writes assets, and MCP inherits that. Edits are
  temporary, as they are in the inspector; closing the tab discards them.
- **Import.** Importing belongs to the `import` CLI, which an agent can run
  directly.
- **Spawning, despawning, adding or removing components.** The editor cannot do
  these yet; MCP will not do them first.
- **Undo.** None exists. Reopening the asset is the undo.
- **Driving the UI by synthetic input.** Tools address the editor's state, not
  its pixels.
- **Headless UI.** A headless editor has no panels. Rendering the editor's own
  UI offscreen would need a stand-in for `Window`, which wraps a real winit
  window; that is the attach mode's job (M5), not headless's.
- **Remote access.** Stdio, or loopback in M5.

## Decisions

| Decision | Chosen | Because |
| --- | --- | --- |
| First transport | Stdio, with the editor headless | The agent spawns the editor itself, so it works where no display exists, and there is nothing to attach to or secure. The engine already runs headless |
| Headless | `EditorPlugin { headless }`, on top of `DefaultPlugins::headless()`'s rule | Same switch the engine uses: no `WindowPlugin`, no `UIPlugin`. The editor splits into a core that runs either way and panels that need a window |
| Second transport | Streamable HTTP on loopback (M5), attaching to a windowed editor | The person launches that editor, so the agent must attach rather than spawn |
| Where the server lives | In the editor process. Protocol, runner and tool registry in `concerto-mcp`; the editor's tools in `concerto-editor` behind an `mcp` feature | Tools need the live `World`. The server knows nothing about the editor, so a game can serve its own tools the same way — and the editor binary can depend on it without a package cycle (see M1 notes) |
| Protocol implementation | `rmcp`, the official Rust SDK | Protocol revisions and JSON Schema generation are someone else's problem. tokio runs on one thread inside one crate |
| Threading | Server thread queues requests; one exclusive system answers them | The `World` is touched only on the main thread, at a known point in the frame, like `apply_property_commits` |
| Headless frame pacing | Frames run only while a request is in flight | An idle headless editor on lavapipe would otherwise burn a core rendering a view nobody is looking at |
| Long operations | Tools block until done, with a timeout | An agent that opens a project wants the catalogue, not a "loading" reply and a polling loop |
| Selection | Shared: `select` writes the editor's `Selection` | An agent and a person looking at one editor should look at the same thing, and an agent should be able to point |
| Attribution | None. Agent edits look like any other edit | They are the same live-world edit through the same path; a label adds UI for no decision anyone makes |
| Entity ids | `"42v3"` (index, generation) | Short, and the generation makes a stale id an error rather than a different entity |
| Output | Compact text for trees and lists, JSON for values | Trees are read, not parsed; component values are structured and are fed back into `set_property` |
| Edits | `PropertyEditor` gains an optional JSON parser | Validation lives in the adapter; a JSON path that bypasses it would let MCP write values the inspector rejects |

Two rejected alternatives are worth recording. A stdio bridge binary that
forwards to a running editor over a socket buys nothing once the editor speaks
stdio itself. A hand-written JSON-RPC server avoids tokio, but MCP has revised
its transport twice in a year, and tracking that is not this engine's job.

---

## Section 1 — The headless editor

### What headless means already

`DefaultPlugins::headless()` registers everything except `WindowPlugin` and
`UIPlugin`. `RenderPlugin` already copes: with no `Window` at build it creates
no surface and renders only to texture targets. On a machine with no GPU,
Mesa's lavapipe (`apt-get install -y mesa-vulkan-drivers libvulkan1`) runs real
frames; the command-flushing measurements were taken that way.

The editor's `main.rs` does not use `DefaultPlugins` — it registers its own
smaller list — but follows the same rule: `--headless` drops `WindowPlugin` and
`UIPlugin` from that list and passes `headless: true` to `EditorPlugin`.

### Why the editor cannot run headless today

A system whose `Res<T>` is missing panics, and much of the editor reads
window-world resources: `Res<Window>` in the dock, viewport and window chrome;
`Res<Input>` in viewport navigation; `EventReader<ActionFired>` in framing;
`Res<UITheme>`, `PanelRegistry` and UI components throughout the panels. None
of those systems has a reason to run without a window.

### The split

`EditorPlugin { project, decorated, headless }` registers a core always, and the
panels only when `headless` is false.

| Core — runs headless | Panels — windowed only |
| --- | --- |
| `Selection` | `FontsPlugin`, `DockPlugin`, `ActionsPlugin` |
| `ProjectPlugin`, `EditorCommands`, `ProjectState` | `HierarchyPlugin`, `ContentPlugin`, `DiagnosticsPlugin` |
| `AssetEditorRegistry`, `AssetEditorCommands`, `ActiveEditor`, `process_editor_commands` | Inspector panel systems (`build_panel`, row sync, numeric fields) |
| `ScenePlugin` (the `Scene` asset editor) | Workspace hosts and visibility, `TabsPlugin`, `ShellPlugin` |
| Inspector model: `InspectorRegistry`, `PropertyCommits`, `apply_property_commits`, `register_editable::<Transform>()` | `WindowChromePlugin` |
| Viewport core: `EditorViewport`, `FlyCamera`, `ViewportCommands`, `spawn_camera`, framing | Viewport panel, `navigate`, `sync_viewport_size`, `sync_viewport_context`, `zoom` |

`InspectorPlugin` and `ViewportPlugin` each become a core plugin plus a panel
plugin. Asset editors already tolerate having no panels — "headless hosts have
no UI containers" — so `create_editor_hosts` simply does not run.

### Three viewport changes

The viewport mixes camera state with window input in two systems, and owns its
resolution through the UI layout. Headless needs each pulled apart:

1. **Fly camera → transform.** `navigate` both reads input and writes the
   camera's `Transform` from `FlyCamera`, and so do framing and `zoom`. The
   write moves to one core system, `apply_fly_camera`, which runs last and
   writes only when the pose differs. Everything else — input, framing, a
   tool — only writes `FlyCamera`. The navigation requests `navigate` used to
   consume (`reset` after a scene is replaced, `release_navigation` on a tab
   switch) move to a core `process_navigation_requests`; the panel keeps only
   the window-side half, releasing a captured pointer.
2. **Framing.** `frame_requested_bounds` reads `ActionFired`. `ViewportCommands`
   gains `frame_selected` beside the existing `frame_all`; a panel-side system
   turns the `FrameSelected`/`FrameAll` actions into those flags, and the core
   system reads only the flags.
3. **Resolution.** Windowed, `sync_viewport_size` follows the panel's layout.
   Headless, there is no layout: the viewport starts at `--viewport 1280x720`
   (default) and `screenshot` may resize it, since nobody else is looking.
   `EditorViewport` carries the size, so the camera's initial aspect comes from
   it rather than from the texture store, which `AssetServer::add` has not
   reached by `Startup`.

### The runner

Without `WindowPlugin` the app keeps `run_once` and exits after one frame, and
`ScheduleRunnerPlugin` spins without sleeping. The MCP stdio plugin installs its
own runner:

```
finish plugin build
loop:
    if no request is queued or pending:
        block on the inbox (or exit when stdin closes)
    app.update()
```

The editor runs frames only while there is work: a queued request, a pending
one waiting on a project load or a readback, or a background job a pending
request depends on. When stdin closes, the runner returns and the process
exits; an agent never leaves an orphaned editor behind.

---

## Section 2 — The server

```
crates/mcp/src/            concerto-mcp: knows nothing about the editor
  lib.rs        McpPlugin, McpTool, McpTools, McpInbox, PendingRequests,
                serve_requests, Handled::after, call_tool
  runner.rs     McpRunnerPlugin, McpActivity, run_until_closed
  server.rs     rmcp ServerHandler on its own thread; spawn_server, stdio
crates/editor/src/mcp/     concerto-editor, `mcp` feature
  mod.rs        EditorMcpPlugin, identity, WorldIndex, argument helpers
  observe.rs    status, list_assets, scene_tree, find_entities, inspect
  act.rs        open_project, open_asset, close_editor, select, frame, set_camera
```

### The request path

```
 agent ──stdio──▶ rmcp (tokio thread)
                     │ McpRequest { tool, args, reply: oneshot::Sender }
                     ▼
               crossbeam channel  ── also wakes the runner
                     │
 main thread, First:  serve_requests(&mut World)   ◀── exclusive system
 between frames:      read-only calls only (see M1 notes)
                     │ immediate → reply now
                     │ deferred  → PendingRequests
                     ▼
 each later frame:   poll pending → reply, or time out
```

`serve_requests` is an exclusive system (`fn(&mut World)`), registered in
`First`, so whatever a handler queues is processed by the same frame's
`Update`. It is the editor's second dynamic
mutation boundary and follows the first one's rules: it drains its queue
completely, never holds a world borrow across frames, and leaves UI entities to
the systems that own them.

A handler returns one of two things:

```rust
enum Handled {
    Done(Result<ToolOutput, ToolError>),
    /// Checked once per frame until it yields, or until the deadline.
    Pending {
        deadline: Instant,
        poll: Box<dyn FnMut(&mut World) -> Option<Result<ToolOutput, ToolError>> + Send>,
    },
}
```

`open_project` pushes `EditorCommand::OpenProject` and returns `Pending` with a
poll that waits for `ProjectState::generation` to advance or for `status` to
report an error. `open_asset` waits for the document's `pending` to clear.
`screenshot` waits for the render world's readback. Nothing blocks a frame.

Pending requests capture `ProjectState::generation` and the document's
`request_generation`, exactly as asynchronous asset editors do, and fail with
`superseded` rather than report on the wrong project.

### Configuration

The repository ships a project-scoped `.mcp.json`:

```json
{
  "mcpServers": {
    "concerto": {
      "command": "cargo",
      "args": ["run", "-q", "-p", "concerto-editor", "--", "--headless", "--mcp"]
    }
  }
}
```

No `--project`: one checkout holds several example projects, and the agent
opens the one it is working on with `open_project`. Tools appear to the agent as
`mcp__concerto__<tool>`, so their names carry no prefix.

A cold `cargo run` compiles the engine, Jolt included, and outlasts an MCP
client's startup timeout. The README says to build once first
(`cargo build -p concerto-editor`); after that `cargo run -q` only checks
freshness before starting. Claude Code's `MCP_TIMEOUT` raises the limit for a first run.

**Stdout belongs to the protocol.** Logs already go to stderr through
`env_logger`. The only `println!`s reachable from engine crates today are in
tests; `--mcp` additionally installs a panic hook that writes to stderr, and a
test that runs the headless editor through a few requests and asserts that
every stdout line parses as JSON-RPC keeps it that way.

**No GPU.** If `RenderPlugin` finds no adapter the editor exits at startup with
a stderr message naming lavapipe, rather than serving tools that will all fail.

---

## Section 3 — Tools

Every tool that names an entity takes the `"42v3"` form and fails with
`stale_entity` when the generation no longer matches. Every tool that returns a
collection takes `limit` and says how many it left out.

### Observe

| Tool | Arguments | Returns |
| --- | --- | --- |
| `status` | — | Mode (headless or windowed), project root and status line, open documents (id, asset, kind, status), active document, selection, scene loading state, viewport size, frame number |
| `list_assets` | `query?`, `kind?`, `folder?`, `limit = 50`, `offset = 0` | `address · kind · id` lines from `Project::filtered_assets`, plus the total |
| `scene_tree` | `root?`, `depth = 3`, `limit = 200`, `all = false` | Indented tree under the open scene's `SceneRoot`, or under `root` |
| `find_entities` | `name?`, `component?`, `limit = 50` | Matching entities with their paths from the root |
| `inspect` | `entity`, `components?` | Each component's JSON value via `TypeInfo::read`; editable property paths join it with `set_property` in M3 |

`scene_tree` renders one line per entity:

```
12v1  Hero                       Transform Name MeshRenderer
  13v1  Body                     Transform Name SkinnedMesh
    14v1  Spine                  Transform Name
      … 23 more below depth 3 (scene_tree root=14v1)
  15v1  Camera_Rig               Transform Name VirtualCamera
… 112 more siblings (scene_tree root=12v1 limit=400)
```

The truncation lines are written as the call that would continue, so the agent
never has to work out how to page. `all = true` drops the `SceneRoot` filter and
shows the editor's own helpers — the toggle the Wonderland design planned for
the hierarchy panel, wanted for the same reason: the runtime-debugger case.

`inspect` keeps the inspector's honesty rule: a component that is present but
fails to serialise is listed with `"value": null, "unavailable": true`, never
omitted.

### Act

| Tool | Arguments | Effect |
| --- | --- | --- |
| `open_project` | `path` | `EditorCommand::OpenProject`; waits for the catalogue (timeout 30s) |
| `open_asset` | `asset` (address or id) | `EditorCommand::OpenAsset`; waits for the document to load or fail (timeout 30s) |
| `close_editor` | `document` | `AssetEditorCommand::Close` |
| `select` | `entity` or `asset`, or nothing to clear | Writes `Selection`; in a windowed editor the hierarchy and inspector follow |
| `frame` | `entity?` | Selects `entity` if given, then sets `frame_selected`, or `frame_all` with no argument |
| `set_camera` | `position`, then `look_at` or `yaw`+`pitch` | Writes `FlyCamera`; returns the resulting pose |

### Edit

| Tool | Arguments | Effect |
| --- | --- | --- |
| `set_property` | `entity`, `component`, `path`, `value` | Applies one validated edit; returns the component's new value |

`component` is the short name (`Transform`) or the full path when the short
name is ambiguous. `path` is dot-separated (`translation`, `light.intensity`).
`value` is JSON in the shape `inspect` reported, which is what makes the two
tools a pair: read, change one field, write back.

### Capture

| Tool | Arguments | Returns |
| --- | --- | --- |
| `screenshot` | `max_edge = 1024`; headless only: `width?`, `height?` | PNG of the viewport's render target as MCP image content, plus the camera pose it was taken from |

It captures what the editor camera sees. To see something from elsewhere, an
agent calls `set_camera` or `frame` first; a capture that moved the camera
itself would, in a windowed editor, leave the person's view somewhere
unexpected. Headless, `width`/`height` resize the viewport before the capture
(and stay), since there is no panel to own its size.

### Deliberately absent

`run_action` (firing an `ActionFired` by name) would make every shortcut
scriptable, but interned `ActionLabel`s have no name lookup, and headless has no
`ActionMap`. Logs are M4; until then the agent that spawned the editor can read
its stderr only through the MCP client's own logs.

---

## Section 4 — Engine additions

### 4a. Entity ids as text (`crates/ecs`)

`Entity` displays as `Entity(index: 42, gen: 3)`. Add a compact form and its
inverse, `Entity::to_id_string() -> "42v3"` and `Entity::parse_id(&str)`, and
`World::entity_is_current(Entity)`. Nothing else needs to know the format.

### 4b. Edits from JSON (`crates/editable`, `crates/editor`)

`PropertyPath` holds `&'static str` segments, which is what lets it be built
without allocation from `visit`. A path arriving as text has to be resolved
against the value it addresses:

```rust
impl PropertyPath {
    /// Resolves `"light.intensity"` by walking `root`'s visitors, so every
    /// segment is the `&'static str` the derive produced.
    pub fn resolve(root: &dyn Editable, dotted: &str) -> Result<Self, PathError>;
}
```

`PropertyEditor` gains one method with a default, so no existing editor breaks:

```rust
/// Edits that set this value from JSON, parsed in full before any is applied.
/// Editors that cannot be driven from data keep the default.
fn edits_from_json(&self, _current: &T, _json: &Value) -> Result<Vec<Self::Edit>, EditError> {
    Err(EditError::NotScriptable)
}
```

It returns a `Vec` because the built-in numeric editor edits one slot at a time:
`[1, 2, 3]` for a `Vec3` is three `NumericEdit`s. Taking `current` lets an
editor accept a partial value (`{"y": 2}`). The default numeric editors
implement it and reject non-finite numbers while parsing, so a parsed edit list
cannot fail half-way.

`set_property` then builds a `PropertyCommit` and calls the existing
`apply_property_commit`. The stale-editor check, the type checks and the
adapter's validation all run exactly as they do for a click in the inspector.
`EditError` gains `NotScriptable`, and the MCP error names the editor type so
an agent knows the field exists but is not settable.

### 4c. Texture readback (`crates/render`)

The terminal renderer already reads a render target back to the CPU, and its
split-worlds note (`2026-09-25-terminal-renderer-split-worlds.md`) records
where that went wrong: readback must be a render-world system, in the render
subapp's `LateRender`, after `finish_render`. Build it once, generally:

```rust
/// Main world: ask for a texture's pixels after the next frame renders.
pub struct ReadbackRequest { pub texture: AssetHandle<Texture>, pub reply: Sender<ReadbackResult> }
```

Requests are extracted into the render world, copied to a staging buffer after
the frame's submit, mapped, and sent back with row padding removed. Render
targets are already created with `COPY_SRC`. The terminal renderer can move onto
this when it is fixed; `screenshot` is its first user. PNG encoding happens on
the server thread, not the main one.

---

## Section 5 — Errors

Tool errors are returned as MCP tool results with `isError: true`, never as
protocol errors, so the agent sees them and can correct itself. Each has a
stable code and a sentence that says what to do next:

| Code | Example message |
| --- | --- |
| `no_project` | No project is open. Call `open_project` first. |
| `stale_entity` | 42v3 no longer exists (the slot now holds 42v4). Call `scene_tree` again. |
| `unknown_component` | No component named `Light` on 42v3. It has: Transform, Name, PointLight. |
| `ambiguous_component` | `Transform` matches two types; pass the full path. |
| `path_not_found` | `Transform` has no field `position`. Fields: translation, rotation, scale. |
| `not_scriptable` | `MaterialSlot.texture` is edited by `TexturePicker`, which cannot be set from JSON. |
| `rejected` | The editor rejected the value. |
| `busy` | The editor is opening a project; try again when `status` reports it ready. |
| `superseded` | The project changed while this request was pending. |
| `timeout` | Still loading after 30s. `status` shows its progress. |

---

## Section 6 — Testing

Handlers are plain functions of `&mut World` and JSON arguments, so most of the
surface is tested without a transport, a window or a GPU: build a `World` with
the core plugins and a spawned scene, call the handler, assert on its output.
The `scene_tree` truncation lines, the stale-id path and the not-scriptable
path each get a test, because those are the outputs an agent acts on.

The headless split gets its own test: build the app with
`EditorPlugin { headless: true }` and no window plugins, finish the build, and
run a few frames. Today that panics on the first missing `Res<Window>`; the
test is what keeps a future panel system from leaking into the core.

One end-to-end test runs `rmcp`'s client against the server over an in-memory
duplex: initialise, list tools, call `status`, open `examples/render-test`,
read the tree. It pins the schema and the stdout-is-protocol rule.

`edits_from_json` for the numeric editors is tested alongside the existing
numeric commit tests, including that `[1, NaN, 3]` changes nothing.

`screenshot` needs a GPU. Its test skips when no adapter is found, as
`test_headless_gpu_render_produces_output` already does, and runs on lavapipe
where it is installed.

---

## Section 7 — Milestones

Each leaves the editor working and is shippable alone.

**M1 — Headless editor over stdio.** The core/panel split and the three
viewport changes (Section 1), `--headless`, the request-driven runner,
an MCP server crate, `--mcp`, `.mcp.json`, entity ids (4a). Tools:
`status`, `list_assets`, `scene_tree`, `find_entities`, `inspect`,
`open_project`, `open_asset`, `close_editor`, `select`, `frame`, `set_camera`.
After M1 an agent anywhere can answer "what is in this scene and where is it".

*Done.* Verified against `examples/render-test` on lavapipe: open the project,
open Sponza (105 entities), walk, find, inspect, frame, place the camera, close.
Departures from the design, and what forced them:

- **Two crates, not one.** `crates/editor-mcp` depending on the editor, with the
  editor binary depending on it for `--mcp`, is a package cycle, which Cargo
  rejects even for optional dependencies. So the server became the app-agnostic
  `concerto-mcp` (protocol thread, inbox, pending calls, runner, a registry
  keyed by tool name with schemas generated from each tool's argument struct),
  and the tools moved into the editor behind a default `mcp` feature. A game
  can now serve its own tools through the same crate — the runtime-debugger
  case again.
- **Read-only calls cost no frame.** As designed, every call ran a frame, and a
  frame right after Sponza loads took 11 s in a debug build on lavapipe, so
  `scene_tree` did too. Tools now declare themselves read-only, and the runner
  answers read-only calls straight from the inbox between frames, stopping at
  the first call that may change something so order is kept and writes still
  land inside a frame where change detection sees them. Looking now takes
  milliseconds; acting costs one frame.
- **Calls wait for a project that is still opening.** With `--project` at
  launch, an immediate `open_asset` found the editor busy and failed — which an
  agent would hit every time. `Handled::after(ready, then)` in `concerto-mcp`
  waits for a condition and then runs a handler that may itself go pending;
  every tool that needs the catalogue waits for it through that.
- **The runner needs to know about background work.** A project opened at
  launch loads on a worker thread that only finishes if frames run. Apps set
  `McpActivity::keep_awake` while busy; the editor does for a project opening,
  a scene loading, or any document with a pending request. Busy frames are
  capped at ~60 per second.
- **No GPU now says what to install.** `RenderPlugin` unwrapped the adapter
  request; it now names lavapipe in the panic message, which is what an agent
  sees in the MCP client's server log.
- **`inspect` has no editable paths yet.** They only mean something with
  `set_property`, so they move to M3 with it.

Found on the way, not fixed here:

- `concerto-render` did not compile on any target: `Limits` was used without
  an import inside a `cfg!` branch, and `cfg!` compiles both branches. Fixed in
  passing (`wgpu::Limits`), since nothing could be built without it.
- **As many `App`s updating at once as the global compute pool has threads
  deadlock it.** Eight bare apps (`MainSchedulePlugin` and `TimePlugin` only)
  on eight threads hang on a 4-core machine. Tests that build apps in parallel —
  the default for `cargo test` — are exposed. The new tests serialise their
  apps behind a lock; the pool itself is untouched.

**M2 — See.** Texture readback (4c) and `screenshot`. This is the milestone
that pays for the rest.

**M3 — Edit.** `PropertyPath::resolve`, `edits_from_json` and `set_property`
(4b).

**M4 — Logs.** A ring-buffer logger in front of `env_logger`, and a `logs` tool
(`since`, `level`, `limit`). The `PropertyCommit dropped` warning is the kind
of thing an agent should see without reading stderr.

**M5 — Attach.** `--mcp-http [port]` on a windowed editor: Streamable HTTP on
`127.0.0.1`, `Origin` checked against localhost (the MCP specification's
defence against DNS rebinding), and the `ControlFlow::Poll` frame loop draining
the inbox instead of the request-driven runner. Shared selection is when
`select` becomes visible to a person. Adds `screenshot target=window`, which
captures the editor's own UI and needs the surface created with `COPY_SRC`.
