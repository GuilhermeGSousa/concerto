# Trait Query Scheduling: Access Resolved from Query State — Design Plan

**Status:** proposed. Read-only trait queries (`All<&dyn Trait>`) have landed with
conservative access. This plan makes their reads and writes precise, which unlocks
`All<&mut dyn Trait>`.
**Decided:** implementors are registered per `World`. Trait queries are specific to the world
they query. An implementor registered after a system querying its trait was initialized is
logged as a warning and then panics.
**Touches:** `crates/ecs` (system/input, system/schedule, system/access, query, table,
trait_query), every `SystemInput` and `QueryData` implementation (in `ecs`, plus
`app/extractor.rs`, `debug-gizmos/gizmos.rs` and `editor/inspector/registry.rs`), and four
editor tests that read the access of an uninitialized system.
**Constraint:** determinism is non-negotiable (see
`2026-09-26-command-flushing-and-graph-reduction-design.md`). A system's declared access,
and therefore the compiled graph, must depend only on what was registered on its world
before the schedule compiled. It must never depend on timing.

## Where things stand

- `Query<All<&dyn Trait>>` yields, for each entity, every registered component that
  implements `Trait`. Implementors are registered with
  `World::register_component_as::<dyn Trait, C>()` into the world's `TraitRegistry`. That
  registry holds an append-only implementor list per trait and a generation counter.
- A query's state keeps the implementors' component ids, and refreshes them from the world
  when the registry's generation moves (`WorldQuery::refresh_state`). A fetch visits the
  same-length prefix of the world's list, so a fetch never sees more than its state matched.
- **Access is static.** `SystemInput::fill_access(meta, access)` and
  `QueryData::fill_access(meta, access)` receive no state. `&T` knows its `TypeId` at
  compile time, but a trait query cannot: its components are runtime data. So `All<&dyn T>`
  declares `read_all_components()`. That is sound and runs alongside other readers, but it
  is ordered against every system that writes any component.
- **`All<&mut dyn T>` is blocked on this.** Its only sound static declaration would be
  "writes every component", which serialises it against nearly everything.
- **Access is gathered before `initialize`.** `Schedule::compile` calls `fill_access`
  in `setup_systems_for_compilation` (to find `needs_apply`) and again when building nodes.
  `System::initialize` runs only afterwards, so no system state exists when access is read.

## Goal

A system's access is computed from its initialized state. A trait query then declares
exactly the components its state covers:

| Query | Implementors at `initialize` | Declared access |
|---|---|---|
| `All<&dyn Interactable>` | `Door`, `Lever` | reads `Door`, `Lever` |
| `All<&mut dyn Interactable>` | `Door`, `Lever` | writes `Door`, `Lever` |

`SystemAccess::are_disjoint` and `add_implicit_edges` do not change. Precise access feeds
the existing conflict rules, and those rules already treat a declared write correctly. The
scheduler needs nothing more for writes than for reads. What writes add is a set of
soundness obligations on the fetch side, covered in phase 3.

## Phase 1 — Stateful `fill_access`, initialize before access

No behaviour change. Every existing input ignores the new argument.

**Signatures.**

```rust
pub trait SystemInput {
    fn fill_access(state: &Self::State, meta: &mut SystemMetadata, access: &mut SystemAccess);
}

pub trait QueryData: WorldQuery {
    fn fill_access(state: &Self::State, meta: &mut SystemMetadata, access: &mut SystemAccess);
}
```

- Tuple impls pass `&state[[i]]` to each element. `Query<T, F>` passes `state.data_state()`.
- `StaticSystemInput`, `Extracted` and the editor's `InspectionSource` forward their inner
  state.
- `System::fill_access(&self, meta, access)` keeps its signature. `FunctionSystem` passes
  `self.system_state`. `SystemWithArgs` and `BoxedSystem` forward as they do today.

**`SystemAccess` gains id-based methods.** A trait query only has `ComponentId`s, not types:

```rust
pub fn read_component_id(&mut self, id: ComponentId);
pub fn write_component_id(&mut self, id: ComponentId);
```

`read_component::<T>()` and `write_component::<T>()` become one-line wrappers.

**Compile order.** `Schedule::compile` initializes every system first, before
`setup_systems_for_compilation`. That pass already needs `needs_apply`, which now
comes from state. Inserted `SyncPoint`s are initialized too (a no-op) so that every system
in `CompiledScheduleData` has been initialized. No system runs during compile, so moving
`initialize` earlier is not observable.

**Uninitialized systems.** `FunctionSystem::fill_access` panics with
"`fill_access` called before `initialize`" when it has no state (see open question 1).
These callers read access from systems that were never initialized and must initialize on
a fresh `World` first:

