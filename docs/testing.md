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
benchmarks, not tests. Production types carry no `#[cfg(test)]` fields,
counters or getters of internal state for a test to read. A `#[cfg(test)]`
setter may stay when it puts the code in a state no public path reaches in a
test, as `World::set_change_tick` does for the tick-check threshold.

## What not to write

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
