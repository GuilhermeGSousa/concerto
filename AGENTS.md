# Concerto

A Rust game engine for character-driven, cinematic games: a custom ECS, a wgpu
renderer, an offline asset pipeline, and an engine-native editor. Everything is
work in progress; `concerto` 0.1.0 is on crates.io and predates the hosting
work.

The workspace is `crates/*` (29 engine crates, all named `concerto-<dir>`) plus
`examples/*` (five standalone games). The root crate re-exports the engine and
defines `DefaultPlugins`.

## Read when relevant

| File | Read it when |
|---|---|
| [.agents/reference/architecture.md](.agents/reference/architecture.md) | You need the crate map, the frame loop, the two worlds, or the asset pipeline |
| [.agents/reference/gotchas.md](.agents/reference/gotchas.md) | Before touching scheduling, change detection, rendering, skinning, animation, or the editor |
| [docs/scheduling.md](docs/scheduling.md) | Ordering systems, sets, chains |
| [docs/asset-importing.md](docs/asset-importing.md) | Importing sources, `.gasset` files, `asset_id!` |
| [docs/profiling.md](docs/profiling.md) | Measuring performance |
| [crates/editor/README.md](crates/editor/README.md) | Anything in the editor |

Skills in [.agents/skills/](.agents/skills/) cover visual verification, driving
the editor without real input, and running on wasm. Roles in
[.agents/agents/](.agents/agents/) are the spec reviewer, the implementer, and
the whole-branch reviewer. Both are linked into `.claude/`.

## Prerequisites

- The Jolt submodule: `git submodule update --init`. Without it every native
  build fails in `concerto-jolt-ffi`.
- Linux system packages: `libasound2-dev libudev-dev libwayland-dev`.
- The wasm target: `rustup target add wasm32-unknown-unknown`.
- `trunk` to run an example in a browser.

## Commands

```sh
cargo build --workspace
cargo run -p tech-demo
cargo run -p tech-demo-editor -- --project <scratch copy of examples/tech-demo>
cargo run -p concerto-editor --example custom_property
cd examples/tech-demo && cargo run -p concerto-import -- assets/forest.glb
cd examples/tech-demo && trunk serve
cargo install --path crates/project
cargo concerto new|editor <dir> [--engine <path>]
```

Examples are `animation-test`, `physics-test`, `render-test`, `tech-demo`, and
`ui-showcase`. `import` runs from the directory that owns the `content/` tree.
An example may have an `editor/` crate, named `<game>-editor`, that runs the
editor over the game's library; an editor exists only for a game that has one.
Today that is `tech-demo`.

## Done gate

A change that touches code, a manifest, a shader, or CI is done only when all
of these pass, in this order. They are the CI jobs, with CI's flags.

```sh
cargo fmt --all -- --check
cargo clippy -- -A clippy::type_complexity -A clippy::too_many_arguments -D warnings
cargo clippy -p concerto-physics --no-default-features --features force-rapier -- -A clippy::type_complexity -A clippy::too_many_arguments -D warnings
cargo test --workspace
cargo test -p concerto-physics --no-default-features --features force-rapier
cargo build --target wasm32-unknown-unknown
```

Report each command you ran with its result. If one fails, say so and show the
output. A command you did not run is reported as not run. A change to Markdown
alone needs none of them.

Rendering, UI, and editor changes also need a look at the running program; a
passing test suite says nothing about what is on screen. Use the
`visual-verification` skill.

## Code rules

**Comments.** Write none, in any file type, except a one-line `///` on a public
item. That covers Rust, WGSL, TOML, YAML, and shell. Two exceptions: an
`unsafe` block may carry one `// SAFETY:` line saying why it is sound, and an
`unsafe fn` may carry a `# Safety` section in its doc comment. Design reasoning
goes in your reply or in the spec. Leave existing comments alone unless the
code they describe changes.

**Systems never take the world.** A system's parameters are `Query`, `Res`,
`Option<Res<T>>`, `ResMut`, `CommandQueue`, `EventReader`, `EventWriter`, `SystemLocal`, and, in
an extract system, `Extracted`. In the editor crate a system may also take
`EditWorld<T>`, which reads the world of the scene being edited, or of the running
game while playing, and `EditWorldSignals`, which sends signals to it. Both skip
the system while no document is the edit target: no scene is open, or its game
is starting. Nothing
else reaches that world. Do not write a system that takes `&World` or
`&mut World`. Work that needs the whole world goes through commands, so it runs
with exclusive access when the queue is applied: the typed `CommandQueue`
methods (`spawn`, `insert`, `remove`, `despawn`, `insert_resource`), or
`cmd.trigger(signal)` with a listener that does the exclusive work. The only
existing systems that take `&mut World` are `run_main` and `render_main`, which
run the main and render schedules; they are not a pattern to copy. To react
when a component appears or disappears, listen for its lifecycle signals,
`On<Add<T>>`, `On<Replace<T>>`, `On<Remove<T>>` and `On<Despawn<T>>`. A system
whose `Res<T>` or `ResMut<T>` names a missing resource is skipped with a warning
on every run until the resource exists; take `Option<Res<T>>` when absence is
normal.

**No revision counters.** Do not add a number that is bumped to say "this
changed" and compared somewhere else. State that changed is read through change
detection (`Changed<T>`, `has_changed()`). Something that happened is an event.
Something to be done is a signal. If change detection cannot express the case,
say so in the spec and propose the fix to change detection; do not work around
it with a counter.

**Warnings are errors.** `[lints.rust] warnings = "deny"` is set, so an unused
import fails the build.

**Match the code around you.** Naming, module layout, error handling, and test
style follow the file you are in.

**Tests first.** Write the failing test, watch it fail, then make it pass.

## Constraints

- A game crate never depends on `concerto-editor`. The editor hosts the game;
  whatever a game says about editing must be reachable from engine crates.
- Jolt is the native physics backend and Rapier is the web one. Do not make
  Jolt build for wasm.
- Render-world systems are added with `add_render_system`. `add_system` with
  `Extract`, `Render`, or `LateRender` puts the system in the main world, where
  those schedules never run.
- An entity reaches the renderer only if it has `SyncWithRenderWorld`.
- `content/` trees and `<source>.import.toml` files are committed. Never run a
  flow that saves or imports against `examples/*`; copy the project's `content/`
  somewhere disposable first.

## Git

- Commit small, focused changes on the current branch as the work lands. The
  subject is imperative and says what changed: "Run queued imports one at a
  time and refresh on completion".
- Stage the paths you changed by name. Never `git add -A`.
- Never push, never commit to `master`, never rewrite history, never force
  anything. Squashing and rewriting branches is the owner's job.
- The owner edits files while you work. Re-read a file before you edit it, and
  expect the build state to shift between commands.

## Process

1. Anything beyond a small fix starts as a spec at
   `.agents/specs/YYYY-MM-DD-<topic>-design.md`. This path overrides any default
   a skill suggests.
2. The owner approves the spec. Nothing is implemented before that.
3. The owner then picks how it is built:
   - inline, test-first, in the current session; or
   - a plan at `.agents/plans/YYYY-MM-DD-<topic>.md`, one `implementer` per
     task, and a `branch-reviewer` pass over the whole branch at the end.

When a design question has a known answer in Bevy, say how Bevy does it and
what differs here.

While brainstorming a design, before the spec is written:

- Assume nothing. Put every open decision to the owner as a question, and keep
  asking until none is left.
- Aim for the architecture that is easiest to maintain, even when that means
  changing an established public API.

## Documentation

A change in behaviour updates the user-facing document that describes it, in
the same commit: `docs/*.md`, a crate `README.md`, or this file.