- `crates/editor/src/workspace.rs:211`
- `crates/editor/src/asset_editor.rs:349`
- `crates/editor/src/inspector/tests.rs:32` and `:679`
- `crates/ecs/tests/trait_query.rs` (`is_scheduled_as_reading_every_component`)

Their inputs are `Res`, `ResMut`, `Query`, `CommandQueue` and `InspectionSource`. None of
these reads a resource in `init_state`, so a bare `World::new()` is enough.

**Tests.**
- Every existing ordering test passes unchanged, and compiled graphs are edge-for-edge
  identical. A debug test can compare `CompiledScheduleData::system_access` before and
  after on the headless app with `UIPlugin`.
- `fill_access` on an uninitialized function system panics with the message above.

## Phase 2 — Precise reads, and pinning

**Declared access.** `All<&dyn T>::fill_access` calls `read_component_id` for each id in
its state, and drops `read_all_components()`.

**Pinning.** Today a query's state follows its world's registry. Under precise access that
would be unsound: a system compiled with "reads `{Door, Lever}`" that later starts reading
`Chest` touches a component its access never declared. On the multithreaded executor it
could then race a `Chest` writer. So:

- A `QueryState` created as a **system input** (`Query::init_state`) is *pinned*: its
  implementor set is fixed at `initialize`.
- A `QueryState` created ad hoc (`World::query`) is not scheduled. It keeps following the
  registry, as today.
- When a pinned state's refresh sees the generation move **and** its trait has gained
  implementors, it logs the message at warn level and then panics with the same message:

  ```
  `game::Chest` was registered as `dyn game::Interactable` after a system querying it
  was initialized, so that system's scheduled access does not cover it. Register
  implementors in Plugin::build, before the app runs.
  ```

  The warning comes first so the message still reaches the log when the panic happens on
  a worker thread or is caught by the executor. Registrations for other traits move the
  generation too but are ignored: the check compares implementor lists, not counters.
- The check runs in `QueryState::update_archetypes`, from `Query::new`, before any item is
  produced. So a pinned query never touches an undeclared column, even for one fetch. For
  writes (phase 3), this ordering is what keeps the panic ahead of any data race.

Suggested mechanism: a `pinned: bool` on `QueryState`, set by `Query::init_state` and passed
through `WorldQuery::refresh_state(state, world, pinned)`. The trait query implements the
check. Every other `WorldQuery` ignores the flag.

**Scheduling consequence, documented in `docs/scheduling.md`.** Registering a new
implementor changes the graph. `Chest: Interactable` adds conflict edges between every
`Interactable` trait-query system and every system that writes `Chest`. Implicit edges
follow registration order, so the result stays deterministic. It can still surprise people,
so the rule belongs in the "Rules" section of the docs, along with "register implementors
in `Plugin::build`".

**Tests.**
- A trait-query system runs in parallel with (shares no edge with) a system that writes a
  component which does not implement the trait. Today they are ordered.
- It is ordered against a writer of an implementor.
- Registering an implementor of the queried trait after `initialize` panics on the
  system's next run, with the message above.
- Registering an implementor of an unrelated trait after `initialize` does not panic.
- An ad-hoc `World::query` still picks up late implementors (the existing test).

## Phase 3 — Precise writes: `All<&mut dyn Trait>`

### Declared access

`fill_access` calls `write_component_id` for each id in the pinned state.
`ReadOnlyQueryData` is not implemented, so it cannot appear in read-only inputs. That is the
whole scheduler side. The remaining work is making the fetch as sound as the declaration.

### Casting and storage

- `ImplementedBy<C>` gains `fn cast_mut(&mut C) -> &mut Self::Static`, and
  `QueryableTrait` gains `from_static_mut`. `#[queryable]` emits both.
- `TraitImpl` gains `cast_mut: unsafe fn(*mut u8) -> *mut Dyn`.
- `Column` gains a mutable row pointer and a changed-tick pointer.

  **Provenance:** these must be derived from a raw base pointer, without creating `&mut`
  over the whole column per fetch. Earlier items for other rows of the same column are
  still alive, and a fresh `&mut` over the buffer would invalidate them under Rust's
  aliasing model. The existing `&mut T` fetch has the same issue: it materialises
  `&mut World` for every entity. Both paths should go through the new pointer accessors,
  and both should be run under Miri.

### Items

- The item is `TraitIterMut<'w, Dyn>`, yielding `Mut<'w, Dyn>`. `Mut` already accepts
  unsized targets.
- Each `Mut` carries its own component's changed tick, so mutating through the trait marks
  exactly the touched components, and `Changed<Door>` works through a trait query.
