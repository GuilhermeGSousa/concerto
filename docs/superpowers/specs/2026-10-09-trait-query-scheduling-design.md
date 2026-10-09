# Trait Query Scheduling: Access Resolved from Query State — Design Plan

**Status:** proposed. Read-only trait queries (`All<&dyn Trait>`) have landed with
conservative access. This plan makes their access precise and unlocks `All<&mut dyn Trait>`.
Open questions below must be settled before an implementation plan.
**Touches:** `crates/ecs` (system/input, system/schedule, system/access, query, trait_query),
every `SystemInput` and `QueryData` implementation (in `ecs`, plus `app/extractor.rs`,
`debug-gizmos/gizmos.rs` and `editor/inspector/registry.rs`), and four editor tests that
read the access of an uninitialized system.
**Constraint:** determinism is non-negotiable (see
`2026-09-26-command-flushing-and-graph-reduction-design.md`). A system's declared access,
and therefore the compiled graph, must depend only on what was registered before the
schedule compiled. It must never depend on timing.

## Where things stand

- `Query<All<&dyn Trait>>` yields, for each entity, every registered component
  implementing `Trait`. Implementors are registered with
  `register_component_as::<dyn Trait, C>()` into a process-wide registry. A query's state
  snapshots that registry and refreshes when the registry's generation moves
  (`WorldQuery::refresh_state`).
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
exactly the components its snapshot covers: `All<&dyn Interactable>` with implementors
`{Door, Lever}` reads `Door` and `Lever`, and nothing else.
`SystemAccess::are_disjoint` and `add_implicit_edges` do not change. Precise access feeds
the existing conflict rules.

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
"`fill_access` called before `initialize`" when it has no state (see open question 3).
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

## Phase 2 — Precise access for `All<&dyn Trait>`

**Declared access.** `All<&dyn T>::fill_access` calls `read_component_id` for each
implementor in its state's snapshot, and drops `read_all_components()`.

**Pinning.** Today a query's state follows the registry. Under precise access that would
be unsound: a system compiled with "reads `{Door, Lever}`" that later starts reading
`Chest` touches a component its access never declared, and could race a `Chest` writer on
the multithreaded executor. So:

- A `QueryState` created as a **system input** (`Query::init_state`) is *pinned*: it keeps
  the implementor snapshot taken at `initialize`, and matching never widens.
- A `QueryState` created ad hoc (`World::query`) is not scheduled and keeps following the
  registry, as today.
- When a pinned state sees the generation move and the registry now has more implementors
  for its trait, it logs once per state:

  ```
  `dyn game::Interactable` gained implementor `game::Chest` after a system querying it
  was initialized; that system will not visit it. Register implementors in Plugin::build.
  ```

  See open question 2 for warning vs. panic.

Suggested mechanism: a `pinned: bool` on `QueryState`, set by `Query::init_state`. When it
is set, `update_archetypes` calls a `WorldQuery::check_drift(&state)` hook instead of
`refresh_state`. The hook defaults to a no-op; the trait query implements it.

**`Extracted`.** `Extracted<Query<All<&dyn T>>>` is pinned like any system query.
`Extracted::fill_access` keeps declaring only `read_resource::<MainWorld>()`: the inner
query's access concerns the main world, and forwarding it to the render schedule would
create false conflicts with render-world systems.

**Scheduling consequence, documented in `docs/scheduling.md`.** Registering a new
implementor changes the graph. `Chest: Interactable` adds conflict edges between every
`Interactable` trait-query system and every system that writes `Chest`. Implicit edges
follow registration order, so the result stays deterministic. It can still surprise people,
so the rule belongs in the "Rules" section of the docs.

**Tests.**
- A trait-query system runs in parallel with (shares no edge with) a system that writes a
  component which does not implement the trait. Today they are ordered.
- It is ordered against a writer of an implementor.
- A pinned system query does not visit an implementor registered after `initialize`, and
  the warning is emitted once.
- An ad-hoc `World::query` still picks up late implementors (the existing test).
- An `Extracted` trait query declares only `MainWorld`.

## Phase 3 — `All<&mut dyn Trait>`

- `ImplementedBy<C>` gains `fn cast_mut(&mut C) -> &mut Self::Static`.
  `QueryableTrait` gains `from_static_mut`. `#[queryable]` emits both.
- `TraitImpl` gains `cast_mut: unsafe fn(*mut u8) -> *mut Dyn`.
- `Column::get_ptr_mut(row) -> (*mut u8, &mut Tick)`. The pointer must come from the
  column's mutable buffer (`as_bytes_mut`), not from the shared slice `get_ptr` uses, so
  that writes have valid provenance.
