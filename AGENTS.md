# Concerto

A Rust game engine for character-driven, cinematic games: a custom ECS, a wgpu
renderer, an offline asset pipeline, and an engine-native editor. Everything is
work in progress; `concerto` 0.1.0 is on crates.io and predates the hosting
work.

The workspace is `crates/*` (the engine crates, all named `concerto-<dir>`)
plus `examples/ui-showcase`. The root crate re-exports the engine and defines
`DefaultPlugins`.

## Workspace

This repository is checked out as `concerto/` inside a `concerto-dev/`
directory, beside the games built on it, each in its own repository:

```
concerto-dev/
  concerto/                 this repository, the engine
  tech-demo/                the tech demo, with its tech-demo-editor
  one-more-night/           One More Night, with its case crate and editor
  concerto.code-workspace   a VS Code workspace over all three, with launch configs
```

Each game depends on the engine by path (`../concerto`), is its own Cargo
workspace, and is built and run from its own directory, for example
`cd ../tech-demo && cargo run`. Each has its own `AGENTS.md` with its commands,
its content and its constraints; this file's rules apply there too. An engine
change that a game shows, such as rendering or animation, is checked by
running that game from its directory.

## Read when relevant

| File | Read it when |
|---|---|
| [.agents/reference/architecture.md](.agents/reference/architecture.md) | You need the crate map, the frame loop, the two worlds, or the asset pipeline |
| [.agents/reference/gotchas.md](.agents/reference/gotchas.md) | Before touching scheduling, change detection, rendering, skinning, animation, or the editor |
| [docs/required-components.md](docs/required-components.md) | Declaring `#[require(..)]`, or a component that must always come with another |
| [docs/scheduling.md](docs/scheduling.md) | Ordering systems, sets, chains |
| [docs/asset-importing.md](docs/asset-importing.md) | Importing sources, `.gasset` files, `asset_id!` |
| [docs/testing.md](docs/testing.md) | Writing, changing, or reviewing any test |
| [docs/profiling.md](docs/profiling.md) | Measuring performance |
| [crates/editor/README.md](crates/editor/README.md) | Anything in the editor |
| [docs/agent-pipeline.md](docs/agent-pipeline.md) | Working on an issue labelled `agent`, or a PR from a `claude/issue-<n>` branch |

Skills in [.agents/skills/](.agents/skills/) cover visual verification, driving
the editor without real input, running on wasm, the issue pipeline, and acting
on events on a pipeline PR (`steward`). Roles in
[.agents/agents/](.agents/agents/) are the spec reviewer, the implementer, and
the whole-branch reviewer. Both are linked into `.claude/`.

## Prerequisites

- The Jolt submodule: `git submodule update --init`. Without it every native
  build fails in `concerto-jolt-ffi`.
- Git LFS: `git lfs install` before cloning, or `git lfs pull` in a clone made
  without it. `.gasset` files are LFS objects, and building a game from their
  pointers fails with `missing the GRDY magic prefix`.
- Linux system packages: `libasound2-dev libudev-dev libwayland-dev`.
- The wasm target: `rustup target add wasm32-unknown-unknown`.
- `trunk` to run a game in a browser.
- `cargo-deny` for the done gate: `cargo install cargo-deny --locked`.

## Commands

```sh
cargo build --workspace
cargo run -p ui-showcase
cargo run -p concerto-editor --example custom_property
cargo install --path crates/project
cargo concerto new|editor <dir> [--engine <path>]
cargo concerto editor <dir> --update [--engine <path>]
cargo concerto build <dir> [--cook] [--release] [--target wasm32-unknown-unknown]
cargo concerto run <dir> [--cook] [--release] [-- <game args>]
cargo concerto build ../tech-demo --cook --release --target wasm32-unknown-unknown --public-url ./ --dist <dir>
cargo run -p concerto-import --bin pack-imports -- <scratch copy of a project>
```

The engine's only example is `ui-showcase`. A game may have an `editor/`
crate, named `<game>-editor`, that runs the editor over the game's library; an
editor exists only for a game that has one. The `custom_property` example in
the commands above runs the editor without a game.

## Done gate

A change that touches code, a manifest, a shader, or CI is done only when all
of these pass, in this order. They are the CI jobs, with CI's flags.

