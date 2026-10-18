# Gotchas

Behaviour that is not visible from the API and has cost real debugging time.
Each entry was checked against the code on 2026-10-11. If one no longer holds,
fix the entry in the same commit as the change that made it false.

## ECS

- **`Changed<T>` is frame-exact.** It is true only when the component's changed
  or added tick equals the world's current tick, and the tick advances once per
  frame, at the end of `Main`. A `Changed<T>` filter therefore fires only in a
  system that runs later in the same frame than the write. A system that runs
  earlier in the frame never sees it, on this frame or the next.
- **Transform propagation writes every frame.** `propagate_global_transforms`
  calls `set_matrix` on every hierarchy member each frame, so
  `Changed<GlobalTransform>` does not mean "this entity moved".
- **A command-using system's sync point sits at its registration position,
  not after it in the run order.** The scheduler inserts the sync point next
  to the system in registration order, and orders it against other systems
  only through conflicts, also by registration order
  (`setup_systems_for_compilation` and `add_implicit_edges` in
  `crates/ecs/src/system/schedule.rs`). So when `a` runs before `b` but is
  registered after it, whether the edge is written `a.before(b)` or
  `b.after(a)`, `a`'s commands apply after `b` under the multi-threaded
  executor, the native default. The single-threaded executor,
  used on wasm, applies each system's commands right after it, which hides
  this. To make `b` see `a`'s decision, have `a` change data `b` reads, or
  register `a` before `b`.
- **A missing resource skips the system and warns every frame.** A system
  input declines through `SystemInput::get_data`, which returns a
  `ValidationError`. `Res`, `ResMut`, `EventReader`, `EventWriter` and
  `Extracted` decline as `Missing`, which logs a warning on every skipped run;
  `Editor<T>` and `EditorSignals` decline as `Skip`, which is silent. A warning
  flood in the log names the system and the missing type. Use `Option<Res<T>>`
  where absence is normal.
- **`Added<T>` and `Changed<T>` read `T`.** A query filtered by them conflicts
  with any system writing `T`, so the two never run in parallel. `With<T>` and
  `Without<T>` read only archetypes and do not conflict.
- **`.after()` and `.before()` reference a system; they do not add it.**
  Something else must add the target to the same schedule.
- **A system takes at most twelve parameters.** A thirteenth fails with an
  `IntoSystemConfig` trait-bound error that does not mention the count. Split
  the system or group parameters.
- **A system a `TagFilter` rejects is a silent stand-in.** Nothing is logged. A
  component that only stand-ins would have queried is never registered in that
  world, so its lifecycle callbacks never fire. Likewise, in an edit `App` an
  unmarked listener given to `App::add_listener` is silently dropped.
- **Lifecycle signals run inside the operation.** `On<Add<T>>`, `Replace<T>`,
  `Remove<T>` and `Despawn<T>` listeners run right after `T`'s hook, in the
  middle of the spawn, insert, remove or despawn. They see the new value on add
  and the old value otherwise; their `CommandQueue` applies when the operation's
  commands flush. `add_listener` panics for one that takes `&World` or
  `&mut World`, and registering one registers `T`, so its signals fire even if
  nothing queries it. An overwrite fires `Replace`, then `Add`.
- **A lifecycle callback maintains data and is not game behaviour.** It runs in
  every `App`. Put behaviour in a system or a listener.
- **A plugin's `build` may check `app.is_editing()` only to skip resources that
  its unmarked systems alone use.** Anything else it builds the same way in both
  roles.
- **`SystemLocal<T>` needs `T: FromWorld`,** and its field is private. Give the
  local type a named field and implement `FromWorld` for it.
- **An entity carries one `Listener<S>` per signal type.** Two features that
  both want `UIPointerDown` on the same node must share a listener, or one of
  them polls input instead.
- **Lifecycle hooks fire only where the component is registered.** `on_add` and
  `on_remove` are recorded by `World::register_component`, which queries and
  filters call for the types they name. Spawning or inserting a component does
  not register it. A hook on a component that nothing in that world queries
  never runs.

## Rendering

- **`add_system(Extract | Render | LateRender, …)` is always a bug.** It adds
  the system to the main world, where those schedules never run. Nothing
  panics and nothing draws. Use `add_render_system`.