- The item is `TraitIterMut<'w, Dyn>`, yielding `Mut<'w, Dyn>`. `Mut` already accepts
  unsized targets. Each yielded component stamps its own changed tick, so
  `Changed<Door>` works through a trait query.
- `TraitIterMut` is a plain `Iterator`: the registry deduplicates component ids, so items
  never alias each other. It must **not** be `Clone`, because a clone would yield a second
  `&mut` to the same component.
- `fill_access` calls `write_component_id` for each implementor. `ReadOnlyQueryData` is
  not implemented.

**Aliasing inside one query.** `Query<(&mut Door, All<&dyn Interactable>)>` hands out
`&mut Door` and `&dyn Interactable` pointing at the same `Door` together. The same hazard
already exists for plain components (`Query<(&mut A, &A)>` is accepted today), but trait
queries make it far easier to hit by accident. Proposed check, at `initialize`: the
`QueryData` tuple impl collects each element's access into its own `SystemAccess`, and it
panics if two elements conflict. Elements of one tuple always fetch from the same entity,
so a conflict there is always a real alias and this check has no false positives. Across
separate `Query` parameters the check would misfire on disjoint filters (`With<X>` vs.
`Without<X>`), which `SystemAccess` does not model, so it stays scoped to a single tuple.

**Tests.**
- Mutating through `All<&mut dyn T>` marks exactly the mutated components as changed.
- Two trait-query writers of the same trait are ordered, and a writer and a reader of
  disjoint traits are not.
- `Query<(&mut Door, All<&dyn Interactable>)>` panics at `initialize`, naming `Door`.
- `Query<(&mut Door, All<&dyn Unrelated>)>` is accepted.

## Phase 4 — Optional follow-ups

- `One<&dyn T>` (exactly one implementor, else no match) and filters
  `WithAny<dyn T>` / `WithoutAny<dyn T>`. Filters need no access: they only match
  archetypes.
- Show trait expansions in the schedule graph dump, e.g.
  `collect_interactables reads dyn Interactable = {Door, Lever}`. This means storing
  the trait name alongside the expanded ids in `SystemAccess`, for diagnostics only.
- Cache per-archetype implementor columns in `TraitQueryState`, so `fetch` stops probing
  every implementor's column for each entity.

## Open questions

1. **Registry scope.** The registry is process-wide today. That was the simplest way for
   `Extracted` state, initialized on the render world, to see main-world registrations.
   With precise access, though, two `App`s in one process, such as tests in one binary,
   share implementors. A test's graph can then depend on what another test registered for
   the same trait. Engine traits used across many tests (render contributors, gizmos)
   would hit this. The alternative is a per-`App` registry: each `World` holds an
   `Arc<TraitRegistry>`, and `SubApp` creation hands the main world's to the render world.
   **Recommendation:** move to per-`App` before phase 2 lands.
2. **Drift policy.** When a pinned query misses a late implementor: warn once (proposed),
   `debug_assert!` panic, or flag schedules for recompilation (compiling every schedule
   takes about 0.9 ms). A panic is the safest for gameplay code but hostile to editor
   tooling that loads content late. Recompiling is the most capable option, but it adds
   lifecycle to `CompiledSchedules` that nothing else needs yet.
3. **Uninitialized `fill_access`.** Panic (proposed) or fall back to conservative access
   (`read_all_components` / `write_world`) for states that do not exist yet. The fallback
   avoids touching editor tests but lets a scheduling bug slip by silently.
4. **Implementor order.** `All` yields in registration order, which depends on plugin
   order. Should there be an explicit order (an `order` key at registration), for traits
   where order matters, such as instance-data contributors patching the same field?
5. **Filter access generally.** `Changed<T>` and `Added<T>` read ticks without declaring
   reads. That is harmless today because ticks are only written alongside data that is
   declared, but it is worth confirming before trait-level change filters
   (`ChangedAny<dyn T>`) arrive.

## Out of scope

- **Runtime conflict checking** (rejected in the command-flushing plan for determinism).
- **Per-archetype access** (Flecs style), where a system declares access per matched
  table. It would allow more parallelism than per-component access, but needs a
  different executor.
- **Automatic registration** (an `inventory`/`linkme`-style distributed slice). It is
  unreliable on `wasm32-unknown-unknown`, which the engine ships to. A
  `#[component(implements(Trait))]` derive attribute that registers on first
  `register_component` is possible, but lazy registration would turn most registrations
  into "late" ones under phase 2's pinning.

## References

- bevy-trait-query, the prior art for `#[queryable]` and `All`/`One` (it registers per
  world, because Bevy's component ids are per world):
  <https://github.com/JoJoJet/bevy-trait-query>
- Bevy's `FilteredAccess` and in-system conflict validation:
  <https://docs.rs/bevy_ecs/latest/bevy_ecs/query/struct.FilteredAccess.html>