```sh
cargo fmt --all -- --check
python3 .github/scripts/test_budget.py
python3 .github/scripts/dependencies.py
cargo deny check bans advisories
CARGO_BUILD_WARNINGS=deny cargo clippy --locked --workspace --all-targets -- -D warnings
cargo clippy --locked --workspace -- -A clippy::all -D clippy::cognitive_complexity
CARGO_BUILD_WARNINGS=deny cargo clippy --locked -p concerto-physics --no-default-features --features force-rapier -- -D warnings
RUSTDOCFLAGS="-D warnings" CARGO_BUILD_WARNINGS=deny cargo doc --locked --workspace --no-deps
cargo test --locked --workspace
cargo test --locked -p concerto-physics --no-default-features --features force-rapier
cargo build --locked --target wasm32-unknown-unknown
```

CI installs the latest stable Rust, and each clippy release adds lints, so
run `rustup update stable` before the gate; an older local toolchain passes
code that CI's clippy rejects.

`CARGO_BUILD_WARNINGS=deny` makes cargo's own warnings fail clippy, among them
a dependency that no target of its crate uses: remove it, or make it optional
behind the feature that enables the targets using it, as `concerto-benches`
does with `run`.

The cognitive complexity line fails on a function in production code that
scores above `cognitive-complexity-threshold` in `clippy.toml`; tests are not
checked. Split the function. The threshold is a ceiling set at the highest
score in the workspace, lowered as those functions are split and never raised.

`clippy.toml` also holds the two lints the workspace ignores, `too_many_arguments`
and `type_complexity`, as thresholds set too high to reach; the clippy lines
carry no `-A` flags for them.

`cargo deny check bans` fails when the lockfile holds two
versions of one crate. When an upstream crate brings the second version, add
it to `skip` in `deny.toml` with a `reason` naming that crate; delete the entry
once cargo-deny reports it unmatched.

`cargo deny check advisories` fails when a dependency in the lockfile has a
RustSec advisory: a vulnerability, or a crate marked unmaintained or unsound.
Update the dependency (`cargo update -p <crate>`). When no fixed version
exists, add the advisory's id to `ignore` under `[advisories]` in `deny.toml`
with a `reason`, and open an issue for getting off the crate.

A test that needs a GPU skips with a `SKIPPED` line on stderr when there is no adapter or the adapter gives no device; the line says which, with the device error and the adapters wgpu enumerated. The test harness captures it, so it shows only for a failing test or under `--nocapture`. `CONCERTO_REQUIRE_GPU=1` turns the skip into a failure with the same reason. Neither CI runner, Ubuntu or Windows, has an adapter, so GPU tests skip in CI; run them locally on a machine with a GPU.

Test builds carry line tables only and `cargo build` carries full debug info,
so the two compile everything separately, third-party crates and Jolt
included: the first `cargo build` or `cargo run` after `cargo test` takes
minutes, and an edit rebuilds the affected crates once for each. To step through
a test,
`CARGO_PROFILE_TEST_DEBUG=true cargo test -p <crate> --test <name>` gives full
debug info, rebuilding what that test needs.

Both build third-party crates at `opt-level = 3` and the workspace crates at
1, so a plain `cargo run` is playable. A variable can show as optimized out
in the debugger; `--profile profiling` is the fully optimized build, and the
one to measure performance with.

While a plan is in progress, each task runs a gate scoped to the crates it
touched: `cargo fmt --all -- --check`, the test budget and the dependency
layers, then the clippy and test lines above
with `-p <crate>` for each crate in place of `--workspace` (and the Rapier
lines when physics is one of them), and
`RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps -p <crate>` for each
crate. The full gate runs once, after the last
task and before the branch review.

Report each command you ran with its result. If one fails, say so and show the
output. A command you did not run is reported as not run. A change to Markdown
alone needs none of them. A cloud pipeline session runs only the first three, and
the `cargo metadata` step below when a `Cargo.toml` changed, and leaves the
rest to CI. When CI could not start its jobs, for want of Actions minutes, the
session goes on and reports those commands as not run.

A change to a `Cargo.toml` commits the `Cargo.lock` that cargo rewrites. A
session that doesn't build runs `cargo metadata --format-version 1 > /dev/null`,
which resolves and rewrites the lockfile without compiling. A merge conflict in
`Cargo.lock` is resolved by taking `master`'s file and running the same command,
and any merge of `master` that brings in a `Cargo.toml` change runs it too.

