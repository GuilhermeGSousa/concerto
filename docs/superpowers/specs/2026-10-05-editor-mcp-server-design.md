# Editor MCP Server — Design

**Status:** draft, for review
**Touches:** new `crates/editor-mcp`; small additions to `crates/editor`,
`crates/editable`, `crates/ecs` and `crates/render`

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

- An agent can attach to an editor the user is running, observe it, and act on
  it with the same verbs the user has.
- An agent can see what the viewport renders, as an image.
- Edits made through MCP take the inspector's path: the same registry,
  validation and mutation boundary. There is no second way to change the world.
- Tool output is compact and bounded. A 5,000-entity scene must not produce a
  5,000-line answer.
- The server is opt-in, local-only, and costs nothing when off.

## Non-goals

- **Saving.** The editor never writes assets, and MCP inherits that. Edits are
  temporary, as they are in the inspector; closing the tab discards them.
- **Import.** Importing belongs to the `import` CLI, which an agent can run
  directly.
- **Spawning, despawning, adding or removing components.** The editor cannot do
  these yet; MCP will not do them first.
- **Undo.** None exists. MCP edits are as temporary as inspector edits, so
  reopening the asset is the undo.
- **Driving the UI by synthetic input.** Clicking buttons by coordinate is
  brittle and slow; tools address the editor's state, not its pixels.
- **Remote access.** Loopback only.

## Decisions

| Decision | Chosen | Because |
| --- | --- | --- |
| Where the server lives | In the editor process, in its own crate | Tools need the live `World`; a separate process would need its own protocol to reach it. A crate keeps tokio out of `concerto-editor` |
| Transport | Streamable HTTP on loopback (attach); stdio later (headless) | The user launches the editor, so the agent must attach to it, not spawn it. Stdio fits the case where the agent *does* spawn it |
| Protocol implementation | `rmcp`, the official Rust SDK | Protocol revisions, capability negotiation and JSON Schema generation are someone else's problem. tokio runs on one thread inside one crate |
| Threading | Server thread queues requests; one exclusive system answers them | The `World` is touched only on the main thread, at a known point in the frame, like `apply_property_commits` |
| Long operations | Tools block until done, with a timeout | An agent that opens a project wants the catalogue, not a "loading" reply and a polling loop |
| Entity ids | `"42v3"` (index, generation) | Short, and the generation makes a stale id an error rather than a different entity |
| Output | Compact text for trees and lists, JSON for values | Trees are read, not parsed; component values are structured and are fed back into `set_property` |
| Edits | `PropertyEditor` gains an optional JSON parser | Validation lives in the adapter; a JSON path that bypasses it would let MCP write values the inspector rejects |

The rejected transport alternative is worth recording: a small stdio bridge
binary that Claude Code spawns and that forwards to the editor over a socket.
It costs a second process and a second protocol to buy nothing HTTP does not
already give, since Claude Code connects to HTTP servers directly.

The rejected protocol alternative is a hand-written JSON-RPC server on
`tiny_http`. It is perhaps 300 lines and avoids tokio, but MCP has revised its
transport twice in a year, and tracking that is not this engine's job.

---

## Section 1 — Architecture

```
crates/editor-mcp/src/
  lib.rs        EditorMcpPlugin { transport }, McpInbox, serve_requests
  server.rs     rmcp ServerHandler on its own thread; tool schemas
  tools/
    observe.rs  status, assets, scene_tree, find_entities, inspect
    act.rs      open_project, open_asset, close_editor, select, frame, camera
    edit.rs     set_property
    capture.rs  screenshot
  format.rs     entity ids, tree rendering, truncation
```

### The request path

```
 Claude Code ──HTTP──▶ rmcp (tokio thread)
                          │ McpRequest { tool, args, reply: oneshot::Sender }
                          ▼
                    crossbeam channel
                          │
 main thread, Update:  serve_requests(&mut World)   ◀── exclusive system
                          │ immediate → reply now
                          │ deferred  → PendingRequests
                          ▼
 each later frame:     poll pending → reply, or time out
```

`serve_requests` is an exclusive system (`fn(&mut World)`), registered in
`Update` before `apply_property_commits`. It is the editor's second dynamic
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
`screenshot` waits for the render world's readback. Nothing blocks the frame.

