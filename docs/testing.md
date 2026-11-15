# Testing

A test costs build time, run time, and the attention of everyone who changes
the code it touches. It earns that cost only by catching a broken promise. A
suite that breaks on every refactor, or that a person must rewrite by the dozen
to change one function, is a defect of the suite, not a sign of rigour.

## What a test is for

A test earns its place when all four hold:

1. **It fails only when a promise breaks.** It sits on a boundary that callers
   rely on: a public API, a persisted encoding (`.gasset`, scenes, reflected
   JSON), an MCP tool, a signal or an event, an ECS invariant. A refactor that
   keeps behaviour keeps it green.
2. **It guards a mistake someone would plausibly make,** or one that was made.
3. **Nothing else already catches it.** A smaller test that fails only together
   with a scenario test above it is noise.
4. **Reading it tells you the promise.** The body states the contract, not the
   steps of the implementation.

## The four kinds

| Kind | Where | Shape |
|---|---|---|
| Invariant | `ecs`, scheduling, change detection, `reflect`, asset encodings | Small and dense. Encodings get a round trip and a committed fixture. |
| Algorithm | animation, retargeting, motion matching, navigation, BVH, layout | One table-driven test per function's edge cases. A property test with `proptest` where a law exists: a round trip is identity, an incremental result equals a fresh one, an output stays in range. |
| Feature scenario | each plugin, the editor most of all | Build an `App`, drive it through its signals, events or MCP tools, assert on what a user or agent sees: world state, saved files, events, a tool's reply. One scenario per feature, with as many assertions as the feature has outcomes. |
| Widget | `concerto-ui` and the editor's reusable widgets | Only what nothing above can express: focus, drag, text entry, hit testing, scrolling. |

Performance guarantees, such as "an idle frame lays nothing out", are
`criterion` benchmarks, not tests. They live in `crates/benches`
(`concerto-benches`), reach the engine through its public API, and run with

```sh
cargo bench -p concerto-benches --features run
```

Benchmarks do not run in CI, and without the `run` feature the crate builds
without `criterion` and skips them. Run them by hand before and after a change
that could slow the code they measure. Production types carry no `#[cfg(test)]` fields,
counters or getters of internal state for a test to read. A `#[cfg(test)]`
setter may stay when it puts the code in a state no public path reaches in a
test, as `World::set_change_tick` does for the tick-check threshold.

## What not to write

A test has to earn its place; the default is to delete. Being the only cover
of a behaviour does not save a test whose failure nobody would notice: a
message's wording, an internal ordering, a defensive branch, a helper's edge
case. Keep the tests whose failure would reach a user, a game or an agent, and
cover each feature once.

- A test of a getter, a constructor, a `Default`, a derive, a constant, a
  label's formatting, or command-line flag parsing.
- One test per error variant or per edge case. Put the cases in a table.
- A family of tests that differ in one input. Make it one table or one
  property.
- A test of a private function, unless the function is maths.
- An editor test that asserts on widget entities, row counts or `UIText`
  strings to show a feature works. Drive the feature through its signal or MCP
  tool and assert on the scene or the reply.
- A test that clicks through panels to reach a state. If an agent could not
  reach that state without clicking, the MCP tool is missing; see the editor
  rules in `AGENTS.md`.
- A test of what a dependency does on its own.

## Writing one

Write the failing test first, at the highest of the four levels that can
express the behaviour, and watch it fail. Extend an existing scenario before
adding a new test: a new outcome of a feature is a new assertion, not a new
function. A regression gets the smallest test that would have failed, at the
level where the bug was visible.

Fixtures and helpers live once per crate, in its `tests/` directory or a
`testing` module, so a change of API is one edit and not a hundred.

## The budget

`test-budget.toml` at the root holds each package's `#[test]` count, and CI's
Test budget job fails when a package has more or fewer tests than it says:

```sh
python3 .github/scripts/test_budget.py
python3 .github/scripts/test_budget.py --write
```

The second writes the current counts. A change that deletes tests lowers the
budget with it. A change that adds tests raises it in the same diff and says
why in its report or PR, after trying an assertion in an existing scenario, a
row in a table or a property first. The budget is a ceiling the owner reviews,
not a revision counter. Every report gives its net change as
`TESTS: +added −deleted, net n`.

Coverage is never a target: it counts lines run, not breaks caught, and
chasing it grows the suite this budget holds down. To find a gap or a test
that catches nothing, run `cargo mutants -p <crate>` now and then on a crate
where a break costs most, and read what survives.

## Layout

A crate's integration tests are one binary: each file is a module of
`tests/<crate>/main.rs`, where `<crate>` is the crate's directory name. A new
file adds a `mod` line there; a file directly in `tests/` would be a binary of
its own, linked against the whole engine.