Rendering, UI, and editor changes also need a look at the running program; a
passing test suite says nothing about what is on screen. Use the
`visual-verification` skill. A cloud pipeline session has no display: it lists
the checks for the owner in its review comment.

## Code rules

**Comments.** Every public item and every crate root carries a doc comment,
and nothing else carries a comment: `///` on a public item, and `//!` at the
top of each target root (`lib.rs`, `main.rs`, `build.rs`, `src/bin/*`,
`benches/*`, `examples/*`, `tests/*`) and of a public module. The workspace
lints enforce it: `missing_docs` for the items and roots, and clippy's
`missing_errors_doc`, `missing_panics_doc` and `missing_safety_doc` for the
sections below. `concerto-jolt-ffi`, `concerto-benches` and the three
examples are exempt from `missing_docs` and opt out with
`#![allow(missing_docs, reason = "...")]` at each of their target roots; the
other lints still apply to them. Write the docs to the Rust API guidelines and
the rustdoc conventions:

- Open with a one-sentence summary of what the item is or does. Add
  paragraphs only for what the signature and the summary do not say:
  invariants, units, ownership, when to reach for it.
- A function that returns `Result` has an `# Errors` section, one that can
  panic a `# Panics` section, and an `unsafe fn` or `unsafe trait` a
  `# Safety` section saying what the caller must uphold.
- Add an `# Examples` section where usage is not obvious from the signature;
  it is a doctest, so it compiles and runs.
- Link the items you name with intra-doc links: ``[`Query`]``.

Nothing else carries a comment, in any file type: no `//` in Rust, and none in
WGSL, TOML, YAML or shell. The one exception is `unsafe` code: every `unsafe`
block and `unsafe impl` carries one `// SAFETY:` line saying why it is sound.
Design reasoning goes in your reply or in the spec. Leave existing comments
alone unless the code they describe changes.

**Systems never take the world.** A system's parameters are `Query`, `Res`,
`Option<Res<T>>`, `ResMut`, `Option<ResMut<T>>`, `CommandQueue`, `EventReader`, `EventWriter`, `RemovedComponents<T>` (once a plugin calls `App::track_removals::<T>()`), `SystemLocal`,
`InputSet<K>` (the read-only inputs plugins registered under `K` with
`App::register_input`, declared as their union), and, in an extract system,
`Extracted`. In the editor crate a system may also take
`EditWorld<T>`, which reads the world of the scene being edited, or of the running
game while playing, and `EditWorldSignals`, which sends signals to it. Both skip
the system while no document is the edit target: no scene is open, a preview is
in front, or its game is starting. Nothing
else reaches that world. An editor system may also take `PreviewSignals`, which
sends signals to the preview `App` of a document it names. Do not write a system that takes `&World` or
`&mut World`. Work that needs the whole world goes through commands, so it runs
with exclusive access when the queue is applied: the typed `CommandQueue`
methods (`spawn`, `insert`, `remove`, `despawn`, `insert_resource`), or
`cmd.trigger(signal)` with a listener that does the exclusive work. The only
existing systems that take `&mut World` are `run_main` and `render_main`, which
run the main and render schedules; they are not a pattern to copy. To react
when a component appears or disappears, listen for its lifecycle signals,
`On<Add<T>>`, `On<Replace<T>>`, `On<Remove<T>>` and `On<Despawn<T>>`. A system
whose `Res<T>` or `ResMut<T>` names a missing resource is skipped with a warning
on every run until the resource exists; take `Option<Res<T>>` or
`Option<ResMut<T>>` when absence is normal. A system whose own parameters alias
panics when it is initialized, naming itself and the conflict: two parameters
on one resource where one is `ResMut`, one query that writes a component and
names it again, or two queries on one component where one writes it (a
`Changed<T>` or `Added<T>` filter reads `T`). Two such queries are accepted
when one requires a component the other excludes with `Without`.

**Listeners carry their own data.** What a listener needs to know about the
widget it sits on is captured by its closure, not put in a component that the
listener reads back through `on.entity()`. A function that builds one returns
`Listener<S>` from an inline closure with its parameter types written out:
`fn pick_asset<T>(id: AssetId) -> Listener<UIClick> { (move |on: On<UIClick>,
mut commits: ResMut<PropertyCommits>| { … }).into_listener() }`, attached with
`.on_click(pick_asset::<T>(id))`. Returning `impl FnMut(…)` instead does not
compile as a listener. A component stays when something besides its listener
reads it.