The editor runs with `ControlFlow::Poll`, so the queue is drained every frame
even when nobody is touching the mouse. A minimised window may stop presenting
on some compositors; `serve_requests` runs in the main world and is unaffected,
but `screenshot` would wait on a render that never happens and time out with a
message saying so.

### Generations

A project switch or a closed tab can land between a tool's request and its
reply. Pending requests capture `ProjectState::generation` and the document's
`request_generation`, exactly as asynchronous asset editors do, and fail with
`superseded` rather than reporting on the wrong project.

---

## Section 2 — Transport and configuration

```sh
cargo run -p concerto-editor -- --project examples/render-test --mcp
cargo run -p concerto-editor -- --project examples/render-test --mcp-port 7311
```

`--mcp` binds `127.0.0.1:7311`; `--mcp-port` picks another port. Off by default.
The repository ships a project-scoped `.mcp.json` so Claude Code finds it with
no setup:

```json
{
  "mcpServers": {
    "concerto": { "type": "http", "url": "http://127.0.0.1:7311/mcp" }
  }
}
```

Tools then appear to the agent as `mcp__concerto__<tool>`, so tool names carry
no prefix of their own.

**Security.** Loopback only, never `0.0.0.0`. The server rejects requests whose
`Origin` header names anything but localhost, as the MCP specification requires
for HTTP servers, which closes DNS rebinding from a browser tab. A bearer token
is not worth its friction while the server cannot write files, and should be
reconsidered the day it can.