- **No `SyncWithRenderWorld`, no render-world mirror.** Lights, cameras,
  meshes, and the world grid all need it. A scene that renders black or has no
  grid is usually missing it on a light or on the grid.
- **Extract only inserts and updates.** Removing `Light`, `Camera`, a mesh, a
  material, or a skeleton from a living entity leaves its render-world copy in
  place, so a removed light keeps shining. Mirrors go away only when the main
  entity is despawned. Do not fix this in passing.
- **Skinned meshes use an identity instance matrix.** The bone palette carries
  the world transform, and the shader applies it to the position. Giving a
  skinned mesh entity a real world transform as well transforms it twice.
- **`Pose::identity` has zero translation per joint.** Applying a pose that was
  never evaluated collapses every bone to the origin and the mesh folds onto
  itself. `AnimationPlayer::evaluate` returns early when no graph evaluated;
  keep that guard in any new pose path.
- **Present mode is whatever the surface lists first,** usually vsync. Frame
  rate is not a measure of CPU cost; see [docs/profiling.md](../../docs/profiling.md).

## Scenes and assets

- **A unit-struct scene component must accept `{}`.** serde reads a unit struct
  only from `null`, and authored scene data carries `{}` for a marker. See
  `MeshCollider` in `crates/physics/src/shape.rs`, which reads its payload with
  `deserialize_ignored_any`.
- **Examples share one `target/` directory.** Each copies its `content/` to its
  own `<package>-content/` directory beside the binary. After changing an
  example's sources or content, build that example, not just the workspace.
- **Large scenes take a long time to load in a debug build.** `render-test`
  loads Sponza and needs more than 40 seconds before anything but the clear
  colour and the grid appears. That looks exactly like a broken renderer.

## Editor

- **One listener applies each kind of edit.** Panels trigger a signal such as
  `EntityEdit`, `Select`, or `SetGizmoMode`; they do not mutate the scene, the
  selection, or the gizmo themselves. The listeners live in
  `crates/editor/src/tools/`, which names no document, tab, or panel type.
- **Hosted events arrive in the editor a frame after a signal wrote them.**
  `forward_events::<E>` runs in `First`, after the hosted step. An event written
  during the step reaches `EventReader<Hosted<E>>` systems in the same frame;
  one written by a signal (`EditorSignals`, `hosted::trigger_on`) arrives on the
  next frame. Decisions that must not lag, such as Play's "save first", read
  the edit `App`'s state (`HostedApp::scene_dirty`) instead of an event.
- **`Unsaved` on a Scene document follows the edit `App`'s `SceneDirty`.**
  `follow_unsaved_edits` inserts or removes it each frame while the document is
  `Editing`. Clearing `Unsaved` by hand does not stick; mark the scene saved
  through `HostedApp::mark_saved` after a successful write.
- **`forward_hosted_event::<E>()` adds a system,** so call it before the editor
  `App` finishes building.
- **Change detection does not cross the seam.** `has_changed()` and
  `Changed<T>` compare against the tick of the world they are read in. Through
  `Editor<T>` they give no usable answer; a panel learns of a change from a
  `Hosted<E>` event.
- **A scene id is not an editor entity.** Both are `Entity`. A field in the
  editor's world that holds a scene id is named `scene_…` and is only ever
  looked up through `Editor<…>`.
- **Tools run in the edit `App`.** A system added to the editor `App` never
  sees a scene entity, and a component used only by tools must be registered
  there for its hooks to fire. A tool's system or listener must be marked
  `.at_edit_time()`, or it is a stand-in or is dropped there. A play `App`
  gets only `EditToolsCorePlugin`, the tools that select, edit, and capture,
  so nothing from the viewport half runs in a game. Changes made while
  playing are made to the play `App`'s world and are lost on Stop; they never
  mark the document unsaved.
- **`Editor<T>` and `EditorSignals` skip their system** while the active tab
  has no scene, while its game is starting, and from the moment its scene
  changes until the host announces it in the next `First`. Whatever such a
  system drew stays on screen unless a system in `First`, ordered after
  `announce_edit_target`, empties it when `Res<LastEditTarget>` has changed.
  A skipped system does not advance its event readers either, so on its next
  run it reads the events of the frame it missed.