**Decode reflected data once, at load.** An asset holding trait objects by
type name builds them in `on_load` into a `Stored`/`Live` data enum; runtime
code reads the live values and never takes a `TypeRegistry`. See Assets in
[architecture.md](.agents/reference/architecture.md).

**No revision counters.** Do not add a number that is bumped to say "this
changed" and compared somewhere else. State that changed is read through change
detection (`Changed<T>`, `is_changed()`). Something that happened is an event.
Something to be done is a signal. If change detection cannot express the case,
say so in the spec and propose the fix to change detection; do not work around
it with a counter.

**Warnings are errors.** `[workspace.lints.rust]` sets `warnings` and
`missing_docs` to `deny`, `[workspace.lints.clippy]` denies
`missing_errors_doc`, `missing_panics_doc`, `missing_safety_doc` and
`undocumented_unsafe_blocks`, and every package inherits both tables with
`[lints] workspace = true`, so an unused import or an undocumented public item
fails the build of any crate. A new crate's `Cargo.toml` carries those two
lines, and each of its target roots starts with a `//!` line.

**Match the code around you.** Naming, module layout, error handling, and test
style follow the file you are in.

**No tuples as types.** Don't use a tuple such as `(u32, f32)` as a field
type, a collection element, a return type or a parameter, in tests too. Name
each field in a struct, and give it methods where they read better.

**Builders for wide structs.** A public struct with many public fields that
users set, such as a graph node's settings, gets `new(..)` for what it can't
default and a consuming `with_<field>(mut self, value) -> Self` per field, as
`UINode` has, so users change a few defaults without a struct literal. A
collection field gets `with_<item>` that adds one.

**Tests first.** Write the failing test, watch it fail, then make it pass. A
cloud pipeline session writes the test first but cannot run it, so the red step
is not observed.

**Few tests, on boundaries.** Follow [docs/testing.md](docs/testing.md). A test
fails only when a promise breaks: a public API, an encoding, an MCP tool, a
signal or event, an ECS invariant. It never pins how the code is built, so a
refactor that keeps behaviour keeps every test green. Test a feature with one
scenario that drives its signals or MCP tool and asserts each outcome; add an
assertion to the scenario before adding a test. Edge cases go in one
table-driven test, and a law (round trip, incremental equals fresh) in one
`proptest` property, never in a family of near-identical tests. Unit tests of
private functions are for maths only. Do not test a getter, a constructor, a
`Default`, a derive, a constant, a label's formatting, flag parsing, or what a
dependency does on its own. Performance guarantees are benchmarks, and no
production type carries a `#[cfg(test)]` field, counter or getter for a test to
read; a test-only setter may reach a state no public path can.
Engine maths and a value's persisted encoding stay however simple they look.
Being the only cover of a behaviour is not enough on its own: a test stays only
if a break it catches would reach a user, a game or an agent. When in doubt,
delete it. `test-budget.toml` caps each package's test count: a change that
deletes tests lowers it, and one that adds tests raises it and says why. It is
a ceiling the owner reviews, not a revision counter; see the budget section of
[docs/testing.md](docs/testing.md).

## Editor rules

These bind every change to `concerto-editor`, its extension crates, and the
editor's MCP tools.

- **Agents reach every feature.** A new editor feature is reachable over MCP in
  the same change, through an existing tool or a new one, without clicking at
  positions read off a screenshot. The panel and the tool run the same code, so
  they cannot drift apart. The Agents (MCP) section of
  [crates/editor/README.md](crates/editor/README.md) describes it.
- **Agents act on meaning, not on pixels.** An MCP tool names what it changes
  (an entity, a property path, an asset, a document) and does the work by
  signal, never by clicking, dragging or typing into the editor's own UI. Having
  to drive panels to reach a feature is a missing tool. `send_input` exists
  only to play the game; it is not a way to operate the editor.
- **Tests go through the same seam.** An editor feature is tested by its
  signals or MCP tool, asserting on the scene, the saved files or the reply.
  Widget tests cover only focus, drag, text entry, hit testing and scrolling,
  which no tool expresses.
- **Modular, like the engine.** Each feature is a plugin that brings its
  panels, systems, signals and MCP tools, and the editor works without it.
  Features talk through components, events and signals, not through each
  other's internals. The extension crates, such as
  `concerto-animation-editor`, are the model.
