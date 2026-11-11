# Test suite rewrite: plan

Spec: `.agents/specs/2026-11-09-test-suite-rewrite-design.md`. Rules:
`docs/testing.md`. Branch: `claude/gallant-edison-uoal8k`.

## For every task

- Read `AGENTS.md`, `docs/testing.md` and the spec before touching a file.
- Give every test in the task's crates one disposition: keep, merge into a
  scenario, merge into a table, replace with a property, move to a benchmark,
  or delete. Production behaviour does not change. A test rewrite needs no red
  step of its own, but for each new table or property, break the behaviour it
  guards with a one-line edit, watch it fail, and revert the edit before
  committing.
- A deleted test is listed in the report under `DELETED:` with the test or
  benchmark that still covers it, or `trivia`. The branch reviewer checks this
  list; nothing else measures coverage.
- Fixtures: one module per crate (`tests/<crate>/support.rs`, or a `testing`
  module for unit tests).
- Gate per task, for the task's crates only:
  `cargo fmt --all -- --check`,
  `cargo clippy --locked -p <crate>... --all-targets -- -A clippy::type_complexity -A clippy::too_many_arguments -D warnings`,
  `cargo test --locked -p <crate>...`. Physics adds the Rapier lines of the
  done gate. The full done gate runs once, in task 13.
- Before and after counts of `#[test]` for the task's crates go in the report.
- Commit per crate or per coherent step. Never push.

## Tasks

### 0. Baseline (session, not an implementer)

Count `#[test]` functions and test binaries, and time a cold and a warm
`cargo test --locked --workspace` on this machine. Record the numbers here.

### 1. Infrastructure

- `proptest` in `[workspace.dependencies]`, added as a dev-dependency only to
  crates that use it, in their own tasks.
- `crates/benches` (`concerto-benches`): `criterion` optional behind a `run`
  feature, every `[[bench]]` with `required-features = ["run"]`. Its first
  bench is the timing in the root crate's `tests/app_build_cost.rs` (building
  and stepping a headless app), which is then deleted.
- `docs/testing.md`: the bench command,
  `cargo bench -p concerto-benches --features run`. `docs/profiling.md`: a
  line pointing at it.
- Commit `Cargo.lock`.

### 2. One integration-test binary per crate

For every crate with a `tests/` directory, and the root crate: move each
`tests/<file>.rs` to `tests/<crate>/<file>.rs` as a module of
`tests/<crate>/main.rs`. Keep `tests/encoding/` fixtures where they are and fix
their paths. Fix relative includes (`editor/tests/fonts.rs:8-11`,
`render/tests/webgl2_shaders.rs:59,71,80`) and the documents that name test
files (`docs/asset-importing.md:356-362`, `crates/editor/README.md:1105`,
`docs/book/07-physics-animation-gameplay.md:580`, any other hit of
`grep -rn 'tests/[a-z_]*\.rs' docs crates/*/README.md AGENTS.md .agents`).
No test changes. Gate: the whole workspace's `cargo test --locked --no-run`
plus fmt and clippy.

### 3. `ecs`

Change ticks, hierarchy, scheduling constraints and required components:
families become tables or properties. `World::set_change_tick` stays.

### 4. `reflect`, `asset-format`, `foundation`, `scene`, `name`, `tasks`, `app`, `window`, `winit`

Invariants and encodings stay. Trivia goes. `asset_server.rs:392,397` per the
spec's table.

### 5. `ui`

- The ten `*_lays_out_as_a_fresh_engine_would` tests become one `proptest`
  property over random edit sequences.
- Tests that count layout passes, measured labels, quad rebuilds, bindings
  and prepared text layers become benchmarks in `concerto-benches`, over the
  public API. Remove `UITextMeasure::measured` and `UILayoutEngine::visited`
  (`node.rs:499,507,517,613,637,1149`).
- Text input keeps focus, caret, selection and typing as widget tests.
  `tests/elements` and `tests/anchored_panel` keep placement and hit testing.

### 6. Rendering and import crates

`render` (non-GPU tests; GPU tests keep their assertions), `terminal-renderer`,
`world-grid`, `debug-gizmos`, `mesh`, `color`, `noise`, `bvh`, `obj`, `gltf`,
`import`, `asset-import`, `asset-build`. Importers: one table of inputs and
outcomes each, plus a scenario per pipeline step.

