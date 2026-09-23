# System Scheduling API — Design

**Status:** approved, ready for an implementation plan
**Touches:** `crates/ecs` (system/config, system/schedule, macros), `crates/app`,
`crates/render`, `crates/ui`

## Problem

There is no way to say "this system runs after that one" without the scheduler
running that one twice, and no way at all to say it across a crate boundary.

**`.after` duplicates systems.** `SystemConfig` owns its dependencies:

```rust
pub struct SystemConfig {
    pub(crate) system: BoxedSystem,
    pub(crate) after: Vec<SystemConfig>,
    pub(crate) before: Vec<SystemConfig>,
}
```

`Schedule::add_config` recurses into those children and unconditionally pushes a
new node and a new boxed system for each (`crates/ecs/src/system/schedule.rs`).
Nothing looks up whether that system is already in the schedule, because nothing
can — the graph is built incrementally as systems are registered, and a
constraint can only name something that exists at that moment.

This is not hypothetical. In the render pipeline as it stands today:

- `update_changed_lights` is registered standalone in `RenderPlugin::build`, then
  again as an owned dep of `update_shadow_view_proj` and of `resize_shadow_maps`
  — three copies, three executions per frame.
- `update_shadow_view_proj` is registered in `RenderPlugin::build` and again as a
  dep in `ShadowPipelinePlugin::build` — two copies.
- `finish_render` is pulled in as a dep by both `present_window`
  (`crates/render/src/plugin.rs`) and `readback_terminal_frame`
  (`crates/terminal-renderer/src/plugin.rs`).

**Ordering is really registration order.** The mechanism that actually sequences
almost everything is implicit: every pair of systems whose `SystemAccess` is not
disjoint gets an edge, directed by the order they were added.

```rust
for node_index in &self.system_ids {
    if let Some(other) = self.graph.node_weight(**node_index)
        && !SystemAccess::are_disjoint(&access, other.access())
    {
        self.graph.add_edge(**node_index, *node_idx, ());
    }
}
```

So "run after X" is spelled "be registered after X", and plugin registration
order silently defines frame semantics.

**Hence LateUpdate.** `UIPlugin` puts 24 systems in `LateUpdate` whose
correctness rests entirely on line order, with a comment block standing in for
the constraint graph (`crates/ui/src/plugin.rs`). The editor adds roughly twenty
more, and can only position itself relative to UI by being registered after
`UIPlugin` — the systems it needs to sit between are private to `ui`. Every
crate that needs to run "late relative to something" has no vocabulary other
than joining `LateUpdate`, so `LateUpdate` is where everything ends up.

An explicit constraint that contradicts the implicit registration-order edge is
also not an error anyone can read: the two edges form a cycle and `compile`
panics on a bare `expect` at the toposort.

## Goals

- Ordering constraints reference systems instead of owning them; a referenced
  system runs once.
- Ordering targets that cross a crate boundary without exposing the target
  function: named sets.
- Registration order stops deciding semantics for anything explicitly
  constrained, so plugins can be registered in any order.
- Existing behaviour is preserved where nothing is explicitly constrained.
- `ui` and `render` migrated onto the new API as proof; the rest follows later.

## Non-goals

- Emptying `LateUpdate`. UI stays in `LateUpdate` here (see Migration).
- Nested sets. Sets are flat; a system may belong to several.
- Run conditions, states, or anything else schedule-adjacent.
- Automatic ambiguity detection/reporting beyond what is named below.

## Design

### Ordering targets

A constraint names a target rather than carrying a system:

```rust
pub enum DependencyTarget {
    System(TypeId),
    Set(InternedSystemSet),
}

pub trait IntoDependencyTarget {
    fn into_target(self) -> DependencyTarget;
}
```

`IntoDependencyTarget` is implemented for anything implementing `IntoSystem` and
for anything implementing `SystemSet`. For a system, the `TypeId` is obtained by
converting it and reading `System::system_type()` — which returns
`TypeId::of::<FunctionSystem<F, Input>>()`, stable and unique per named function
item. The temporary boxed system is dropped; only the id is kept.

`SystemConfig` becomes:

```rust
pub struct SystemConfig {
    system: BoxedSystem,
    sets: Vec<InternedSystemSet>,
    after: Vec<DependencyTarget>,
    before: Vec<DependencyTarget>,
}
```

