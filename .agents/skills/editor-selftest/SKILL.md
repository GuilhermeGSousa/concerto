---
name: editor-selftest
description: Use when an editor or UI flow in Concerto needs clicks, drags, typing, or a sequence of steps to reach the state under test, and no person is driving it - opening a scene, focusing a text field, dragging a gizmo, confirming a dialog, saving
---

# Editor selftest

Drive the program from inside itself with a temporary, environment-gated
plugin, capture each stage, then delete the plugin. Nothing from the selftest is
ever committed.

## Shape

- Gate on one variable: `if std::env::var_os("CONCERTO_SELFTEST").is_some()`
  registers the plugin. Without the variable the program is unchanged.
- Register it inside the editor's own recipe in `crates/editor/src/run.rs`,
  after `UIPlugin`, and put the module under the editor crate (for example
  `src/inspector/selftest.rs`, which can reach that module's private
  components). The generated `editor/src/main.rs` cannot register it: it only
  passes the game's recipe, which builds each edit `App`, not the editor's.
- Run it against tech-demo's editor: `cargo build -p tech-demo-editor`, then
  `./target/debug/tech-demo-editor --decorated --project <scratch copy>`.
- One `Update` system steps a small stage machine held in a resource. Each
  stage waits on an `Instant`, does one thing, and prints a marker:
  `eprintln!("SELFTEST <stage>")`.
- The shell side waits for each marker in the log, waits about two seconds more,
  then captures the window with the `visual-verification` skill.

## Two ways to act

| Goal | Do this |
|---|---|
| Reach a state quickly | Push the command a click would produce, such as `EditorCommand::OpenAsset(id)` onto `EditorCommands`, or trigger the signal a panel would trigger |
| Test the input path itself | Push `WindowEvent`s with an `EventWriter<WindowEvent>` from the selftest's `Update` system: `CursorMoved { position }` in physical pixels (logical × scale factor), `MouseButton { button, pressed: true }` on a later frame than the move, then `pressed: false` on a later frame still, `Key { key, pressed, text }` for typing, `MouseWheel { delta }` for the wheel |

Pushed events are folded into `Input` at the start of the next frame, then go
through the real hit test, focus update, and signal listeners, so they exercise
picking, gizmo drags, and text fields exactly as a pointer does. Pushing events
is also the only way to simulate the wheel, a resize, or a close request. The
viewport learns where the pointer is from the hit test of the frame before, so
a press pushed in the same frame as the move that brings the pointer onto the
viewport is not forwarded to the scene. Reading
`Res<Window>` in a probe shows what the program asked of its window: cursor
mode, maximised, size.

## Driving Play

- **Press Play as a pointer would.** Find the `EditorTabPlay` button's
  `UILayout`, click its centre (logical × scale factor), and watch the
  document's `PlayState`. The play `App` is built inside the Play press: the
  editor pauses briefly, and the state and the button go straight to
  `Playing` and "Stop". Do not try to capture "Starting…"; it is not on screen.
  Ctrl+P is `ControlLeft` then `KeyP` pushed as `Key` events, one per frame.
- **Who has input.** Once `Playing`, the document's `InputOwner` is `Game`:
  every pushed `Key`, button, and wheel event reaches the play `App`, and the
  editor's UI sees none. A focus loss or a stray click anywhere but the
  viewport's picture, the tab strip and the panels over it included, hands
  input back to the editor and lands on what was clicked; before each key the
  game must see, check `InputOwner` and, if it is `Editor`, push a click on the
  viewport's centre (`ViewportShown`'s origin plus half its size, already
  physical). Shift+F1 is `ShiftLeft` then `F1`.
- **Reaching the game's world.** While playing, `Editor<T>` reads the play
  `App`'s world and `EditorSignals` triggers in it, so the selftest finds and
  selects game entities the same way it does scene entities. A game's own
  marker types are not reachable from the editor crate; find its entities by
  engine components (`Transform`, `Children`, `Name`), and remember that
  `SceneSpawnerComponent` is removed once its scene has spawned.
- **A log line inside the game.** To prove a key reached the game, add a
  selftest-only system to the play `App` from the `extra` closure in
  `scene.rs`'s `begin_play`, gated on `CONCERTO_SELFTEST`, and delete it with
  the rest.
- **Memory.** Read `VmRSS` from `/proc/self/status` in the selftest at each
  stage, after the state has settled for several seconds.

## Seeing what happened

Add a `LateUpdate` probe that prints only when a value changes: the focused
widget, the hovered node, the set of widget entity ids. Entity ids that change
between frames mean rows are being respawned, which is how a lost-focus bug
shows up.

## Rules

- **Never point a saving or importing flow at `examples/*`.** Copy
  `examples/tech-demo/content` to `<scratch>/td/content` and pass
  `--project <scratch>/td`: `--project` takes the directory that contains
  `content/`, and given the `content` directory itself the editor opens an
  empty project without an error.
- **Expect the owner's pointer.** Real pointer input reaches the window, and a
  stray click changes the selection. Have the selftest re-trigger `Select` for
  the entity it wants whenever `Selection` differs, send `CursorMoved` on every
  frame of a click, and use the swatch from `visual-verification` so that each
  capture is known to be current.
- **Never open the native file picker.** It appears on the owner's desktop and
  blocks. Call `accept_picked` with the paths instead.
- **Split a system that needs more than twelve parameters** into two systems
  that share the selftest resource.
- The selftest systems are throwaway and follow the same rule as all systems:
  no `&World` or `&mut World` parameters.

## Before finishing

1. Delete the selftest module and the line that registers it.
2. `grep -rn SELFTEST crates examples src` returns nothing.
3. `git status` shows no selftest file and no change under `examples/*/content`.
