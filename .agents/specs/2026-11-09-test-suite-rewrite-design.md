# Test suite rewrite

## Problem

The workspace has 4,055 `#[test]` functions: 2,864 unit tests in `src/`, one
lib-test binary per crate, and 1,191 in 138 integration-test files, each its
own binary linked against the whole engine. The editor alone has 1,000 (862 in
`src/`). The suite slows CI and the local done gate (issue #729), breaks on
refactors that keep behaviour, fills PRs with test noise, and makes a change by
hand cost a rewrite of dozens of tests. The owner considers that unacceptable.

What makes it large, from a survey on 2026-11-09:

- **One test per edge case.** `crates/animation/src/retarget.rs` has 91 tests,
  about 30 of them one per validation error. Sixteen tests elsewhere are named
  `*_is_rejected`.
- **Families that differ in one input.** `crates/ui/src/node.rs` has ten
  `*_lays_out_as_a_fresh_engine_would` tests, one per kind of edit, which are
  one property.
- **Tests that read internals.** 23 `#[cfg(test)]` items sit on production
  code, listed under Work 3. The worst are two counters in `crates/ui/src/node.rs`:
  `UITextMeasure::measured` (`:499`, pushed at `:517`) and
  `UILayoutEngine::visited` (`:613`, bumped at `:1149`). Tests count layout
  passes and shaped labels through them, so any change to how layout is cached
  breaks them.
- **Editor tests that assert on widgets.** Feature tests check which row and
  card entities survive an update, row counts and `UIText` strings
  (`crates/editor/src/inspector/tests.rs`, 3,741 lines), and reach states by
  clicking, most of all in `ui/src/text_input.rs`, `editor/src/content.rs`,
  `editor/src/hierarchy.rs`, `editor/src/timeline.rs`,
  `animation-editor/src/transport.rs` and
  `motion-matching-editor/src/panels/tests.rs`.
- **Trivia.** Flag defaults (`editor/src/run.rs:217`), label formatting
  (`inspector/tests.rs:575`), `Name::default()` (`name/tests/name.rs:33`).
- **Process.** The branch reviewer made "a requirement with code and no test"
  a finding, so every spec sentence got its own test.

## Rules

`docs/testing.md` states them, and `AGENTS.md`, the implementer, the spec
reviewer and the branch reviewer now point at it. This spec applies those rules
to the code that predates them.

## Work

All crates in one branch.

### 1. Infrastructure

- `proptest` as a workspace dev-dependency.
- Benchmarks use `criterion` and never run or build in CI; the owner runs them
  now and then. A dev-dependency would be compiled by every `cargo test` and
  `cargo clippy --all-targets`, so they live in one workspace crate,
  `crates/benches` (`concerto-benches`), with `criterion` as an optional
  dependency behind a `run` feature and every `[[bench]]` declaring
  `required-features = ["run"]`. The done gate then builds the crate without
  `criterion` and skips its benches. They are run with
  `cargo bench -p concerto-benches --features run`. They reach the engine only
  through public APIs, so a guarantee that a `pub(crate)` counter showed
  becomes a timing: "an idle frame of 1,000 labels" rather than "zero layout
  passes".
- One integration-test binary per crate: `tests/<crate>/main.rs` with a `mod`
  per current file, as issue #729 proposes (its option 2). That turns 138
  binaries into about 29, one per crate with a `tests/` directory plus the
  root crate. The other options in #729 stay with that issue. Relative
  includes move one directory deeper: `editor/tests/fonts.rs:8-11` and
  `render/tests/webgl2_shaders.rs:59,71,80`.
- One fixture module per crate, as `crates/physics/tests/common/mod.rs`
  already is. Copies of a fixture merge into it.

### 2. Per-crate dispositions

Every test gets one disposition against `docs/testing.md`: keep, merge into a
scenario, merge into a table, replace with a property, move to a benchmark, or
delete.

- **`ui`.** The fresh-engine family becomes one `proptest` property over
  random edit sequences. The tests that count layout passes, quad rebuilds,
  bindings and text layers become benchmarks. Text input keeps its focus,
  caret, selection and typing tests as widget tests. `tests/elements.rs` and
  `tests/anchored_panel.rs` keep what they cover of placement and hit testing.
- **`editor`, `animation-editor`, `motion-matching-editor`,
  `motion-matching-gizmos`, `navigation-gizmos`, `procedural-animation-gizmos`.**
  Each feature gets one scenario that drives its signals, or its MCP tool in
  `editor/tests/mcp.rs`, and asserts on the scene, the files and the reply.
  Widget-structure assertions go wherever a scenario covers the feature.
  Widget tests stay for focus, drag, text entry, hit testing and scrolling.
  Features with no signal yet keep their click tests until their issues land
  (see Out of scope).
- **`animation`, `procedural-animation`, `motion-matching`, `navigation`,
  `bvh`, `mesh`, `noise`, `color`.** Maths and algorithms. Retarget validation
  errors become one table. Families of samples and edge cases become tables.
  Laws (round trips, symmetry, an output in range) become properties.
- **`ecs`, `reflect`, `asset-format`, `foundation`, `scene`, and the
  `tests/encoding` fixtures of seven crates.** Invariants. They mostly stay.
  Families still become tables or properties: change ticks, hierarchy,
  scheduling constraints.
- **`import`, `asset-import`, `asset-build`, `gltf`, `obj`.** One table of
  inputs and outcomes per importer, plus a scenario per pipeline step.
- **`project`.** It is a scaffolder. Each command (`new`, `editor`, `build`,
  `run`) gets a scenario over a temporary directory, including rollback on
  failure. The refusals of options that do not apply are user-facing errors
  and become one table. Tests that only check a flag's default go.
- **`app`, `window`, `winit`, `tasks`, `name`, `mcp`, `physics`, `gameplay`,
  `director`, `render` (its non-GPU tests), `terminal-renderer`,
  `world-grid`, `debug-gizmos`, the examples.** The general rules. Each
  plugin keeps one scenario per feature, and trivia goes. Physics keeps its
  backend parity tests on both Jolt and Rapier.
- **Root crate.** `src/lib.rs` and `tests/headless.rs` follow the general
  rules. `tests/app_build_cost.rs`, an ignored timing run by hand, becomes a
  benchmark.
- **GPU tests.** The tests that call `Gpu::request_for_test`
  (`render/src/gpu.rs:101`) keep their assertions, in whatever file they end
  up in. They live in 20 files across render, ui, editor, mcp and
  terminal-renderer, two of which are also rewritten here
  (`editor/tests/mcp.rs`, `ui/src/node.rs`).

### 3. Test-only items on production code

Per `docs/testing.md`, a test-only setter that puts code in a state no public
path reaches in a test may stay. Test-only fields, counters and getters of
internal state go, and the tests that read them assert outcomes instead or
move to a benchmark.

| Item | Disposition |
|---|---|
| `ui/src/node.rs:499,507,517` `UITextMeasure::measured` | remove; its tests become benchmarks |
| `ui/src/node.rs:613,637,1149` `UILayoutEngine::visited` | remove; its tests become benchmarks |
| `editor/src/hierarchy.rs:79` `pending_scene_reveal`, `:84` `items` | remove; assert on what the hierarchy shows |
| `foundation/src/assets/asset_server.rs:392` `is_loaded`, `:397` `resolve_by_id` | remove where a public load state answers the same question, else keep |
| `editor/src/inspector/registry.rs:242` `collect_component` | review; keep only if it is a setup path |
| `editor/src/inspector/mod.rs:334` `row_systems` | review; goes with the widget-structure tests |
| `animation/src/graph.rs:748` `made_live` | review |
| `animation/src/clip.rs:724` `build_delta_against`, `:1052` `impl AnimationClip` | review |
| `animation-editor/src/rig/preview.rs:198`, `:228` | review |
| `ecs/src/world.rs:564` `set_change_tick` | keep: no public path reaches the tick-check threshold |
| `editor/src/project.rs:117` `begin_job_for_test` | keep if no public path holds a job open |
| `editor/src/hosted/mod.rs:390` `warm_up_now`, `:408` `warming_up` | keep `warm_up_now` as a setter; review `warming_up` |
| `editor/src/tools/camera.rs:88` `set_looking`, `tools/gizmo.rs:111` `set_captured` | keep as setters |

A "review" item stays only if it is a setter as defined above.

### 4. Documentation

- `docs/testing.md` gains the commands to run the benchmarks.
- `docs/profiling.md` points at them.
- Documents that name test files by path follow the moves:
  `docs/asset-importing.md:356-362`, `crates/editor/README.md:1105`,
  `docs/book/07-physics-animation-gameplay.md:580`.

## Acceptance

- Every test that remains passes `docs/testing.md`.
- Every item in the table of Work 3 has its stated disposition.
- For every deleted test, the branch reviewer names the test or benchmark
  that still covers its behaviour, or agrees the behaviour was trivia.
- Measured before and after on the same machine, reported in the PR: the test
  count, the number of test binaries, and the wall time of a cold and a warm
  `cargo test --locked --workspace`.
- The done gate passes.

## Out of scope

The content browser's New, rename and folder actions have no signal and no MCP
tool, so their tests click. Adding them is issue #733, and the
import tool is issue #726. Their click tests stay until those land.

## Decisions

- Benchmarks: `criterion`, not in CI, run by hand.
- Nothing measures lost coverage. Review is the check: the branch reviewer
  reads every deleted test and names what still covers its behaviour.
- No target size. The suite is whatever the rules leave; the PR reports the
  before and after counts.