**Stdio (M5).** `--headless --mcp-stdio` runs the editor without a window and
speaks MCP on stdin/stdout, for an agent that should start its own editor — in
CI, or in a cloud session with no display. It needs two things this design does
not otherwise: a windowless runner (the render plugin already tolerates having
no surface; the editor's `WindowPlugin` dependency is what has to give), and a
stdout that carries nothing but protocol, which means auditing every `println!`
reachable from the editor. A GPU is still required for `screenshot`; a software
Vulkan driver (lavapipe) is enough.

---

## Section 3 — Tools

Every tool that names an entity takes the `"42v3"` form and fails with
`stale_entity` when the generation no longer matches. Every tool that returns a
collection takes `limit` and says how many it left out.

### Observe

| Tool | Arguments | Returns |
| --- | --- | --- |
| `status` | — | Project root and status line, open documents (id, asset, kind, status), active document, selection, scene loading state, viewport size, frame number |
| `list_assets` | `query?`, `kind?`, `folder?`, `limit = 50`, `offset = 0` | `address · kind · id` lines from `Project::filtered_assets`, plus the total |
| `scene_tree` | `root?`, `depth = 3`, `limit = 200`, `all = false` | Indented tree under the open scene's `SceneRoot`, or under `root` |
| `find_entities` | `name?`, `component?`, `limit = 50` | Matching entities with their paths from the root |
| `inspect` | `entity`, `components?` | Each component's JSON value via `TypeInfo::read`, and its editable property paths with their types |

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
shows the editor's own helpers and UI — the toggle the Wonderland design
planned for the hierarchy panel but never built, wanted for the same reason:
the runtime-debugger case.

`inspect` keeps the inspector's honesty rule: a component that is present but
fails to serialise is listed with `"value": null, "unavailable": true`, never
omitted.

### Act

| Tool | Arguments | Effect |
| --- | --- | --- |
| `open_project` | `path` | `EditorCommand::OpenProject`; waits for the catalogue (timeout 30s) |
| `open_asset` | `asset` (address or id) | `EditorCommand::OpenAsset`; waits for the document to load or fail (timeout 30s) |
| `close_editor` | `document` | `AssetEditorCommand::Close` |
| `select` | `entity` or `asset`, or nothing to clear | Writes `Selection`, so the hierarchy and inspector follow |
| `frame` | `entity?` | Frames the entity, or the whole scene, through the viewport's existing bounds path |
| `set_camera` | `position`, then `look_at` or `yaw`+`pitch` | Writes `FlyCamera`; returns the resulting pose |

`select` changes what the user sees. That is deliberate: an agent and a person
looking at the same editor should be looking at the same thing, and "I've
selected the spine bone for you" is a useful thing for an agent to be able to
say.

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
| `screenshot` | `max_edge = 1024` | PNG of the viewport's render target, as MCP image content, plus the camera pose it was taken from |

It captures what the viewport shows, at the viewport's resolution, downscaled
so the longer edge fits `max_edge`. To see something from elsewhere, an agent
calls `set_camera` or `frame` first. A capture that changed the camera itself
would leave the user's view somewhere unexpected.

Capturing the whole window — the editor's own UI, which is engine UI and has had
its share of bugs only visible on screen — is a second target, `window`, in M2b.
It needs the surface texture created with `COPY_SRC`, which costs a usage flag
on the swapchain and is worth confirming against each backend before turning on.

### Deliberately absent

`run_action` (firing an `ActionFired` by name) is the obvious next tool, and it
would make every keyboard shortcut scriptable. It waits on a name lookup for
interned `ActionLabel`s, which do not have one. Logs are M4; until then an agent
running the editor itself can read stderr.

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

### 4c. Viewport readback (`crates/render`, `crates/editor-mcp`)

The terminal renderer already reads a render target back to the CPU, and its
split-worlds note (`2026-09-25-terminal-renderer-split-worlds.md`) records
where it went wrong: readback must be a render-world system, in the render
subapp's `LateRender`, after `finish_render`. This design needs the same thing
and should build it once, as a general `TextureReadback` in `crates/render`:

```rust
/// Main world: ask for a texture's pixels.
pub struct ReadbackRequest { pub texture: AssetHandle<Texture>, pub reply: Sender<ReadbackResult> }
```

Requests are extracted into the render world, copied to a staging buffer after
the frame's submit, mapped, and sent back with the row padding removed. The
terminal renderer can move onto it when it is fixed; `screenshot` is its first
user. PNG encoding happens on the server thread, not the main one.

### 4d. Editor `WindowPlugin` independence (M5 only)

The editor reads `Input`, `ActionMap` and `CloseRequest`, all inserted by
`WindowPlugin`. Headless needs them inserted without a window. Scoped in M5,
not before.

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

Handlers are plain functions of `&mut World` and JSON arguments, so almost all
of the surface is tested without a transport, a window or a GPU: build a
`World` with the registries and a spawned scene, call the handler, assert on
its output. The `scene_tree` truncation lines, the stale-id path and the
not-scriptable path each get a test, because those are the outputs an agent
acts on.

One end-to-end test runs `rmcp`'s client against the server over an in-memory
duplex: initialise, list tools, call `status`. It pins the schema and catches a
handler that panics instead of replying.

`edits_from_json` for the numeric editors is tested alongside the existing
numeric commit tests, including that `[1, NaN, 3]` changes nothing.

Screenshot is tested by hand until M5, since it needs a GPU; M5's headless mode
makes it testable on software Vulkan.

---

## Section 7 — Milestones

Each leaves the editor working and is shippable alone.

**M1 — Attach and observe.** `crates/editor-mcp`, `--mcp`, `.mcp.json`. Tools:
`status`, `list_assets`, `scene_tree`, `find_entities`, `inspect`,
`open_project`, `open_asset`, `close_editor`, `select`, `frame`, `set_camera`.
Entity ids (4a). After M1 an agent can answer "what is in this scene and where
is it" against the running editor.

**M2 — See.** `TextureReadback` (4c) and `screenshot` of the viewport. This is
the milestone that pays for the rest. **M2b:** `window` capture.

**M3 — Edit.** `PropertyPath::resolve`, `edits_from_json` and `set_property`
(4b).

**M4 — Logs.** A ring-buffer logger installed in front of `env_logger`, and a
`logs` tool (`since`, `level`, `limit`). The `PropertyCommit dropped` warning
is the kind of thing an agent should see without the user copying it over.

**M5 — Headless.** `--headless --mcp-stdio` (4d), so an agent can start its
own editor in CI or a cloud session.

## Open questions

1. **Should `select` move the user's selection?** This design says yes, so the
   agent and the user share a view. The alternative is an agent-only selection
   that the panels ignore, which avoids surprises but means an agent can never
   point at something.
2. **Should MCP edits show in the editor?** They do, since they are live-world
   edits. Should the status line say "edited by agent", or is the changed value
   enough?
3. **Is M5 worth pulling forward?** Cloud sessions — including the one this
   document was written in — have no display, so until M5 the server helps
   only when the agent runs on the same machine as the editor.