- **Simple and functional first.** Build the smallest thing that does the job
  for a person and for an agent alike, and make it work before making it
  elaborate. A panel shows what it holds and what it does in plain labels, with
  defaults that need no setup. A tool takes few parameters, defaults the rest,
  answers in short text an agent can read without a screenshot, and says in its
  errors what to do next.
- **Built for reuse.** An element another panel or extension crate could use is
  a public widget with its own plugin, configured by its owner through
  components and signals, as the inspector's `PropertyEditor<T>` and the
  `Timeline` are. Look for an existing element to extend before writing a new
  one.

## Constraints

- A game crate never depends on `concerto-editor`. The editor hosts the game;
  whatever a game says about editing must be reachable from engine crates.
- Each workspace package sits in a layer and depends only on the layers it is
  allowed, in normal, build and dev dependencies alike.
  `python3 .github/scripts/dependencies.py` checks it.

  | Layer | Packages | May depend on |
  |---|---|---|
  | engine | every package not listed below, the root `concerto` included | engine |
  | gizmos | `concerto-*-gizmos` | engine, gizmos |
  | editor | `concerto-editor`, `concerto-*-editor` | engine, gizmos, editor |
  | example | packages under `examples/` | engine, gizmos |
- Jolt is the native physics backend and Rapier is the web one. Do not make
  Jolt build for wasm. `python3 .github/scripts/dependencies.py` fails when the
  root `concerto` crate reaches `concerto-jolt-ffi` on `wasm32-unknown-unknown`.
- Render-world systems are added with `add_render_system`. `add_system` with
  `Extract`, `Render`, or `LateRender` puts the system in the main world, where
  those schedules never run.
- An entity reaches the renderer only if it has `SyncWithRenderWorld`. Camera,
  light, mesh, material and UI layout components require it; a new extracted
  component declares `#[require(SyncWithRenderWorld)]`.
- `content/` trees and `<source>.import.toml` files are committed. Never run a
  flow that saves, imports or converts against `examples/*` or the sibling game
  repositories `../tech-demo` and `../one-more-night`; copy the project's
  `content/` somewhere disposable first.

## Git

- Commit small, focused changes on the current branch as the work lands. The
  subject is imperative and says what changed: "Run queued imports one at a
  time and refresh on completion".
- Stage the paths you changed by name. Never `git add -A`.
- Never push, never commit to `master`, never rewrite history, never force
  anything. Squashing and rewriting branches is the owner's job. The one
  exception: a pipeline session pushes its `claude/issue-<n>` branch and opens
  its PR.
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

An issue labelled `agent` follows the issue pipeline instead. With
`agent:spec`, questions and spec happen in its PR and the owner approves the
spec with a `/approve` comment; the agent then writes its plan and implements it
without waiting. Without the label, the agent goes straight to an
implementation PR. See [docs/agent-pipeline.md](docs/agent-pipeline.md).

When a design question has a known answer in Bevy, say how Bevy does it and
what differs here.

While brainstorming a design, before the spec is written:

- Assume nothing. Put every open decision to the owner as a question, and keep
  asking until none is left.
- Aim for the architecture that is easiest to maintain, even when that means
  changing an established public API.

## Issues

Work found outside the task at hand, and follow-up work agreed with the owner
in a session, becomes a GitHub issue. Open it without asking. Do not do that
work in the current task.

- Search open issues first; if one covers it, comment there instead.
- Start the body with the line `<!-- concerto-agent -->`. Then say what is
  wrong or missing, where (`path:line`), the evidence, a size estimate, and
  where it was found (the PR, or the session link).
- Label it `agent` and `agent:filed`, plus `documentation` or `lint` when the
  change is only that, `agent:local` when it needs a look at the running
  program, and `agent:spec` only for a large task that changes foundational
  work: the ECS, scheduling, the frame loop, the render architecture, the asset
  pipeline, or a public API that several crates or games build on. An open
  decision on anything smaller goes in the body as a question for the owner,
  without the label.
- Only the top-level session files. The `implementer`, `spec-reviewer` and
  `branch-reviewer` roles report such findings to their caller.

## Documentation

A change in behaviour updates the user-facing document that describes it, in
the same commit: `docs/*.md`, a crate `README.md`, or this file.