- Items within one entity never alias: the registry deduplicates component ids, so each
  column appears at most once. `TraitIterMut` can therefore be a plain `Iterator`. It must
  **not** be `Clone`, because a clone would yield a second `&mut` to the same component.

### Aliasing checks at `initialize`

Precise writes make it easy to write a query that aliases:

| Query | Problem when `Door: A + B` |
|---|---|
| `Query<(&mut Door, All<&dyn A>)>` | `&mut Door` and `&dyn A` to the same `Door` |
| `Query<(All<&mut dyn A>, All<&dyn B>)>` | mutable and shared views of one `Door` |
| `Query<(All<&mut dyn A>, All<&mut dyn B>)>` | two `&mut` to one `Door` |

**Check:** the `QueryData` tuple impl collects each element's access into its own
`SystemAccess` and panics if any two elements conflict, naming the component and both
elements. All elements of one tuple fetch the same entity, so a conflict there is always a
real alias, and the check has no false positives. It needs per-element access computed
from state, which phase 1 provides. It also catches the plain-component case
(`Query<(&mut A, &A)>`), which is accepted today.

**Not covered:** aliasing across separate `Query` parameters, and repeated calls on one
query. `Query::iter` and `Query::get_entity` take `&self` and return items for `'world`, so
two calls can hold two `Mut`s to the same component. That is already true for
`Query<&mut T>` today. Trait writes inherit it, and do not make it worse. Fixing it is a
general `Query` change (`iter_mut(&mut self)`, `get_mut`) and is out of scope here. See
open question 3.

### Tests

- Mutating through `All<&mut dyn T>` marks exactly the mutated components as changed.
- Two trait-query writers of the same trait are ordered. A writer and a reader of traits
  with disjoint implementors are not.
- Each row of the aliasing table panics at `initialize`, naming `Door`.
  `Query<(&mut Door, All<&dyn Unrelated>)>` is accepted.
- The read and write fetch tests pass under Miri.

## Phase 4 — Optional follow-ups

- `One<&dyn T>` / `One<&mut dyn T>` (exactly one implementor, else no match), and filters
  `WithAny<dyn T>` / `WithoutAny<dyn T>`. Filters need no access: they only match
  archetypes.
- Show trait expansions in the schedule graph dump, e.g.
  `collect_interactables reads dyn Interactable = {Door, Lever}`. This means storing
  the trait name alongside the expanded ids in `SystemAccess`, for diagnostics only.
- Fetch cost. Each fetch currently looks the trait up in the registry's map and probes
  every implementor's column. Caching each matched archetype's implementor columns in the
  state would remove both. It would also give phase 3 a natural place to take column base
  pointers once.

## Non-goals

- **Cross-world trait queries.** A trait query reads the registry of the world it queries.
  `Extracted<Query<All<&dyn T>>>` is not supported: its state is initialized on the render
  world, whose registry does not know the main world's implementors.
- **Runtime conflict checking** (rejected in the command-flushing plan for determinism).
- **Per-archetype access** (Flecs style), where a system declares access per matched
  table. It would allow more parallelism than per-component access, but needs a
  different executor.
- **Recompiling schedules** when implementors arrive late. Late registration is a
  programmer error and panics.
- **Automatic registration** (an `inventory`/`linkme`-style distributed slice). It is
  unreliable on `wasm32-unknown-unknown`, which the engine ships to. A lazy derive attribute
  that registers on first use would make most registrations "late" under pinning.

## Open questions

1. **Uninitialized `fill_access`.** Panic (proposed) or fall back to conservative access
   (`read_all_components` / `write_world`) for states that do not exist yet. The fallback
   avoids touching editor tests but lets a scheduling bug slip by silently.
2. **Implementor order.** `All` yields in registration order, which depends on plugin
   order. Should there be an explicit order (an `order` key at registration), for traits
   where order matters, such as instance-data contributors patching the same field?
3. **Borrow-checked mutable queries.** Should `Query` move mutable access behind
   `&mut self` (`iter_mut`, `get_mut`) before or alongside phase 3? It closes the
   repeated-call aliasing hole for both plain and trait queries, but touches every system
   that mutates through a query.
4. **Filter access generally.** `Changed<T>` and `Added<T>` read ticks without declaring
   reads. That is harmless today because ticks are only written alongside data that is
   declared, but it is worth confirming before trait-level change filters
   (`ChangedAny<dyn T>`) arrive.

## References

- bevy-trait-query, the prior art for `#[queryable]` and `All`/`One`:
  <https://github.com/JoJoJet/bevy-trait-query>
- Bevy's `FilteredAccess` and in-system conflict validation:
  <https://docs.rs/bevy_ecs/latest/bevy_ecs/query/struct.FilteredAccess.html>