The disappearance of `Vec<SystemConfig>` is the fix: a config can no longer own
another system, so it can no longer cause one to be registered twice.

`IntoSystemConfig` keeps `.after()` and `.before()` with new signatures taking
`impl IntoDependencyTarget`, and gains `.in_set(set)`.

### SystemSet

A new label trait built with the existing `define_label!`
(`crates/ecs/src/label.rs`), exactly as `ScheduleLabel` is:

```rust
define_label!(SystemSet);
pub type InternedSystemSet = Interned<dyn SystemSet>;
```

`crates/ecs/macros` gains `#[derive(SystemSet)]` so that enum sets — the shape
every consumer wants — are one line. It also gains `#[derive(ScheduleLabel)]`,
and the hand-rolled `define_schedule_label!` macro in
`crates/app/src/schedule_groups.rs` is deleted in favour of it. Same machinery,
and that file is being edited anyway.

### Configuring sets

Sets are ordered relative to each other, per schedule:

```rust
pub struct SetConfig {
    set: InternedSystemSet,
    after: Vec<DependencyTarget>,
    before: Vec<DependencyTarget>,
}
```

`App::configure_sets(schedule, configs)` and `Schedules::configure_sets` accept a
single `SetConfig` or a tuple of sets with `.chain()`, which pairs consecutive
elements with `before` constraints:

```rust
app.configure_sets(LateUpdate, (UiSet::Input, UiSet::Widgets, UiSet::Layout).chain());
app.configure_sets(LateUpdate, UiSet::Layout.after(TransformSet::Propagate));
```

Configuring the same set more than once accumulates constraints, so two crates
can each constrain a shared set without coordinating.

### Systems in bulk

`add_systems(schedule, systems)` takes a tuple of systems or configs, so a set's
members are one call. `.in_set(set)` applied to a tuple applies to each member;
`.chain()` on a tuple of systems orders them pairwise. Tuple impls use `typle`,
matching how `SystemInput` is already implemented in this crate.
`add_system` stays as the single-system form.

### Two-pass compile

`Schedule` stops building the petgraph during registration. It accumulates:

```rust
pub struct Schedule {
    systems: Vec<BoxedSystem>,
    configs: Vec<NodeConfig>,      // sets + constraints, parallel to systems
    set_configs: Vec<SetConfig>,
}
```

`compile()` then builds the graph in one shot, when every system and every set in
that schedule is known:

1. **Index.** One node per system. Build `TypeId -> Vec<node>` from
   `system_type()`, and `Set -> Vec<node>` from the accumulated `in_set`
   memberships.
2. **Resolve explicit edges.** For each system constraint and each set
   constraint, look the target up and add an edge per resolved node. A set
   target expands to all its members, so `A.before(B)` yields an edge from every
   member of `A` to every member of `B`. Membership counts are small; the
   quadratic expansion is not a concern at this scale.
3. **Unresolved targets.** A target that matches no system and no set produces a
   `log::warn` naming the constraint's owner and the target, and is dropped. It
   is not an error (a plugin may legitimately be absent) and it never
   auto-registers the missing system — that behaviour is what this design
   removes.
4. **Check the explicit graph.** If it contains a cycle, panic with a message
   naming the systems and sets on the cycle, replacing the current bare
   `expect`.
5. **Reachability.** Compute a reachability bitset per node over the
   explicit-only graph.
6. **Implicit edges.** Walk conflicting pairs in registration order and add
   `earlier -> later` — today's rule — but skip any edge whose head already
   reaches its tail, and update the reachability bitsets after each insertion.
   The check must be transitive, not pairwise: a sync point (or any third
   system) between two explicitly ordered systems would otherwise close a cycle
   through a path neither endpoint names. Skipping only edges that would close
   a cycle keeps the graph acyclic by construction, so an explicit constraint
   wins over registration order instead of colliding with it. Updating a bitset
   on insertion is `O(V²/64)` per edge on schedules of a few hundred systems,
   at compile time only.
