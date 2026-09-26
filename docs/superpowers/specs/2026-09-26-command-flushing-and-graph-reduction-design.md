# Command Flushing and Graph Reduction — Design Plan

**Status:** brainstorm; phase 0 done. Open questions below must be settled before an implementation plan for phases 2 and 3
**Touches:** `crates/ecs` (command, system/schedule, system/access, executors),
every crate that takes `CommandQueue` (38 files, 89 system parameters)
**Constraint:** determinism is non-negotiable. For a given schedule, which commands
are applied, where, and in what order must depend only on the graph, never on
timing, thread count or executor.

## Measurements this plan is based on

Taken on the headless app plus `UIPlugin` (release, lavapipe, 4 cores). Code for
both probes was throwaway and is not in the tree.

**Graph shape.** Longest chain of systems that must run one after another, with and
without the sync points inserted after every `CommandQueue` system:

| Schedule | Systems | Sync points | Longest chain | Without sync points |
|---|---|---|---|---|
| `Update` | 17 | 2 | 12 | 12 |
| `LateUpdate` | 37 | 5 | 24 | 18 |
| `Extract` | 15 | 10 | 11 | 3 |
| `Render` | 11 | 0 | 8 | 8 |

**Redundant edges.** Edges implied by a longer path:

| Schedule | Edges | Transitive reduction | Redundant |
|---|---|---|---|
| `Update` | 95 | 20 | 79% |
| `LateUpdate` | 396 | 49 | 88% |
| `Extract` | 204 | 27 | 87% |
| `Render` | 33 | 11 | 67% |

**Costs.**
- Compiling every schedule takes ~0.9 ms in total; graph construction is not a concern.
- 42 no-op systems with the same total order: 861 edges run in 190 µs per frame on
  the multithreaded executor, 41 edges in 179 µs, and no edges in 60 µs. Redundant
  edges cost ~6%; each dependency hop costs ~3 µs of task handoff.
- The single-threaded executor ignores edges (0.13 µs for all three).

**Conclusion.** Sync points are the largest lever on parallelism. Redundant edges
are cheap but free to remove.

## Today's semantics

- `CommandQueue` is the only system parameter that defers work (`needs_apply`).
- The single-threaded executor applies a system's commands immediately after it
  runs (`run_and_apply`).
- The multithreaded executor inserts a `SyncPoint` after every system that needs
  apply. A sync point declares `write_world`, so it conflicts with every system, and
  applying at it flushes every finished system's queue.
- Contract, identical on both executors: **a system sees the commands of every
  system registered before it in the same schedule.** Anything that changes when
  commands are applied changes this contract, so it must be migrated deliberately.

## Phase 0 — Fix the command queue's undeclared access (existing bug) — done

`CommandQueue::get_data` handed the system `&mut EntityStore` so it could allocate
entities, but `fill_access` declared only `needs_apply`. Queries read the entity
store (`find_location` in `query/mod.rs` and `query/filter.rs`). In a schedule
`[query_system, command_system]` nothing ordered the two, so they could run
concurrently while `EntityStore::alloc` pushed to `metadata`, which may reallocate:
a data race.

**Fix:** reservation, as in Bevy's `Entities::reserve_entity`. It is safe to use from
several threads without a lock.
- `EntityStore::reserve(&self)` hands out an entity with one atomic decrement of a
  free cursor. Positive cursor values index into the free list; zero or negative
  values count fresh indices past `metadata`.
- `EntityStore::flush(&mut self)` makes reservations real: it trims the free list
  and grows `metadata`. Every `&mut` method flushes first.
- `CommandQueue` now holds `&EntityStore`, so no `&mut` aliases the store during
  parallel execution. Readers stay lock-free: a reserved entity is simply not found
  until its spawn command is applied.

A `Mutex` or `RwLock` would also have fixed the race, but every
`Query::get_entity` would then take a lock, and parallel queries would all contend
on the same counter.

## Phase 1 — Transitive reduction

After explicit and implicit edges are built, drop every edge `u → v` for which
another successor `w` of `u` already reaches `v`. The `Reachability` bitsets
built for cycle detection already hold the transitive closure, so the pass is
`O(E · V / 64)`.

- **No semantic change.** The ordering is identical by definition; only redundant
  dependency-count decrements disappear from the executor.
- **Expected gain:** ~6% executor overhead in the synthetic benchmark. The larger
  benefit is that the compiled graph becomes readable (49 edges instead of 396 in
  `LateUpdate`), which matters for the debugging tools phases 2 and 3 will need.
- **Tests:** chains and diamonds reduce to their minimal edge set; run order is
  unchanged for every existing ordering test.

This phase is independent of the others and can land first.

## Phase 2 — Flush at the end of the schedule, with an opt-in flush marker

**Default.** A system's commands are applied at the end of the schedule, not after
the system. Most command systems (spawning, despawning, inserting) don't need their
effects visible within the same schedule.

