# Terminal Renderer on Split Worlds — Open Problem

**Status:** open, not designed. Deferred from the system scheduling work
(`2026-09-23-system-scheduling-api-design.md`).
**Touches:** `crates/terminal-renderer`, `examples/render-test` (`terminal` feature),
possibly `crates/app` (extract).

## Problem

`crates/terminal-renderer` was written for a single world and was never ported
when the game and render worlds were split (#59). Terminal mode
(`cargo run -p render-test --features terminal`) does not work on `master`:

1. **Startup panic.** `TerminalRendererPlugin::finish` looks up `RenderDevice`
   with `app.get_resource`, which reads the *main* world. `RenderDevice` is
   inserted into the render subapp, so the `expect` fires.
2. **Readback never runs.** `readback_terminal_frame` is registered with
   `app.add_system(LateRender, ..)`, i.e. in the main app. The main app never
   runs `LateRender` (see `crates/app/src/main_schedule.rs`); only the render
   subapp does (`render_main` in `crates/render/src/plugin.rs`). Its inputs
   (`RenderDevice`, `RenderQueue`, `Query<&RenderCamera>`) are all render-world
   data anyway.
3. **Resize mixes worlds.** `handle_terminal_resize` runs in main `Update` but
   reads `RenderDevice`, `RenderContext` and `RenderCamera` (render world)
   alongside `RenderEntity`/`TerminalOutput` on the main-world camera.
4. **The example draws from the wrong place.** `render-test`'s `draw_terminal`
   is also added to main `LateRender`, so it never runs either.

Since the scheduling change, the main `LateRender` schedule also logs
`... ordered against finish_render, which is not in this schedule; the
constraint is ignored` at startup. That warning is accurate. It should go away
once readback moves to the render subapp; don't silence it.

## What stays in the main world

- `poll_terminal_input` / `TerminalInput`: the runner reads `TerminalInput` from
  the main world to detect Esc (`runner.rs`), and gameplay systems read it.
- `TerminalResizeEvent` is produced there by `poll_terminal_input`.
- The camera entity tagged `TerminalOutput`.

## What belongs in the render world

- `TerminalRenderState` (staging buffer, created from `RenderDevice`).
- `readback_terminal_frame`, in render `LateRender`, in `RenderSet::Present`,
  `.after(finish_render)`. That ordering is already written; only the
  registration needs to move to `add_render_system`.
- Resizing the render target (`RenderCamera::resize_render_target`) and
  recreating `TerminalRenderState`.

## Open questions

1. **Getting the ASCII frame back to main.** Readback produces `TerminalFrame`
   in the render world, but `draw_terminal` / `TerminalContext` live in main.
   Extract only goes main → render, and `Extracted<T>` requires
   `ReadOnlySystemInput`, so the render world cannot write into main. Options:
   - Move `TerminalContext` and drawing into the render world entirely. This is
     the simplest, but user-facing ratatui widgets (the example's title bar)
     would then have to be render-world systems.
   - Share the frame through a resource holding `Arc<Mutex<String>>` inserted
     into both worlds.
   - Add a render → main hand-back step to `crates/app` (a general mechanism;
     bigger change).
2. **Resize.** Extract the latest `TerminalResizeEvent` into the render world
   (an `Extract` system reading `EventReader` via `Extracted`) and handle it
   there, or put the new size on the extracted camera.
3. **Plugin order.** Once `finish` reads from `app.render()`, check that
   `RenderPlugin::finish` has already inserted `RenderDevice` when
   `TerminalRendererPlugin::finish` runs.

## Verification

There's no GPU in cloud sessions, but Mesa's lavapipe works:
`apt-get install -y mesa-vulkan-drivers libvulkan1`. With it,
`DefaultPlugins::headless()` runs real frames. A test only needs `ActionMap` and
`concerto_window::input::Input` inserted by hand, because headless mode skips
`WindowPlugin`. `ratatui::init()` takes over the terminal, so a test harness
should construct the plugin's pieces rather than register
`TerminalRendererPlugin` itself, or put the ratatui backend behind a trait.