- **Only the editor camera draws in an edit `App`.** The viewport tools insert
  `ExclusiveCamera` naming it at `Startup`, and `extract_cameras` skips every
  other camera while the resource exists, so a scene camera never gets a render
  camera. Nothing removes one afterwards.
- **The scene root in the hierarchy is the editor's wrapper,** not part of the
  scene. It cannot be deleted, duplicated, or renamed.
- **Entity edits pick their rules from whether the world has a `SceneRoot`.**
  With one, only that scene's entities other than the root can be changed;
  without one, as in a play world, any entity can, and `Create` with no parent
  adds a top-level entity. Only the edit `App`'s `open_scene` may spawn a
  `SceneRoot`, and an edit world always has exactly one.
- **The viewport tools learn about the pointer from one resource.** The fly
  camera, the wheel, picking, and the gizmo read `ViewportPointer`, which
  `fill_pointer_from_window` in `tools/camera.rs` fills at the start of
  `Update` from the edit `App`'s own `Window` and `Input`. Nothing under
  `tools/` imports `concerto_ui`. The host decides what reaches that `Input`:
  the viewport panel publishes `ViewportShown` in `First`, just before the
  step, from the UI's layout and hit test of the frame before, and a press is
  forwarded only while the pointer is over the viewport, so the scene gets no
  press while the pointer is over a menu, a prompt, or a panel. `over` is whether
  the pointer lies on the picture; when the viewport stops being under the
  pointer with no button held, the host moves the edit `App`'s pointer to
  `POINTER_GONE`, off the picture, so a hovered gizmo handle lets go. When the
  viewport is under the pointer again, the host moves it to where the editor's
  pointer is, so a press or a wheel before the next move lands in place.
- **A panic in an edit `App` is caught only while it is built.** A panic in
  the recipe or a plugin's `build`, in a plugin's `ready` or `finish`, in
  `Startup`, or in the ready step that publishes the content and opens the
  scene becomes the open request's error. A panic while the `App` is stepped
  or takes a signal still takes the editor down. Catching a panic from a
  schedule is safe only because `TaskPool::scope` re-raises a task's panic
  after every task it spawned has finished; the panicked system's dependants
  never run.
- **Asynchronous editor work must check its request** before it mutates
  anything: capture the `AssetEntry` it was started for and pass it to
  `asset_request_is_current`. A result for a replaced or cancelled request is
  discarded; a closed tab, and every tab after a project switch, is gone. A
  late result of a request repeated for the same asset is accepted, since it
  read the same file.
- **A closed `InputGate` hides input from that `App`.** Its `Input` sees no
  keys, buttons, wheel, or raw motion, and closing the gate releases whatever
  was held; the cursor position is still folded. The editor closes its own
  gate while a game has input.
- **While a game has input, Esc and every other key go to the game.** The
  editor's UI and `ActionMap` see nothing until input is released by a click
  anywhere but the viewport's picture, Shift+F1, or the window losing focus.
  A press counts as on the picture only if the viewport was the hovered node in
  the frame before and the pointer is inside its rectangle, so the tab strip
  and the panels drawn over the viewport take the click. A shortcut that
  must work while playing cannot be an `ActionMap` binding; the host reads
  Ctrl+P and Shift+F1 from the raw window events in `route_play_input`
  (`hosted/input.rs`). That system decides the gate for the frame, so it runs
  before `concerto_window::FoldInput`; the click that releases input is then
  folded in the same frame and reaches the panel under it. While playing,
  `step_edit_target` routes nothing, including on the frame Stop happens.

## wasm

- **Run with `trunk serve` from the example's directory,** not `cargo run`. The
  `wasm-server-runner` entry in `.cargo/config.toml` points at an `index.html`
  that does not exist at the root.
- **The web target is WebGL2.** wasm pins `wgpu::Backends::GL` and uses
  `downlevel_webgl2` limits. A feature that WebGL2 lacks must be gated through
  `crates/render/src/capabilities.rs`, not requested unconditionally.
- **There are no system fonts on wasm.** The UI embeds DejaVu Sans Mono and
  uses it for every generic family.
- **Assets are fetched over HTTP on wasm,** so `content/` must be served from
  the page's origin. An example's `index.html` does this with a `copy-dir` link.
- **wasm has no threads.** Jolt is not built there; physics is Rapier.