**Opt-in.** A system that must see earlier commands takes a marker parameter, like
`NonSendMarker`, whose `fill_access` records a flush requirement:

```rust
fn attach_children(_: Flushed, query: Query<&Spawned>) { ... }
```

Compilation inserts a sync point before every flush-requiring system `F`:

- The sync point applies the queues of every command system that is an *ancestor*
  of `F` in the graph and has not yet been applied, in registration order.
- Non-ancestors are never flushed early, even if they happen to have finished.
  Which queues are applied depends only on the graph.
- Sync points with the same ancestor set are merged, so several flush-requiring
  systems share one barrier.
- Queues not flushed by any sync point are applied at the end of the schedule, in
  registration order.

**Both executors must follow this model.** The single-threaded executor stops
applying after every system and applies at the same sync points and the same end
of schedule. Otherwise a schedule would behave differently depending on the
executor, which breaks determinism.

**Migration.** This breaks the "see everything registered before you" contract.
Add a per-schedule setting and flip schedules one at a time, as was done for UI
with sets:

```rust
enum FlushPolicy {
    AfterEachCommandSystem, // today's behaviour; the default until migration ends
    Explicit,               // end of schedule plus `Flushed` markers
}
```

Once every engine schedule runs `Explicit` and passes its tests, remove
`AfterEachCommandSystem`.

**Expected gain.** In `Extract`, 10 sync points collapse to at most a few, and the
longest chain drops from 11 toward 3. `LateUpdate` goes from 24 toward 18.

## Phase 3 — Typed command queues and inferred flushes

`CommandQueue<A>`, where `A` declares what the queued commands may touch. Two
possible shapes:

```rust
CommandQueue<(Spawn<(Transform, Mesh)>, Insert<Selected>, Despawn)>
CommandQueue<Writes<(Transform, Selected)>>
```

With declared effects, flushes can be inferred the way Flecs does it instead of
being marked by hand. A flush is needed before system `F` when an ancestor's queue
declares a structural change to a component `F` reads. The `Flushed` marker stays
as an explicit override, and an untyped `CommandQueue` keeps meaning "may touch
anything".

**What typed queues can and can't do.**
- **Can:** decide *where* sync points go, and so how many there are. That is where
  the measured win is.
- **Can't (initially):** make a sync point conflict with less than everything.
  Structural commands move entities between archetype tables, which invalidates any
  query over those tables. Applying currently needs `&mut World`. So a sync point
  remains an exclusive barrier while it runs.
- **Later:** narrow barriers for purely non-structural commands, such as
  `insert_resource::<R>`, which conflict only with users of `R`.

**Determinism holds.** Inference depends only on declared types and graph ancestry.

## Open questions

1. **Producer assigned to two sync points.** If producer `P` is an ancestor of two
   unordered sync points, whichever runs first would apply `P`, making the
   application point depend on timing. Proposed rule: assign `P` to the first sync
   point in topological order among those needing it, and add an edge from that
   sync point to the others that need `P`. Does this add too much ordering?
2. **Marker vs. edge.** Should `F: Flushed` flush all command ancestors, or only
   those named by explicit ordering (Bevy flushes across explicit edges only)? A
   variant: `.after_flushed(producer)` as the flush-requiring counterpart of
   `.after`.
3. **Naming:** `Flushed`, `AfterFlush` or `SeesCommands` for the marker, and
   `CommandQueue<A>` vs. `Commands<A>`.
4. **Typed-queue granularity:** per operation (`Spawn`, `Insert`, `Remove`,
   `Despawn`) or per component written. Per operation is more precise for
   inference; per component is simpler to write.
5. **End of schedule vs. end of frame.** Applying at the end of the schedule keeps
   schedules self-contained. The end of the frame would allow more batching, but
   commands would then leak across schedules.
6. **Diagnostics.** A debug-only check for systems that read entities spawned by an
   unflushed queue. Is that possible without much overhead? It would make phase 2's
   migration much safer.

## Out of scope

- **Runtime conflict checking** (Bevy style). It would free the fixed orientation
  of conflicting pairs, but gives up determinism. Rejected.
- **Executor task batching.** The ~3 µs per hop is a real cost and deserves its own
  plan (grouping tiny systems into one task, or running small schedules
  single-threaded).
- **The terminal renderer**, tracked in
  `2026-09-25-terminal-renderer-split-worlds.md`.

## References

- Bevy 0.13 automatic sync points:
  <https://bevy.org/news/bevy-0-13/>,
  <https://bevy.org/learn/migration-guides/0-12-to-0-13/>
- Flecs sync points and merges:
  <https://www.flecs.dev/flecs/md_docs_2Systems.html>,
  <https://www.flecs.dev/flecs/md_docs_2DesignWithFlecs.html>
- Transitive reduction of a DAG: Aho, Garey and Ullman, *The Transitive Reduction of
  a Directed Graph*, SIAM J. Computing, 1972.