7. **Sync points.** Unchanged in meaning: a system whose access needs apply and
   that is not itself a sync point gets one after it. Sync points are inserted
   during this pass rather than during registration, before step 6, so they
   participate in implicit edges exactly as they do today. They are excluded
   from the `TypeId` index of step 1 — every `SyncPoint` shares one `TypeId`,
   and they are never ordering targets.
8. Toposort, initialize, hand off to the executor exactly as now.

A system whose `TypeId` appears more than once (the same function added twice)
resolves to every matching node and logs a warning. Closures are unaffected —
each has its own type — but they cannot be named as targets, which is inherent
and fine.

### Public surface after the change

```rust
// ecs
trait IntoSystemConfig {
    fn in_set(self, set: impl SystemSet) -> SystemConfig;
    fn after(self, target: impl IntoDependencyTarget) -> SystemConfig;
    fn before(self, target: impl IntoDependencyTarget) -> SystemConfig;
}
trait SystemSet { .. }            // + #[derive(SystemSet)]
trait IntoDependencyTarget { .. }
Schedule::configure_sets(..)
Schedules::configure_sets(label, ..)

// app
App::add_systems(label, systems)
App::configure_sets(label, sets)
App::add_render_systems(label, systems)
App::configure_render_sets(label, sets)
```

## Migration

### render

Declare the sets the pipeline actually has and drop the owned-dep constraints:

```rust
#[derive(SystemSet, ..)]
pub enum RenderSet { Lights, Shadows, Draw, Present }
```

`update_changed_lights` in `Lights`; `update_shadow_view_proj`,
`resize_shadow_maps`, `render_shadow_maps` in `Shadows`; `present_window` and
`readback_terminal_frame` ordered after `finish_render` by reference.
`ShadowPipelinePlugin` and `terminal-renderer` then constrain against exported
sets and stop re-registering systems they do not own. This removes the two
duplicate light/shadow executions per frame.

### ui

UI's systems **stay in `LateUpdate`**. The editor's systems currently depend on
running after UIPlugin's, and moving UI into `Update` now would break it. What
changes is that UI's order is declared rather than implied:

```rust
#[derive(SystemSet, ..)]
pub enum UiSet { Input, Widgets, Setup, Project, Materials, Layout, PostLayout }
```

chained once with `configure_sets`, with the 24 `add_system` calls becoming
`add_systems(..).in_set(..)` grouped by set, and the explanatory comment block
replaced by the sets themselves. The sets are public, so the editor, physics and
animation can later name them and move out of `LateUpdate` — that work is
deliberately not in this change.

### Everything else

Untouched. Unconstrained systems keep their current implicit registration-order
edges, so behaviour is unchanged for crates that are not migrated.

## Testing

In `crates/ecs`, unit tests over `Schedule`:

- a system named by `.after` and also added once produces one node, not two
- `.after` resolves regardless of whether the target was registered before or
  after the constraint (the registration-order independence that motivates the
  two-pass compile)
- an unresolved target is dropped and does not add a node
- set membership expands to per-member edges, in both directions
- an explicit constraint that contradicts registration order produces the
  explicit order and no cycle
- two access-conflicting systems with no explicit constraint keep registration
  order
- a cycle in the explicit graph panics with both names in the message
- an explicit constraint reversing registration order across an intervening
  sync point compiles (the transitive skip in step 6) rather than panicking
- `.chain()` on a set tuple and on a system tuple produces consecutive edges

Integration test: a schedule where a counter system is the target of two
separate `.after` constraints runs it exactly once per `run`. This test fails
against the current implementation, which is the point.

Manual check: `cargo run -p render-test` (or the editor) renders identically, and
the shadow systems appear once each in a Tracy capture rather than two and three
times.

## Risks

- **Silent behaviour change from dropped auto-registration.** Code that relied on
  `.after(x)` to *add* `x` will now warn and drop the constraint instead. The
  audit above found no such case — every current `.after` target is also
  registered explicitly — but the warning is what makes a missed one visible.
- **Reachability cost.** Bitset reachability is O(V·E/64) per schedule at
  compile time only, on schedules of a few hundred systems. Not a runtime cost.
- **Explicit-wins masking a real conflict.** Two systems that genuinely conflict
  and are explicitly ordered the "wrong" way for data reasons will now be
  ordered that way silently rather than panicking. That is the requested
  semantics: the author asked for it.