### 7. `animation`

Retarget validation errors become one table. Retarget maths, sampling and
graph evaluation stay, as tables where they are families. `graph.rs:748`,
`clip.rs:724,1052` per the spec's table.

### 8. `procedural-animation`, `motion-matching`

Maths and algorithms: tables and properties.

### 9. `navigation`, `physics`, `gameplay`, `director`, `mcp`, `project`

Physics keeps backend parity on Jolt and Rapier. `project`: a scenario per
command over a temporary directory, rollback included; option refusals become
one table; flag-default tests go.

### 10. Editor: inspector and pickers

`inspector/`, `asset_picker/`, `entity_picker`, `joint_picker/`. Feature tests
become scenarios over `CommitProperty`, `SetValue`, `EditComponent` and the
`set_property`, `add_component`, `remove_component`, `list_push`,
`list_remove` and `inspect` tools, asserting on the scene and the reply.
Widget-structure assertions go. Widget tests stay for focus, drag and text
entry. `inspector/registry.rs:242`, `inspector/mod.rs:334` per the spec's
table.

### 11. Editor: the rest of `concerto-editor`

`hosted/`, `scene`, `preview`, `project`, `run`, `content`, `hierarchy`,
`import/`, `tools/`, `timeline`, `tests/editor/*` (the MCP tests included).
Scenarios over signals and MCP tools. The Asset Browser's New flow keeps its
click tests until issue #733 lands; the import dialog's until #726.
`hierarchy.rs:79,84` removed; `project.rs:117`, `hosted/mod.rs:390,408`,
`tools/camera.rs:88`, `tools/gizmo.rs:111` per the spec's table.

### 12. Editor extensions, gizmo crates, examples, root crate

`animation-editor`, `motion-matching-editor`, `motion-matching-gizmos`,
`navigation-gizmos`, `procedural-animation-gizmos`, `examples/*`, root
`src/lib.rs` and `tests/headless`. `rig/preview.rs:198,228` per the spec's
table.

### 13. Full gate and measurement (session)

Run the full done gate. Repeat the task 0 measurements and record them here
and in the PR description.

### 14. Branch review

`branch-reviewer` over the whole branch, with every task's `DELETED:` list.

### 15. Clean up (session)

Once the review's findings are dealt with, delete
`.agents/specs/2026-11-09-test-suite-rewrite-design.md` from the branch, as
the owner asked. Ask the owner whether this plan and its reports file go too.

## Baseline

Measured 2026-11-09 on the cloud container (4 cores, 15 GB RAM):

- `#[test]` functions: 4,055 (2,870 in `src/`, 1,185 in 138 integration-test
  files).
- Executables in `target/debug/deps` after `cargo test --workspace`: 235.
- `cargo test --locked --workspace`, cold: 882 s. Warm: 41 s. 4,066 passed,
  0 failed, 12 ignored.
- `target/` after the run: 25 GB, 5.8 GB of it incremental state.

Disk is tight in this container: implementers build with
`CARGO_INCREMENTAL=0`.

## Status after the first pass (2026-11-10)

The owner stopped the first pass during task 8. The second pass resumes here.

- **Done:** tasks 0-7, each with its report in
  `2026-11-09-test-suite-rewrite-reports.md`.
- **Task 8, partly done:** two commits and no report.
  - `92d0cc1`: `procedural-animation`, 136 tests down to 50. It looks finished
    but has no `DELETED:` list.
  - `895f952`: in `motion-matching`, `search.rs`, `simulation.rs`,
    `tests/motion-matching/filter.rs` and `tests/motion-matching/staleness.rs`
    were rewritten. The crate is at 241 of its 272 tests.
  - The rest of `motion-matching` is not started; work on `displacement.rs`
    and `systems.rs` was discarded mid-edit.
  - The second pass finishes `motion-matching`, then writes the task 8 report
    for both commits. It names what covers each test deleted in `92d0cc1` and
    `895f952` (`git show <commit>` lists them).
- **Not started:** tasks 9-15.
- **Tests:** 2,935 `#[test]` functions remain of the 4,055 in the baseline.
