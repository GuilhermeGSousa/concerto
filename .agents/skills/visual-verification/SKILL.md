---
name: visual-verification
description: Use when a change affects what Concerto draws (rendering, shaders, materials, lights, skinning, animation, UI, editor panels) and it must be confirmed in the running window, or when a capture comes back blank, black, stale, or refused by the desktop
---

# Visual verification

A passing test suite says nothing about the picture. Run the program, capture
its window, and read the image before calling a visual change done.

## Steps

1. Build the binary you will look at: `cargo build -p <example>`, or for the
   editor `cargo build -p tech-demo-editor`. The editor is built per game; only
   a game with an `editor/` crate has one.
2. Start it in the background and note the process id.
3. Wait for the scene to load (table below). Do not judge an early frame.
4. Capture the window to a PNG and open the PNG with the Read tool.
5. Stop it with `pgrep -x <binary> | xargs -r kill`. Linux cuts process names
   to 15 characters, so the editor is `pgrep -x tech-demo-edito`.
   `pkill -f tech-demo-editor` also works, but only from a shell whose own
   command line does not contain that string.

Every Concerto window, examples and editor alike, is titled `Concerto`. Run the
editor with `--decorated` so the window manager frames it.

| Binary | Wait before capturing (debug build) |
|---|---|
| `animation-test` | about 15 s; the known-good skinned character |
| `tech-demo` | about 25 s |
| `tech-demo-editor` | about 30 s after the forest scene is opened |
| `render-test`, anything loading Sponza | more than 40 s |

Sponza also needs a bright light: at intensity 10 the atrium is near black, at
about 400 it reads.

## Proving something moves

One frame cannot show animation. Capture two frames a second apart and count
the sub-pixels that differ by more than a small threshold. A static pose
differs by noise only; an advancing clip changes thousands of sub-pixels,
concentrated on the character.

## Stale frames

A capture can return a frame from several seconds earlier when the window is
unfocused. When the state under test changes between shots, have the test
harness recolour a 10×10 swatch in the window's top-left corner for each shot,
and retry the capture until the swatch has the expected colour.

## Flows that need clicks

Use the `editor-selftest` skill. Synthetic input sent from outside the process
is not reliable; input set inside the process is.

## Common mistakes

| Symptom | Cause |
|---|---|
| Only the clear colour and the grid | Captured before the scene loaded |
| Character renders black, no grid | Entity has no camera, light, mesh, material or grid component, so no `SyncWithRenderWorld` |
| Mesh folded into a point | A pose was applied without being evaluated |
| Two identical captures | Stale frame, or the state did not change |
| The capture shows the owner's own clicks | They are using the machine; ask before assuming your input worked |

## Linux with a Wayland compositor that blocks capture

This section applies to COSMIC and to any compositor whose screenshot portal
refuses non-interactive requests. It was worked out on a machine with no
`grim`, `ffmpeg`, or ImageMagick.

- Start the program on XWayland so that X11 can see the window:
  `env -u WAYLAND_DISPLAY DISPLAY=:1 ./target/debug/<binary> &`
- Capture with a small Rust tool built in the scratchpad, never in the repo. It
  uses `x11rb` and `image`: walk the window tree, match `WM_NAME` against
  `Concerto`, call `GetImage` with `ImageFormat::Z_PIXMAP` on the top-level
  window, convert BGRX to RGBA, save a PNG. About 90 lines. A second binary that
  decodes two PNGs and counts differing sub-pixels covers the motion check.
- XTest input sent to that window is dropped. Clicks and key presses have to be
  set in-process.
- `pkill -f <name>` from the same shell command matches the shell itself. Use
  `pgrep -x`.

## Cloud and CI containers

There is no display and no GPU. Do not claim a visual check there. Say that the
change was not looked at, and what the owner should look at.
