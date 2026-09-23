# System Scheduling API Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace owned-dependency `.after`/`.before` with reference-based ordering plus named system sets, so a constraint never duplicates a system and can cross crate boundaries.

**Architecture:** `SystemConfig` stops owning dependency systems and stores `DependencyTarget`s (a `TypeId` for a system, or an interned label for a set). `Schedule` stops building its petgraph during registration and instead accumulates systems, set memberships and unresolved constraints; `Schedule::compile` builds the whole graph at once, when every system and set is known. Implicit access-conflict edges are still added in registration order, but only where they would not contradict an explicit constraint.

**Tech Stack:** Rust 2021, `petgraph` (already a dependency of `ecs`), `typle` (tuple impls, already used), `syn`/`quote` (in `crates/ecs/macros`), `log`.

**Spec:** `docs/superpowers/specs/2026-09-23-system-scheduling-api-design.md`

## Global Constraints

- The workspace denies warnings: `[lints.rust] warnings = "deny"` in the root `Cargo.toml`. Any unused import, unused variable or dead code fails the build. Run `cargo build --workspace` before every commit.
- Comment style: no narrative comments. Only one-line `///` doc comments on public items. Do not add block comments explaining reasoning — that belongs in the commit message.
- Sets are flat. There is no nesting, no set-in-set membership.
- No behaviour change for systems with no explicit constraint: they keep today's registration-order implicit edges.
- `ui` systems stay in the `LateUpdate` schedule in this plan. Do not move them to `Update` — the editor depends on running after them.
- Every task ends with `cargo test --workspace` passing, not just the new test.

## Review Focus

These are the cases the spec implies but does not enumerate. Each has a test attached to the task that owns the code.

1. **A set that no system joined, named as an ordering target.** `.after(SomeSet)` where `SomeSet` has zero members in that schedule must drop the constraint quietly, not panic and not warn — a plugin legitimately contributing no systems is normal. Test in Task 4.
2. **The same function added twice, then named as a target.** `.after(foo)` when `foo` was added twice must order against both copies and log one warning. Test in Task 3.
3. **A target that exists only in another schedule.** `add_system(Update, a.after(b))` where `b` is in `LateUpdate` must warn and drop, never reach across schedules. Test in Task 3.
4. **A self-referential constraint.** `foo.after(foo)`, directly or via a set `foo` belongs to, must not add a self-loop — `toposort` fails on one and the panic would name a single system with no explanation. Test in Task 3 (direct) and Task 4 (via set).
5. **A cycle created by set expansion.** `A.before(B)` plus a member of `B` constrained before a member of `A` must panic naming the sets and systems involved, not produce a bare toposort failure. Test in Task 4.

---

### Task 1: `SystemSet` label and the derive macros

**Files:**
- Create: `crates/ecs/src/system/set.rs`
- Modify: `crates/ecs/src/system/mod.rs` (add `pub mod set;` and re-export)
- Modify: `crates/ecs/src/lib.rs:40` (re-export `SystemSet`)
- Modify: `crates/ecs/macros/src/lib.rs` (add two derives)
- Modify: `crates/app/src/schedule_groups.rs` (delete the hand-rolled macro, use the derive)
- Test: `crates/ecs/tests/system_set.rs`

**Interfaces:**
- Consumes: `crate::define_label!` from `crates/ecs/src/label.rs:33`, `crate::intern::Interned`.
- Produces: `ecs::system::set::{SystemSet, InternedSystemSet}`, re-exported as `ecs::SystemSet`; `#[derive(SystemSet)]` and `#[derive(ScheduleLabel)]` from `ecs_macros`.

- [ ] **Step 1: Write the failing test**

Create `crates/ecs/tests/system_set.rs`:

```rust
use ecs::SystemSet;

#[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
enum TestSet {
    First,
    Second,
}

#[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
struct UnitSet;

#[test]
fn interning_the_same_variant_twice_yields_equal_handles() {
    assert_eq!(TestSet::First.intern(), TestSet::First.intern());
}

#[test]
fn different_variants_intern_differently() {
    assert_ne!(TestSet::First.intern(), TestSet::Second.intern());
}

#[test]
fn different_types_with_the_same_shape_intern_differently() {
    let a: Box<dyn SystemSet> = Box::new(UnitSet);
    let b: Box<dyn SystemSet> = Box::new(TestSet::First);
    assert!(a.as_ref() != b.as_ref());
}
```

The `PartialEq for dyn SystemSet` that the third test relies on comes from `define_label!` (`crates/ecs/src/label.rs:59`), via the blanket `DynEq`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p ecs --test system_set`
Expected: FAIL — `cannot find derive macro `SystemSet``.

- [ ] **Step 3: Add the label trait**

Create `crates/ecs/src/system/set.rs`:

```rust
use crate::{define_label, intern::Interned};

define_label!(
    /// A named group of systems that can be ordered as a unit.
    SystemSet
);

/// A cheap, copyable handle to a [`SystemSet`].
pub type InternedSystemSet = Interned<dyn SystemSet>;
```

In `crates/ecs/src/system/mod.rs`, add `pub mod set;` next to the other module declarations and `pub use set::{InternedSystemSet, SystemSet};` next to the existing `pub use config::...` line.

In `crates/ecs/src/lib.rs`, extend line 40's re-export to:

```rust
pub use system::{
    IntoSystem, IntoSystemConfig, System, SystemConfig, SystemSet, schedule::Schedule,
};
```

- [ ] **Step 4: Add the derive macros**

Append to `crates/ecs/macros/src/lib.rs`:

```rust
#[proc_macro_derive(SystemSet)]
pub fn system_set(input: TokenStream) -> TokenStream {
    let ast = syn::parse(input).unwrap();
    impl_label(&ast, quote!(SystemSet))
}

#[proc_macro_derive(ScheduleLabel)]
pub fn schedule_label(input: TokenStream) -> TokenStream {
    let ast = syn::parse(input).unwrap();
    impl_label(&ast, quote!(ScheduleLabel))
}

fn impl_label(ast: &syn::DeriveInput, trait_name: proc_macro2::TokenStream) -> TokenStream {
    let name = &ast.ident;
    let (impl_generics, type_generics, where_clause) = ast.generics.split_for_impl();
    let gen = quote! {
        impl #impl_generics #trait_name for #name #type_generics #where_clause {
            fn dyn_clone(&self) -> ::std::boxed::Box<dyn #trait_name> {
                ::std::boxed::Box::new(::std::clone::Clone::clone(self))
            }
        }
    };
    gen.into()
}
```

`quote!` returns `proc_macro2::TokenStream`, so add `proc-macro2 = "1"` to `crates/ecs/macros/Cargo.toml` dependencies and `extern crate proc_macro2;` is not needed (2021 edition).

The derive names the trait unqualified, so users must have `SystemSet` / `ScheduleLabel` in scope where they derive. That matches how `#[derive(Component)]` already works in this crate.

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p ecs --test system_set`
Expected: PASS (3 tests).

- [ ] **Step 6: Use the derive in `app`**

Replace the body of `crates/app/src/schedule_groups.rs` with:

```rust
use ecs::system::schedule::ScheduleLabel;
use ecs_macros::ScheduleLabel;

macro_rules! define_schedule_label {
    ($name:ident) => {
        #[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
        pub struct $name;
    };
}

define_schedule_label!(Main);
define_schedule_label!(RenderMain);
define_schedule_label!(Startup);
define_schedule_label!(First);
define_schedule_label!(Update);
define_schedule_label!(FixedUpdate);
define_schedule_label!(LateUpdate);
define_schedule_label!(LateFixedUpdate);
define_schedule_label!(Extract);
define_schedule_label!(Render);
define_schedule_label!(LateRender);
```

Confirm `ecs_macros` is a dependency of `app`; if not, add `ecs-macros = { path = "../ecs/macros" }` to `crates/app/Cargo.toml`. Prefer re-exporting from `ecs` if `ecs` already re-exports its derives — check `crates/ecs/src/lib.rs` for `pub use ecs_macros::*` and use `ecs::ScheduleLabel` if so.

- [ ] **Step 7: Verify the workspace builds and tests pass**

Run: `cargo build --workspace && cargo test --workspace`
Expected: PASS, no warnings.

- [ ] **Step 8: Commit**

```bash
git add crates/ecs/src/system/set.rs crates/ecs/src/system/mod.rs crates/ecs/src/lib.rs \
        crates/ecs/macros/src/lib.rs crates/ecs/macros/Cargo.toml \
        crates/app/src/schedule_groups.rs crates/ecs/tests/system_set.rs
git commit -m "Add SystemSet label and SystemSet/ScheduleLabel derives"
```

---

### Task 2: Cycle-safe reachability

**Files:**
- Create: `crates/ecs/src/system/reachability.rs`
- Modify: `crates/ecs/src/system/mod.rs` (add `mod reachability;`)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `pub(crate) struct Reachability` with `Reachability::new(node_count: usize) -> Self`, `fn reaches(&self, from: usize, to: usize) -> bool`, `fn add_edge(&mut self, from: usize, to: usize)`, and `fn try_add_edge(&mut self, from: usize, to: usize) -> bool` (returns `false` and adds nothing when the edge would close a cycle).

This is a pure data structure with no ECS dependencies, which is why it is its own task: Task 3 depends on it being right.

- [ ] **Step 1: Write the failing test**

Append to `crates/ecs/src/system/reachability.rs` (the file will not exist yet — create it with only this test module for now):

```rust
#[cfg(test)]
mod tests {
    use super::Reachability;

    #[test]
    fn a_fresh_graph_has_no_reachability() {
        let r = Reachability::new(3);
        assert!(!r.reaches(0, 1));
        assert!(!r.reaches(0, 0));
    }

    #[test]
    fn an_edge_makes_its_head_reachable() {
        let mut r = Reachability::new(2);
        r.add_edge(0, 1);
        assert!(r.reaches(0, 1));
        assert!(!r.reaches(1, 0));
    }

    #[test]
    fn reachability_is_transitive() {
        let mut r = Reachability::new(3);
        r.add_edge(0, 1);
        r.add_edge(1, 2);
        assert!(r.reaches(0, 2));
    }

    #[test]
    fn an_edge_added_earlier_still_sees_later_descendants() {
        let mut r = Reachability::new(3);
        r.add_edge(0, 1);
        r.add_edge(1, 2);
        assert!(r.reaches(0, 2));
        r.add_edge(2, 0);
        assert!(r.reaches(1, 0));
    }

    #[test]
    fn try_add_edge_refuses_an_edge_that_would_close_a_cycle() {
        let mut r = Reachability::new(3);
        r.add_edge(0, 1);
        r.add_edge(1, 2);
        assert!(!r.try_add_edge(2, 0));
        assert!(!r.reaches(2, 0));
    }

    #[test]
    fn try_add_edge_refuses_a_self_loop() {
        let mut r = Reachability::new(1);
        assert!(!r.try_add_edge(0, 0));
    }

    #[test]
    fn try_add_edge_accepts_an_edge_that_does_not_close_a_cycle() {
        let mut r = Reachability::new(3);
        r.add_edge(0, 1);
        assert!(r.try_add_edge(1, 2));
        assert!(r.reaches(0, 2));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p ecs reachability`
Expected: FAIL — `cannot find type `Reachability``. Add `mod reachability;` to `crates/ecs/src/system/mod.rs` first if the module is not compiled at all.

- [ ] **Step 3: Write the implementation**

Prepend to `crates/ecs/src/system/reachability.rs`:

```rust
/// Transitive reachability over a growing DAG, backed by one bitset per node.
pub(crate) struct Reachability {
    node_count: usize,
    words_per_node: usize,
    bits: Vec<u64>,
}

impl Reachability {
    pub(crate) fn new(node_count: usize) -> Self {
        let words_per_node = node_count.div_ceil(64);
        Self {
            node_count,
            words_per_node,
            bits: vec![0; node_count * words_per_node],
        }
    }

    pub(crate) fn reaches(&self, from: usize, to: usize) -> bool {
        self.bits[from * self.words_per_node + to / 64] & (1 << (to % 64)) != 0
    }

    fn set(&mut self, from: usize, to: usize) {
        self.bits[from * self.words_per_node + to / 64] |= 1 << (to % 64);
    }

    /// Records `from -> to`, propagating `to`'s descendants to every ancestor of `from`.
    pub(crate) fn add_edge(&mut self, from: usize, to: usize) {
        let mut closure: Vec<u64> = self.bits
            [to * self.words_per_node..(to + 1) * self.words_per_node]
            .to_vec();
        closure[to / 64] |= 1 << (to % 64);

        for node in 0..self.node_count {
            if node == from || self.reaches(node, from) {
                let base = node * self.words_per_node;
                for word in 0..self.words_per_node {
                    self.bits[base + word] |= closure[word];
                }
            }
        }
    }

    /// Records `from -> to` unless it would close a cycle; returns whether it was recorded.
    pub(crate) fn try_add_edge(&mut self, from: usize, to: usize) -> bool {
        if from == to || self.reaches(to, from) {
            return false;
        }
        self.add_edge(from, to);
        true
    }
}
```

Add `mod reachability;` to the module list at the top of `crates/ecs/src/system/mod.rs`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p ecs reachability`
Expected: PASS (7 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/ecs/src/system/reachability.rs crates/ecs/src/system/mod.rs
git commit -m "Add cycle-safe reachability tracking for schedule compilation"
```

---

### Task 3: Reference-based ordering and two-pass compile

This is the core change. After it, `.after`/`.before` reference systems instead of owning them, and the graph is built in `compile`.

**Files:**
- Modify: `crates/ecs/src/system/config.rs` (rewrite)
- Modify: `crates/ecs/src/system/schedule.rs` (rewrite `Schedule` and `compile`, replace the graph-structure tests)
- Modify: `crates/ecs/src/system/mod.rs` (re-export `DependencyTarget`, `IntoDependencyTarget`)
- Test: `crates/ecs/src/system/schedule.rs` (inline `mod tests`), `crates/ecs/tests/ordering.rs`

**Interfaces:**
- Consumes: `Reachability` from Task 2; `InternedSystemSet` from Task 1.
- Produces:
  - `pub enum DependencyTarget { System(TypeId), Set(InternedSystemSet) }`
  - `pub trait IntoDependencyTarget<Marker> { fn into_target(self) -> DependencyTarget; }` with marker types `SystemTarget<M>` and `SetTarget`
  - `SystemConfig { system: BoxedSystem, sets: Vec<InternedSystemSet>, after: Vec<DependencyTarget>, before: Vec<DependencyTarget> }` (fields `pub(crate)`)
  - `IntoSystemConfig::after<M>(self, target: impl IntoDependencyTarget<M>) -> SystemConfig`, same for `before`, plus `in_set(self, set: impl SystemSet) -> SystemConfig`

- [ ] **Step 1: Write the failing integration test**

Create `crates/ecs/tests/ordering.rs`:

```rust
use ecs::{
    Resource, World,
    resource::ResMut,
    system::{IntoSystemConfig, executor::single_thread::SingleThreadedExecutor},
    Schedule,
};

#[derive(Resource, Default)]
struct Log(Vec<&'static str>);

fn first(mut log: ResMut<Log>) {
    log.0.push("first");
}

fn second(mut log: ResMut<Log>) {
    log.0.push("second");
}

fn run(schedule: Schedule) -> Vec<&'static str> {
    let mut world = World::new();
    world.insert_resource(Log::default());
    schedule
        .compile::<SingleThreadedExecutor>(&mut world)
        .run(&mut world);
    world.remove_resource::<Log>().unwrap().0
}

#[test]
fn a_referenced_system_runs_once() {
    let mut schedule = Schedule::new();
    schedule.add_system(first);
    schedule.add_system(second.after(first));

    assert_eq!(run(schedule), vec!["first", "second"]);
}

#[test]
fn a_system_referenced_by_two_constraints_runs_once() {
    let mut schedule = Schedule::new();
    schedule.add_system(first);
    schedule.add_system(second.after(first));
    schedule.add_system(|mut log: ResMut<Log>| log.0.push("third"));

    let order = run(schedule);
    assert_eq!(order.iter().filter(|name| **name == "first").count(), 1);
}

#[test]
fn a_constraint_can_name_a_system_registered_later() {
    let mut schedule = Schedule::new();
    schedule.add_system(second.after(first));
    schedule.add_system(first);

    assert_eq!(run(schedule), vec!["first", "second"]);
}

#[test]
fn an_explicit_constraint_beats_registration_order() {
    let mut schedule = Schedule::new();
    schedule.add_system(second);
    schedule.add_system(first.before(second));

    assert_eq!(run(schedule), vec!["first", "second"]);
}
```

`a_referenced_system_runs_once` and `a_constraint_can_name_a_system_registered_later` are the two that fail against today's code, for different reasons.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p ecs --test ordering`
Expected: FAIL. `a_referenced_system_runs_once` gives `["first", "first", "second"]` (the duplicate), and `an_explicit_constraint_beats_registration_order` panics with the cycle `expect`.

- [ ] **Step 3: Rewrite `config.rs`**

Replace the contents of `crates/ecs/src/system/config.rs` with:

```rust
use std::any::TypeId;

use crate::system::{
    BoxedSystem, IntoSystem,
    set::{InternedSystemSet, SystemSet},
};

/// Something a system can be ordered against: another system, or a set.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum DependencyTarget {
    System(TypeId),
    Set(InternedSystemSet),
}

/// Marker for the [`IntoDependencyTarget`] impl covering systems.
pub struct SystemTarget<M>(M);

/// Marker for the [`IntoDependencyTarget`] impl covering sets.
pub struct SetTarget;

/// Converts a system function or a [`SystemSet`] into a [`DependencyTarget`].
pub trait IntoDependencyTarget<Marker> {
    fn into_target(self) -> DependencyTarget;
}

impl<M, S: IntoSystem<M> + 'static> IntoDependencyTarget<SystemTarget<M>> for S {
    fn into_target(self) -> DependencyTarget {
        DependencyTarget::System(self.into_system().system_type())
    }
}

impl<S: SystemSet> IntoDependencyTarget<SetTarget> for S {
    fn into_target(self) -> DependencyTarget {
        DependencyTarget::Set(self.intern())
    }
}

/// A system bundled with its set memberships and ordering constraints.
pub struct SystemConfig {
    pub(crate) system: BoxedSystem,
    pub(crate) sets: Vec<InternedSystemSet>,
    pub(crate) after: Vec<DependencyTarget>,
    pub(crate) before: Vec<DependencyTarget>,
}

impl SystemConfig {
    /// Declares that `target` must run before this system.
    pub fn after<M>(mut self, target: impl IntoDependencyTarget<M>) -> Self {
        self.after.push(target.into_target());
        self
    }

    /// Declares that `target` must run after this system.
    pub fn before<M>(mut self, target: impl IntoDependencyTarget<M>) -> Self {
        self.before.push(target.into_target());
        self
    }

    /// Adds this system to `set`.
    pub fn in_set(mut self, set: impl SystemSet) -> Self {
        self.sets.push(set.intern());
        self
    }
}

/// Converts a system function or [`SystemConfig`] into a [`SystemConfig`].
pub trait IntoSystemConfig<Marker>: Sized {
    fn into_config(self) -> SystemConfig;

    /// Declares that `target` must run before this system.
    fn after<M>(self, target: impl IntoDependencyTarget<M>) -> SystemConfig {
        self.into_config().after(target)
    }

    /// Declares that `target` must run after this system.
    fn before<M>(self, target: impl IntoDependencyTarget<M>) -> SystemConfig {
        self.into_config().before(target)
    }

    /// Adds this system to `set`.
    fn in_set(self, set: impl SystemSet) -> SystemConfig {
        self.into_config().in_set(set)
    }
}

impl<M, F: IntoSystem<M> + 'static> IntoSystemConfig<M> for F {
    fn into_config(self) -> SystemConfig {
        SystemConfig {
            system: self.into_system(),
            sets: Vec::new(),
            after: Vec::new(),
            before: Vec::new(),
        }
    }
}

/// Marker type used to implement [`IntoSystemConfig`] for [`SystemConfig`] itself.
pub struct AlreadyConfigured;

impl IntoSystemConfig<AlreadyConfigured> for SystemConfig {
    fn into_config(self) -> SystemConfig {
        self
    }
}
```

Note `System::system_type()` is already on the `System` trait (`crates/ecs/src/system/mod.rs:35`) and is forwarded by the `BoxedSystem` impl, so `self.into_system().system_type()` gives the stable per-function `TypeId`. The temporary box is dropped immediately.

In `crates/ecs/src/system/mod.rs`, extend the `pub use config::...` line to include `DependencyTarget, IntoDependencyTarget, SetTarget, SystemTarget`.

- [ ] **Step 4: Rewrite `Schedule` registration**

In `crates/ecs/src/system/schedule.rs`, replace the `Schedule` struct, `add_system`, `add_config` and `add_sync_point` with:

```rust
/// A system plus everything it declared, before the graph exists.
struct NodeConfig {
    sets: Vec<InternedSystemSet>,
    after: Vec<DependencyTarget>,
    before: Vec<DependencyTarget>,
}

#[derive(Default)]
pub struct Schedule {
    systems: Vec<BoxedSystem>,
    configs: Vec<NodeConfig>,
}

impl Schedule {
    /// Creates an empty schedule.
    pub fn new() -> Schedule {
        Self {
            systems: Vec::new(),
            configs: Vec::new(),
        }
    }

    /// Adds a system (or [`SystemConfig`]) to the schedule.
    pub fn add_system<M>(&mut self, system: impl IntoSystemConfig<M> + 'static) -> &mut Self {
        let config = system.into_config();
        self.systems.push(config.system);
        self.configs.push(NodeConfig {
            sets: config.sets,
            after: config.after,
            before: config.before,
        });
        self
    }
}
```

Delete the `system_ids` and `graph` fields, the `SystemNodeIndex` uses in registration, and the whole old `add_config` body. Keep `SystemNodeIndex` and `SystemIndex` — `compile` still uses them.

Replace `Schedule`'s `Debug` impl (it printed the graph, which no longer exists at this point) with one that prints system names:

```rust
impl fmt::Debug for Schedule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.systems.iter().map(|system| system.name()))
            .finish()
    }
}
```

- [ ] **Step 5: Write the two-pass `compile`**

Replace `Schedule::compile` in `crates/ecs/src/system/schedule.rs` with the following. `resolve_sets` is a stub here returning an empty map — Task 4 fills it in.

```rust
impl Schedule {
    pub fn compile<T: SystemExecutor + 'static>(mut self, world: &mut World) -> CompiledSchedule {
        self.insert_sync_points();

        let mut graph = SystemDependencyGraph::new();
        let nodes: Vec<SystemNodeIndex> = self
            .systems
            .iter()
            .enumerate()
            .map(|(index, system)| {
                let mut access = SystemAccess::default();
                let mut metadata = SystemMetadata::default();
                system.fill_access(&mut metadata, &mut access);
                graph
                    .add_node(SystemNode::new(index.into(), access, metadata, system.name()))
                    .into()
            })
            .collect();

        let by_type = self.index_by_type();
        let by_set = self.index_by_set();
        let mut reachability = Reachability::new(self.systems.len());

        for (index, config) in self.configs.iter().enumerate() {
            for target in &config.after {
                for source in resolve(target, &by_type, &by_set, self.systems[index].name()) {
                    add_explicit_edge(&mut graph, &nodes, &mut reachability, source, index);
                }
            }
            for target in &config.before {
                for sink in resolve(target, &by_type, &by_set, self.systems[index].name()) {
                    add_explicit_edge(&mut graph, &nodes, &mut reachability, index, sink);
                }
            }
        }

        self.add_implicit_edges(&mut graph, &nodes, &mut reachability);

        let compiled_data = self.build_compiled_data(&graph, &nodes, world);

        CompiledSchedule {
            executor: Box::new(T::init(&compiled_data)),
            compiled_data,
            graph,
        }
    }
}
```

with these helpers in the same `impl` block (or as free functions in the module, as the borrow checker prefers):

```rust
fn add_explicit_edge(
    graph: &mut SystemDependencyGraph,
    nodes: &[SystemNodeIndex],
    reachability: &mut Reachability,
    from: usize,
    to: usize,
) {
    if from == to {
        return;
    }
    if reachability.reaches(to, from) {
        panic!(
            "Cycle in schedule ordering: {} and {} are each constrained to run before the other",
            graph.node_weight(*nodes[from]).unwrap().name,
            graph.node_weight(*nodes[to]).unwrap().name,
        );
    }
    graph.add_edge(*nodes[from], *nodes[to], ());
    reachability.add_edge(from, to);
}
```

`from == to` covers Review Focus item 4: a self-referential constraint is dropped, not turned into a self-loop.

```rust
impl Schedule {
    fn index_by_type(&self) -> HashMap<TypeId, Vec<usize>> {
        let mut index: HashMap<TypeId, Vec<usize>> = HashMap::new();
        for (position, system) in self.systems.iter().enumerate() {
            if is_sync_point(system.as_ref()) {
                continue;
            }
            index.entry(system.system_type()).or_default().push(position);
        }
        index
    }

    fn insert_sync_points(&mut self) {
        let mut systems = Vec::with_capacity(self.systems.len());
        let mut configs = Vec::with_capacity(self.configs.len());

        for (system, config) in self.systems.drain(..).zip(self.configs.drain(..)) {
            let mut access = SystemAccess::default();
            let mut metadata = SystemMetadata::default();
            system.fill_access(&mut metadata, &mut access);
            let needs_sync = access.needs_apply() && !is_sync_point(system.as_ref());

            systems.push(system);
            configs.push(config);

            if needs_sync {
                systems.push(Box::new(SyncPoint));
                configs.push(NodeConfig {
                    sets: Vec::new(),
                    after: Vec::new(),
                    before: Vec::new(),
                });
            }
        }

        self.systems = systems;
        self.configs = configs;
    }

    fn add_implicit_edges(
        &self,
        graph: &mut SystemDependencyGraph,
        nodes: &[SystemNodeIndex],
        reachability: &mut Reachability,
    ) {
        let access: Vec<SystemAccess> = self
            .systems
            .iter()
            .map(|system| {
                let mut access = SystemAccess::default();
                let mut metadata = SystemMetadata::default();
                system.fill_access(&mut metadata, &mut access);
                access
            })
            .collect();

        for later in 0..self.systems.len() {
            for earlier in 0..later {
                if SystemAccess::are_disjoint(&access[earlier], &access[later]) {
                    continue;
                }
                if reachability.try_add_edge(earlier, later) {
                    graph.add_edge(*nodes[earlier], *nodes[later], ());
                }
            }
        }
    }
}
```

`try_add_edge` returning `false` is exactly the explicit-wins rule, and because it refuses any edge that would close a cycle it is transitive — the sync-point case in the spec cannot panic.

`build_compiled_data` is the existing body of `compile` from `dependency_count` through `sorted_systems` and `initialize`, moved verbatim into a method that takes `&graph` and `&nodes` and consumes `self.systems`. The `toposort` `expect` can stay: the graph is now acyclic by construction, and explicit cycles already panicked with a readable message in `add_explicit_edge`.

Add the imports these need at the top of `schedule.rs`: `crate::system::{reachability::Reachability, set::InternedSystemSet, config::DependencyTarget, sync_point::SyncPoint}` and `std::any::TypeId` (already present).

- [ ] **Step 6: Write `resolve` with the warning path**

Add to `crates/ecs/src/system/schedule.rs`:

```rust
fn resolve(
    target: &DependencyTarget,
    by_type: &HashMap<TypeId, Vec<usize>>,
    by_set: &HashMap<InternedSystemSet, Vec<usize>>,
    owner: &str,
) -> Vec<usize> {
    match target {
        DependencyTarget::System(type_id) => match by_type.get(type_id) {
            Some(matches) => {
                if matches.len() > 1 {
                    log::warn!(
                        "`{owner}` is ordered against a system registered {} times in this schedule; ordering against all copies",
                        matches.len()
                    );
                }
                matches.clone()
            }
            None => {
                log::warn!(
                    "`{owner}` is ordered against a system that is not in this schedule; the constraint is ignored"
                );
                Vec::new()
            }
        },
        DependencyTarget::Set(set) => by_set.get(set).cloned().unwrap_or_default(),
    }
}
```

A missing *system* warns (Review Focus items 2 and 3); a set with no members is silent (Review Focus item 1), because a plugin contributing no systems to a set it named is normal.

For this task, `index_by_set` returns an empty map:

```rust
fn index_by_set(&self) -> HashMap<InternedSystemSet, Vec<usize>> {
    HashMap::new()
}
```

Task 4 replaces it. Add `log` to `crates/ecs/Cargo.toml` if it is not already a dependency.

- [ ] **Step 7: Replace the graph-structure unit tests**

In `crates/ecs/src/system/schedule.rs`'s `mod tests`, the tests `after_registers_dep_and_main`, `after_creates_dep_to_main_edge`, `before_registers_dep_and_main`, `before_creates_main_to_dep_edge`, `after_before_chain_has_correct_edges` and `nested_after_chain_has_correct_edges` all assert the old auto-registration behaviour and must be deleted — they assert the bug. `schedule_new`, `system_dependency_graph_creation` and `multiple_systems_added` reference `schedule.graph`, which no longer exists before compile; rewrite them against `schedule.systems.len()`.

Replace the deleted ones with:

```rust
#[test]
fn a_constraint_does_not_register_its_target() {
    fn dep() {}
    fn main_sys() {}

    let mut schedule = Schedule::new();
    schedule.add_system(main_sys.after(dep));

    assert_eq!(schedule.systems.len(), 1);
}

#[test]
fn a_system_added_twice_is_two_nodes() {
    fn sys() {}

    let mut schedule = Schedule::new();
    schedule.add_system(sys).add_system(sys);

    assert_eq!(schedule.systems.len(), 2);
}

#[test]
fn a_self_referential_constraint_does_not_deadlock_compile() {
    fn sys() {}

    let mut schedule = Schedule::new();
    schedule.add_system(sys.after(sys));

    let mut world = World::new();
    schedule
        .compile::<SingleThreadedExecutor>(&mut world)
        .run(&mut world);
}

#[test]
#[should_panic(expected = "Cycle in schedule ordering")]
fn contradicting_explicit_constraints_panic_with_both_names() {
    fn a() {}
    fn b() {}

    let mut schedule = Schedule::new();
    schedule.add_system(a.before(b));
    schedule.add_system(b.before(a));

    let mut world = World::new();
    let _ = schedule.compile::<SingleThreadedExecutor>(&mut world);
}
```

Also add the cross-schedule case (Review Focus item 3) to `crates/ecs/tests/ordering.rs`:

```rust
#[test]
fn a_target_in_another_schedule_is_ignored() {
    let mut schedule = Schedule::new();
    schedule.add_system(second.after(first));

    assert_eq!(run(schedule), vec!["second"]);
}

#[test]
fn a_constraint_orders_against_every_copy_of_a_twice_added_target() {
    let mut schedule = Schedule::new();
    schedule.add_system(second.after(first));
    schedule.add_system(first);
    schedule.add_system(first);

    let order = run(schedule);
    assert_eq!(order, vec!["first", "first", "second"]);
}
```

`a_constraint_orders_against_every_copy_of_a_twice_added_target` is Review Focus item 2: `second` must follow *both* copies, not just the first one `resolve` happened to find.

- [ ] **Step 8: Run the tests**

Run: `cargo test -p ecs`
Expected: PASS, including all four tests in `tests/ordering.rs`.

- [ ] **Step 9: Fix the call sites that used owned deps**

`cargo build --workspace` will now fail wherever a `.after` argument was relied on to register its target. The audit found these, and they need no change in signature (they still compile — the target is resolved by reference), but verify each still has its target registered in the *same* schedule:

- `crates/render/src/plugin.rs:187-189`
- `crates/render/src/shadow_pipeline.rs:30`
- `crates/terminal-renderer/src/plugin.rs:23`

Run the build and read any warning about an ignored constraint; each one is a real ordering that is now silently missing and must be fixed in Task 6.

Run: `cargo build --workspace && cargo test --workspace`
Expected: PASS.

- [ ] **Step 10: Commit**

```bash
git add crates/ecs/src/system/config.rs crates/ecs/src/system/schedule.rs \
        crates/ecs/src/system/mod.rs crates/ecs/tests/ordering.rs crates/ecs/Cargo.toml
git commit -m "Order systems by reference and build the schedule graph in compile"
```

---

### Task 4: Set membership and `configure_sets`

**Files:**
- Modify: `crates/ecs/src/system/set.rs` (add `SetConfig`, `IntoSetConfigs`, `chain`)
- Modify: `crates/ecs/src/system/schedule.rs` (`index_by_set`, `configure_sets`, set-aware cycle message)
- Modify: `crates/app/src/lib.rs`, `crates/app/src/subapp.rs` (`configure_sets`, `configure_render_sets`)
- Test: `crates/ecs/tests/ordering.rs`

**Interfaces:**
- Consumes: `SystemSet`/`InternedSystemSet` (Task 1), `DependencyTarget`/`IntoDependencyTarget` (Task 3).
- Produces:
  - `pub struct SetConfig { set: InternedSystemSet, after: Vec<DependencyTarget>, before: Vec<DependencyTarget> }`
  - `SystemSet::after`/`before` returning `SetConfig` (via an extension trait `IntoSetConfig`, so the label trait itself stays object-safe)
  - `pub trait IntoSetConfigs { fn into_set_configs(self) -> Vec<SetConfig>; fn chain(self) -> Vec<SetConfig>; }` implemented for `SetConfig`, for any `SystemSet`, and for tuples of 2..=12 sets
  - `Schedule::configure_sets(&mut self, configs: impl IntoSetConfigs)`, `Schedules::configure_sets(label, configs)`, `App::configure_sets(label, configs)`, `App::configure_render_sets(label, configs)`, `SubApp::configure_sets(label, configs)`

- [ ] **Step 1: Write the failing test**

Append to `crates/ecs/tests/ordering.rs`:

```rust
use ecs::{SystemSet, system::set::IntoSetConfigs};

#[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
enum Phase {
    Early,
    Late,
    Unused,
}

fn third(mut log: ResMut<Log>) {
    log.0.push("third");
}

#[test]
fn sets_order_their_members() {
    let mut schedule = Schedule::new();
    schedule.configure_sets((Phase::Early, Phase::Late).chain());
    schedule.add_system(second.in_set(Phase::Late));
    schedule.add_system(first.in_set(Phase::Early));

    assert_eq!(run(schedule), vec!["first", "second"]);
}

#[test]
fn a_system_can_be_ordered_against_a_set() {
    let mut schedule = Schedule::new();
    schedule.add_system(second.in_set(Phase::Late));
    schedule.add_system(first.before(Phase::Late));

    assert_eq!(run(schedule), vec!["first", "second"]);
}

#[test]
fn an_empty_set_named_as_a_target_is_ignored() {
    let mut schedule = Schedule::new();
    schedule.add_system(first.before(Phase::Unused));

    assert_eq!(run(schedule), vec!["first"]);
}

#[test]
fn a_system_in_a_set_is_not_self_ordered_by_that_set() {
    let mut schedule = Schedule::new();
    schedule.configure_sets(Phase::Late.after(Phase::Early));
    schedule.add_system(first.in_set(Phase::Early).in_set(Phase::Late));
    schedule.add_system(second.in_set(Phase::Late));

    assert_eq!(run(schedule), vec!["first", "second"]);
}

#[test]
#[should_panic(expected = "Cycle in schedule ordering")]
fn a_cycle_created_by_set_expansion_panics() {
    let mut schedule = Schedule::new();
    schedule.configure_sets((Phase::Early, Phase::Late).chain());
    schedule.add_system(first.in_set(Phase::Early).after(second));
    schedule.add_system(second.in_set(Phase::Late));

    let mut world = World::new();
    world.insert_resource(Log::default());
    let _ = schedule.compile::<SingleThreadedExecutor>(&mut world);
}
```

`a_system_in_a_set_is_not_self_ordered_by_that_set` is Review Focus item 4 via sets: `first` is in both sets, so expansion produces `first -> first`, which `add_explicit_edge` must drop rather than panic on.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p ecs --test ordering`
Expected: FAIL — `no method named `configure_sets``.

- [ ] **Step 3: Add `SetConfig` and `IntoSetConfigs`**

Append to `crates/ecs/src/system/set.rs`:

```rust
use crate::system::config::{DependencyTarget, IntoDependencyTarget};

/// A set together with its ordering constraints.
pub struct SetConfig {
    pub(crate) set: InternedSystemSet,
    pub(crate) after: Vec<DependencyTarget>,
    pub(crate) before: Vec<DependencyTarget>,
}

impl SetConfig {
    /// Declares that `target` must run before every member of this set.
    pub fn after<M>(mut self, target: impl IntoDependencyTarget<M>) -> Self {
        self.after.push(target.into_target());
        self
    }

    /// Declares that `target` must run after every member of this set.
    pub fn before<M>(mut self, target: impl IntoDependencyTarget<M>) -> Self {
        self.before.push(target.into_target());
        self
    }
}

/// Turns a set into a configurable [`SetConfig`].
pub trait IntoSetConfig: Sized {
    fn into_set_config(self) -> SetConfig;

    fn after<M>(self, target: impl IntoDependencyTarget<M>) -> SetConfig {
        self.into_set_config().after(target)
    }

    fn before<M>(self, target: impl IntoDependencyTarget<M>) -> SetConfig {
        self.into_set_config().before(target)
    }
}

impl<S: SystemSet> IntoSetConfig for S {
    fn into_set_config(self) -> SetConfig {
        SetConfig {
            set: self.intern(),
            after: Vec::new(),
            before: Vec::new(),
        }
    }
}

/// One or more [`SetConfig`]s, optionally chained into a sequence.
pub trait IntoSetConfigs {
    fn into_set_configs(self) -> Vec<SetConfig>;

    /// Orders the configs so each runs before the next.
    fn chain(self) -> Vec<SetConfig>
    where
        Self: Sized,
    {
        let mut configs = self.into_set_configs();
        for index in 1..configs.len() {
            let previous = configs[index - 1].set;
            configs[index].after.push(DependencyTarget::Set(previous));
        }
        configs
    }
}

impl IntoSetConfigs for SetConfig {
    fn into_set_configs(self) -> Vec<SetConfig> {
        vec![self]
    }
}

impl IntoSetConfigs for Vec<SetConfig> {
    fn into_set_configs(self) -> Vec<SetConfig> {
        self
    }
}

macro_rules! impl_into_set_configs_for_tuple {
    ($($name:ident),*) => {
        impl<$($name: IntoSetConfig),*> IntoSetConfigs for ($($name,)*) {
            #[allow(non_snake_case)]
            fn into_set_configs(self) -> Vec<SetConfig> {
                let ($($name,)*) = self;
                vec![$($name.into_set_config()),*]
            }
        }
    };
}

impl_into_set_configs_for_tuple!(A, B);
impl_into_set_configs_for_tuple!(A, B, C);
impl_into_set_configs_for_tuple!(A, B, C, D);
impl_into_set_configs_for_tuple!(A, B, C, D, E);
impl_into_set_configs_for_tuple!(A, B, C, D, E, F);
impl_into_set_configs_for_tuple!(A, B, C, D, E, F, G);
impl_into_set_configs_for_tuple!(A, B, C, D, E, F, G, H);
impl_into_set_configs_for_tuple!(A, B, C, D, E, F, G, H, I);
impl_into_set_configs_for_tuple!(A, B, C, D, E, F, G, H, I, J);
impl_into_set_configs_for_tuple!(A, B, C, D, E, F, G, H, I, J, K);
impl_into_set_configs_for_tuple!(A, B, C, D, E, F, G, H, I, J, K, L);
```

A bare `SystemSet` also needs to work as an argument to `configure_sets`; that is covered because `IntoSetConfigs for SetConfig` plus `IntoSetConfig for S` means callers write `Phase::Late.after(..)`. For the bare case, add:

```rust
impl<S: SystemSet> IntoSetConfigs for S {
    fn into_set_configs(self) -> Vec<SetConfig> {
        vec![self.into_set_config()]
    }
}
```

If that collides with the `SetConfig` impl under coherence (it will not — `SetConfig` does not implement `SystemSet`), keep both.

- [ ] **Step 4: Store and resolve set memberships**

In `crates/ecs/src/system/schedule.rs`, add `set_configs: Vec<SetConfig>` to `Schedule`, initialise it to `Vec::new()` in `Schedule::new` (the `#[derive(Default)]` covers `Default`), and add:

```rust
impl Schedule {
    /// Declares ordering between sets in this schedule.
    pub fn configure_sets(&mut self, configs: impl IntoSetConfigs) -> &mut Self {
        self.set_configs.extend(configs.into_set_configs());
        self
    }

    fn index_by_set(&self) -> HashMap<InternedSystemSet, Vec<usize>> {
        let mut index: HashMap<InternedSystemSet, Vec<usize>> = HashMap::new();
        for (position, config) in self.configs.iter().enumerate() {
            for set in &config.sets {
                index.entry(*set).or_default().push(position);
            }
        }
        index
    }
}
```

`insert_sync_points` shifts positions, so `index_by_set` and `index_by_type` must both be called *after* it — they already are, in `compile`.

In `compile`, after the per-system constraint loop, add the per-set loop:

```rust
for set_config in &self.set_configs {
    let members = by_set.get(&set_config.set).cloned().unwrap_or_default();
    if members.is_empty() {
        continue;
    }
    let owner = format!("{:?}", set_config.set);
    for target in &set_config.after {
        for source in resolve(target, &by_type, &by_set, &owner) {
            for member in &members {
                add_explicit_edge(&mut graph, &nodes, &mut reachability, source, *member);
            }
        }
    }
    for target in &set_config.before {
        for sink in resolve(target, &by_type, &by_set, &owner) {
            for member in &members {
                add_explicit_edge(&mut graph, &nodes, &mut reachability, *member, sink);
            }
        }
    }
}
```

`resolve` is reused unchanged — its `owner` parameter is already `&str`, and passing the set's `Debug` form makes an ignored constraint name the set rather than a system.

`add_explicit_edge`'s panic message is unchanged. A set-expanded cycle still names the two systems that actually conflict, which is what a reader needs in order to fix it, and it is what `a_cycle_created_by_set_expansion_panics` asserts.

- [ ] **Step 5: Plumb `configure_sets` through `Schedules`, `SubApp` and `App`**

In `crates/ecs/src/system/schedule.rs`, on `Schedules`:

```rust
/// Declares ordering between sets in the schedule identified by `label`.
pub fn configure_sets(&mut self, label: impl ScheduleLabel, configs: impl IntoSetConfigs) {
    self.schedules
        .entry(label.intern())
        .or_default()
        .configure_sets(configs);
}
```

In `crates/app/src/subapp.rs`, mirroring the existing `SubApp::add_system`:

```rust
/// Declares ordering between sets in the schedule identified by `update_group`.
pub fn configure_sets(
    &mut self,
    update_group: impl ScheduleLabel,
    configs: impl IntoSetConfigs,
) -> &mut Self {
    self.get_resource_mut::<Schedules>()
        .expect("Schedules resource not found!")
        .configure_sets(update_group, configs);
    self
}
```

In `crates/app/src/lib.rs`, mirroring `App::add_system` and `App::add_render_system`, add `App::configure_sets` (delegating to `self.main_mut()`) and `App::configure_render_sets` (delegating to `self.subapps.render_mut()`).

- [ ] **Step 6: Run the tests**

Run: `cargo test -p ecs --test ordering`
Expected: PASS (all nine tests).

- [ ] **Step 7: Verify the workspace**

Run: `cargo build --workspace && cargo test --workspace`
Expected: PASS, no warnings.

- [ ] **Step 8: Commit**

```bash
git add crates/ecs/src/system/set.rs crates/ecs/src/system/schedule.rs \
        crates/app/src/lib.rs crates/app/src/subapp.rs crates/ecs/tests/ordering.rs
git commit -m "Add system sets and configure_sets"
```

---

### Task 5: `add_systems` for tuples

**Files:**
- Modify: `crates/ecs/src/system/config.rs` (add `IntoSystemConfigs`)
- Modify: `crates/ecs/src/system/schedule.rs` (`Schedule::add_systems`, `Schedules::add_systems`)
- Modify: `crates/app/src/lib.rs`, `crates/app/src/subapp.rs` (`add_systems`, `add_render_systems`)
- Test: `crates/ecs/tests/ordering.rs`

**Interfaces:**
- Consumes: `SystemConfig`, `IntoSystemConfig` (Task 3).
- Produces: `pub trait IntoSystemConfigs<Marker> { fn into_configs(self) -> Vec<SystemConfig>; fn in_set(self, set: impl SystemSet) -> Vec<SystemConfig>; fn chain(self) -> Vec<SystemConfig>; }` implemented for `Vec<SystemConfig>`, for anything implementing `IntoSystemConfig`, and for tuples of 2..=12; `Schedule::add_systems`, `Schedules::add_systems(label, systems)`, `App::add_systems(label, systems)`, `App::add_render_systems(label, systems)`, `SubApp::add_systems(label, systems)`.

Tuple impls use a declarative macro over arities rather than `typle`, because each element carries its own `IntoSystemConfig` marker type and `typle` does not express a second parallel marker tuple cleanly.

- [ ] **Step 1: Write the failing test**

Append to `crates/ecs/tests/ordering.rs`:

```rust
use ecs::system::config::IntoSystemConfigs;

#[test]
fn a_tuple_of_systems_can_join_one_set() {
    let mut schedule = Schedule::new();
    schedule.configure_sets((Phase::Early, Phase::Late).chain());
    schedule.add_systems((second, third).in_set(Phase::Late));
    schedule.add_systems(first.in_set(Phase::Early));

    let order = run(schedule);
    assert_eq!(order[0], "first");
    assert_eq!(order.len(), 3);
}

#[test]
fn chain_orders_a_tuple_against_registration_order() {
    let mut schedule = Schedule::new();
    let configs: Vec<_> = (second, first).chain().into_iter().rev().collect();
    schedule.add_systems(configs);

    assert_eq!(run(schedule), vec!["second", "first"]);
}
```

`chain_orders_a_tuple_against_registration_order` is the load-bearing one. `.chain()` says `second` runs before `first`; `rev()` then registers them in the opposite order, so registration order alone would give `["first", "second"]`. Only a real chain edge gives `["second", "first"]`. It also exercises `IntoSystemConfigs for Vec<SystemConfig>`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p ecs --test ordering`
Expected: FAIL — `no method named `add_systems``.

- [ ] **Step 3: Implement `IntoSystemConfigs`**

Append to `crates/ecs/src/system/config.rs`:

```rust
/// One or more systems, optionally grouped into a set or chained in order.
pub trait IntoSystemConfigs<Marker> {
    fn into_configs(self) -> Vec<SystemConfig>;

    /// Adds every system to `set`.
    fn in_set(self, set: impl SystemSet) -> Vec<SystemConfig>
    where
        Self: Sized,
    {
        let interned = set.intern();
        let mut configs = self.into_configs();
        for config in &mut configs {
            config.sets.push(interned);
        }
        configs
    }

    /// Orders the systems so each runs before the next.
    fn chain(self) -> Vec<SystemConfig>
    where
        Self: Sized,
    {
        let mut configs = self.into_configs();
        for index in 1..configs.len() {
            let previous = DependencyTarget::System(configs[index - 1].system.system_type());
            configs[index].after.push(previous);
        }
        configs
    }
}

/// Marker for the single-system [`IntoSystemConfigs`] impl.
pub struct SingleConfig<M>(M);

impl<M, S: IntoSystemConfig<M>> IntoSystemConfigs<SingleConfig<M>> for S {
    fn into_configs(self) -> Vec<SystemConfig> {
        vec![self.into_config()]
    }
}

/// Marker for the `Vec<SystemConfig>` [`IntoSystemConfigs`] impl.
pub struct ConfigVec;

impl IntoSystemConfigs<ConfigVec> for Vec<SystemConfig> {
    fn into_configs(self) -> Vec<SystemConfig> {
        self
    }
}

macro_rules! impl_into_system_configs_for_tuple {
    ($(($name:ident, $marker:ident)),*) => {
        impl<$($name, $marker),*> IntoSystemConfigs<($($marker,)*)> for ($($name,)*)
        where
            $($name: IntoSystemConfig<$marker>,)*
        {
            #[allow(non_snake_case)]
            fn into_configs(self) -> Vec<SystemConfig> {
                let ($($name,)*) = self;
                vec![$($name.into_config()),*]
            }
        }
    };
}

impl_into_system_configs_for_tuple!((A, MA), (B, MB));
impl_into_system_configs_for_tuple!((A, MA), (B, MB), (C, MC));
impl_into_system_configs_for_tuple!((A, MA), (B, MB), (C, MC), (D, MD));
impl_into_system_configs_for_tuple!((A, MA), (B, MB), (C, MC), (D, MD), (E, ME));
impl_into_system_configs_for_tuple!((A, MA), (B, MB), (C, MC), (D, MD), (E, ME), (F, MF));
impl_into_system_configs_for_tuple!((A, MA), (B, MB), (C, MC), (D, MD), (E, ME), (F, MF), (G, MG));
impl_into_system_configs_for_tuple!((A, MA), (B, MB), (C, MC), (D, MD), (E, ME), (F, MF), (G, MG), (H, MH));
```

Seven arities (2..=8) cover every migration in Tasks 6 and 7; add more only if a call site needs one.

`chain` reads `config.system.system_type()`, which works because `SystemConfig` holds the boxed system.

`.chain()` exists on both `IntoSystemConfigs` and `IntoSetConfigs`. Sets and systems are distinct types, so there is no ambiguity at a call site, but both traits must be in scope where each is used.

- [ ] **Step 4: Add `add_systems` at each layer**

`Schedule::add_systems`:

```rust
/// Adds several systems to the schedule.
pub fn add_systems<M>(&mut self, systems: impl IntoSystemConfigs<M>) -> &mut Self {
    for config in systems.into_configs() {
        self.add_system(config);
    }
    self
}
```

`Schedules::add_systems(label, systems)` mirrors `Schedules::add_system`. `SubApp::add_systems` and `App::add_systems` / `App::add_render_systems` mirror their `add_system` counterparts exactly, delegating to the same resource lookup.

Keep `add_system` everywhere — it is used in roughly a hundred places and there is no reason to churn them.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p ecs --test ordering`
Expected: PASS.

- [ ] **Step 6: Verify the workspace**

Run: `cargo build --workspace && cargo test --workspace`
Expected: PASS, no warnings.

- [ ] **Step 7: Commit**

```bash
git add crates/ecs/src/system/config.rs crates/ecs/src/system/schedule.rs \
        crates/app/src/lib.rs crates/app/src/subapp.rs crates/ecs/tests/ordering.rs
git commit -m "Add add_systems for tuples with in_set and chain"
```

---

### Task 6: Migrate `render`

**Files:**
- Create: `crates/render/src/sets.rs`
- Modify: `crates/render/src/lib.rs` (add `pub mod sets;`)
- Modify: `crates/render/src/plugin.rs:180-190`
- Modify: `crates/render/src/shadow_pipeline.rs:28-31`
- Modify: `crates/terminal-renderer/src/plugin.rs:20-25`
- Test: `crates/render/tests/render_sets.rs`

**Interfaces:**
- Consumes: `SystemSet` derive (Task 1), `in_set`/`before`/`after` (Tasks 3-5), `App::configure_render_sets` (Task 4).
- Produces: `render::sets::RenderSet { Lights, Shadows, Draw, Present }`, public so `terminal-renderer` and downstream crates can order against it.

This task removes the duplicate executions: `update_changed_lights` currently runs three times per frame and `update_shadow_view_proj` twice.

- [ ] **Step 1: Write the failing test**

Create `crates/render/tests/render_sets.rs`:

```rust
use ecs::{
    Resource, Schedule, World,
    resource::ResMut,
    system::{IntoSystemConfig, executor::single_thread::SingleThreadedExecutor, set::IntoSetConfigs},
};
use render::sets::RenderSet;

#[derive(Resource, Default)]
struct Runs(Vec<&'static str>);

fn lights(mut runs: ResMut<Runs>) {
    runs.0.push("lights");
}

fn shadow_view_proj(mut runs: ResMut<Runs>) {
    runs.0.push("shadow_view_proj");
}

fn shadow_maps(mut runs: ResMut<Runs>) {
    runs.0.push("shadow_maps");
}

#[test]
fn the_shadow_chain_runs_each_system_once_in_order() {
    let mut schedule = Schedule::new();
    schedule.configure_sets((RenderSet::Lights, RenderSet::Shadows).chain());
    schedule.add_system(shadow_maps.in_set(RenderSet::Shadows).after(shadow_view_proj));
    schedule.add_system(shadow_view_proj.in_set(RenderSet::Shadows));
    schedule.add_system(lights.in_set(RenderSet::Lights));

    let mut world = World::new();
    world.insert_resource(Runs::default());
    schedule
        .compile::<SingleThreadedExecutor>(&mut world)
        .run(&mut world);

    assert_eq!(
        world.remove_resource::<Runs>().unwrap().0,
        vec!["lights", "shadow_view_proj", "shadow_maps"]
    );
}
```

This mirrors the real wiring (including the cross-plugin `.after` that `ShadowPipelinePlugin` performs) with stand-in systems, so it does not need a GPU.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p render --test render_sets`
Expected: FAIL — `could not find `sets` in `render``.

- [ ] **Step 3: Declare the sets**

Create `crates/render/src/sets.rs`:

```rust
use ecs::SystemSet;

/// The ordered phases of the render schedule.
#[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
pub enum RenderSet {
    Lights,
    Shadows,
    Draw,
    Present,
}
```

Add `pub mod sets;` to `crates/render/src/lib.rs`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p render --test render_sets`
Expected: PASS.

- [ ] **Step 5: Rewire `RenderPlugin`**

In `crates/render/src/plugin.rs`, replace lines 186-189:

```rust
app.configure_render_sets(
    Render,
    (RenderSet::Lights, RenderSet::Shadows, RenderSet::Draw).chain(),
)
.configure_render_sets(LateRender, RenderSet::Present)
.add_render_system(Render, clear_cameras)
.add_render_system(Render, update_changed_lights.in_set(RenderSet::Lights))
.add_render_systems(
    Render,
    (update_shadow_view_proj, resize_shadow_maps).in_set(RenderSet::Shadows),
)
.add_render_system(LateRender, present_window.in_set(RenderSet::Present).after(finish_render));
```

`update_changed_lights` is now registered exactly once, and the `.after(update_changed_lights)` on both shadow systems is expressed by the `Lights -> Shadows` chain instead.

- [ ] **Step 6: Rewire `ShadowPipelinePlugin`**

In `crates/render/src/shadow_pipeline.rs`, replace the `build` body's `add_system` call:

```rust
app.render_mut().add_system(
    Render,
    render_shadow_maps
        .in_set(RenderSet::Shadows)
        .after(update_shadow_view_proj),
);
```

`update_shadow_view_proj` is no longer re-registered here; the `.after` now references the copy `RenderPlugin` added. Update the imports: `update_shadow_view_proj` is still needed as an ordering target, so the `use` stays.

- [ ] **Step 7: Rewire `terminal-renderer`**

In `crates/terminal-renderer/src/plugin.rs:23`, `readback_terminal_frame.after(finish_render)` keeps its shape — it now references `finish_render` rather than registering a second copy. Add `.in_set(RenderSet::Present)` so it is ordered with `present_window` rather than incidentally. Add `render` to `crates/terminal-renderer/Cargo.toml` dependencies if it is not already there.

- [ ] **Step 8: Verify no constraint was dropped**

Run: `cargo build --workspace 2>&1 | grep -i "ignored"` and `RUST_LOG=warn cargo run -p render-test 2>&1 | grep -i "constraint is ignored"`
Expected: no output. A warning here means an ordering target is missing from its schedule — fix it before continuing.

- [ ] **Step 9: Verify the render output is unchanged**

Run: `cargo run -p render-test` (or the editor: `cargo run -p editor`) and confirm the scene, its lighting and its shadows look the same as on `master`. If `docs/profiling.md` describes a Tracy capture workflow, take one and confirm `update_changed_lights` and `update_shadow_view_proj` each appear once per frame rather than three and two times.

- [ ] **Step 10: Run the workspace tests and commit**

```bash
cargo build --workspace && cargo test --workspace
git add crates/render/src/sets.rs crates/render/src/lib.rs crates/render/src/plugin.rs \
        crates/render/src/shadow_pipeline.rs crates/terminal-renderer/src/plugin.rs \
        crates/terminal-renderer/Cargo.toml crates/render/tests/render_sets.rs
git commit -m "Order render systems with RenderSet and stop duplicating light and shadow work"
```

---

### Task 7: Migrate `ui`

**Files:**
- Create: `crates/ui/src/sets.rs`
- Modify: `crates/ui/src/lib.rs` (add `pub mod sets;`)
- Modify: `crates/ui/src/plugin.rs:101-146`
- Test: `crates/ui/tests/ui_sets.rs`

**Interfaces:**
- Consumes: everything from Tasks 1-5.
- Produces: `ui::sets::UiSet { Input, Widgets, Setup, Project, Materials, Layout, PostLayout }`, public so the editor, physics and animation can order against it later.

UI's systems stay in `LateUpdate`. The change is that their order is declared by sets rather than implied by line order.

- [ ] **Step 1: Write the failing test**

Create `crates/ui/tests/ui_sets.rs`:

```rust
use ecs::{
    Resource, Schedule, World,
    resource::ResMut,
    system::{IntoSystemConfig, executor::single_thread::SingleThreadedExecutor, set::IntoSetConfigs},
};
use ui::sets::UiSet;

#[derive(Resource, Default)]
struct Runs(Vec<&'static str>);

fn hit_test(mut runs: ResMut<Runs>) {
    runs.0.push("hit_test");
}

fn layout(mut runs: ResMut<Runs>) {
    runs.0.push("layout");
}

fn app_widget(mut runs: ResMut<Runs>) {
    runs.0.push("app_widget");
}

#[test]
fn a_consumer_can_slot_between_ui_sets_without_naming_a_ui_system() {
    let mut schedule = Schedule::new();
    schedule.configure_sets((UiSet::Input, UiSet::Widgets, UiSet::Layout).chain());
    schedule.add_system(layout.in_set(UiSet::Layout));
    schedule.add_system(hit_test.in_set(UiSet::Input));
    schedule.add_system(app_widget.after(UiSet::Input).before(UiSet::Layout));

    let mut world = World::new();
    world.insert_resource(Runs::default());
    schedule
        .compile::<SingleThreadedExecutor>(&mut world)
        .run(&mut world);

    assert_eq!(
        world.remove_resource::<Runs>().unwrap().0,
        vec!["hit_test", "app_widget", "layout"]
    );
}
```

This is the test for the thing the whole design exists to enable: ordering against `ui` from outside `ui`, without touching a private function and without depending on plugin registration order.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p ui --test ui_sets`
Expected: FAIL — `could not find `sets` in `ui``.

- [ ] **Step 3: Declare the sets**

Create `crates/ui/src/sets.rs`:

```rust
use ecs::SystemSet;

/// The ordered phases of the UI frame, from hit testing to layout.
#[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
pub enum UiSet {
    Input,
    Widgets,
    Setup,
    Project,
    Materials,
    Layout,
    PostLayout,
}
```

Add `pub mod sets;` to `crates/ui/src/lib.rs`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p ui --test ui_sets`
Expected: PASS.

- [ ] **Step 5: Rewire `UIPlugin`**

In `crates/ui/src/plugin.rs`, replace the whole block at lines 101-146 — including the long explanatory comment, which the sets now express — with:

```rust
app.configure_sets(
    LateUpdate,
    (
        UiSet::Input,
        UiSet::Widgets,
        UiSet::Setup,
        UiSet::Project,
        UiSet::Materials,
        UiSet::Layout,
        UiSet::PostLayout,
    )
        .chain(),
);

app.add_systems(
    LateUpdate,
    (update_ui_interaction, update_focus, sync_text_capture).in_set(UiSet::Input),
);
app.add_systems(
    LateUpdate,
    (
        toggle_checkboxes,
        update_text_inputs,
        update_widgets,
        sync_tab_bodies,
        update_tooltips,
        update_popup_menus,
        update_scroll_areas,
    )
        .in_set(UiSet::Widgets),
);
app.add_systems(
    LateUpdate,
    (update_virtual_lists, update_split_panes, update_slider_drag, drag_scrollbar_thumbs)
        .in_set(UiSet::Widgets),
);
app.add_systems(
    LateUpdate,
    (setup_slider_visuals, setup_scrollbars).in_set(UiSet::Setup),
);
app.add_systems(
    LateUpdate,
    (sync_slider_fill, sync_scroll_content, sync_split_panes).in_set(UiSet::Project),
);
app.add_systems(
    LateUpdate,
    (sync_checkbox_material, sync_viewport_textures, apply_interaction_styles)
        .in_set(UiSet::Materials),
);
app.add_systems(
    LateUpdate,
    (compute_ui_nodes, sync_material_params).in_set(UiSet::Layout),
);
app.add_systems(
    LateUpdate,
    (sync_scrollbar_tracks, sync_scrollbar_thumbs).in_set(UiSet::PostLayout),
);
```

Two orderings from the old comment block are *within* a set and so are no longer guaranteed by the set alone; both are preserved by registration order inside the set, which is unchanged, but pin the one the comment called out explicitly:

- `update_virtual_lists` must follow `update_scroll_areas`. They are both in `Widgets`, registered in that order, so the implicit edge holds. Make it explicit anyway: `update_virtual_lists.after(update_scroll_areas)` inside the second `Widgets` call, because this is the one the original comment flagged as load-bearing.
- `update_focus` reads what `update_ui_interaction` wrote. Both in `Input`, in order; add `.after(update_ui_interaction)` to `update_focus` for the same reason.

Apply those two `.after`s by splitting the affected systems out of their tuple:

```rust
app.add_systems(LateUpdate, update_ui_interaction.in_set(UiSet::Input));
app.add_systems(
    LateUpdate,
    (update_focus.after(update_ui_interaction), sync_text_capture).in_set(UiSet::Input),
);
```

- [ ] **Step 6: Verify no constraint was dropped**

Run: `RUST_LOG=warn cargo run -p editor 2>&1 | grep -i "constraint is ignored"`
Expected: no output.

- [ ] **Step 7: Verify the UI still behaves**

Run: `cargo run -p editor` and exercise the widgets the sets order: click a checkbox, drag a slider, scroll a scroll area (the content must move in the same frame), drag a split pane, open a popup menu, focus a text input and type. Each must behave as it does on `master`. If a change in behaviour appears, it means a within-set ordering the old line order provided was lost — find it and pin it with an explicit `.after`.

- [ ] **Step 8: Run the workspace tests and commit**

```bash
cargo build --workspace && cargo test --workspace
git add crates/ui/src/sets.rs crates/ui/src/lib.rs crates/ui/src/plugin.rs crates/ui/tests/ui_sets.rs
git commit -m "Order UI systems with UiSet instead of registration order"
```

---

### Task 8: Document the new API

**Files:**
- Modify: `plans/ecs-todo.md` (drop the line this work closes, if any)
- Create: `docs/scheduling.md`

**Interfaces:**
- Consumes: the finished API.
- Produces: nothing code depends on.

- [ ] **Step 1: Write the document**

Create `docs/scheduling.md` with exactly these sections, each carrying a compiling example lifted from the tests written in Tasks 3-7:

1. *Adding a system* — `app.add_system(Update, my_system)` and `app.add_systems(Update, (a, b, c))`.
2. *Ordering one system after another* — `b.after(a)`, with the sentence that `a` must be registered in the same schedule by someone, because `.after` references and no longer registers it.
3. *Declaring a set* — the `#[derive(SystemSet)]` enum from `crates/ui/src/sets.rs`, and why it is `pub`.
4. *Ordering sets* — `app.configure_sets(LateUpdate, (A, B, C).chain())`, and that configuring the same set twice accumulates rather than replaces.
5. *Joining a set* — `app.add_systems(LateUpdate, (a, b).in_set(A))`.
6. *Ordering against another crate's set* — the `app_widget.after(UiSet::Input).before(UiSet::Layout)` example from `crates/ui/tests/ui_sets.rs`, framed as the reason sets exist.
7. *The "constraint is ignored" warning* — what it means (the named system is not in that schedule), the two usual causes (the plugin was not registered; the target is in a different schedule), and that a set with no members is silent by design.
8. *Rules* — unconstrained conflicting systems still run in registration order; sets are flat; closures cannot be ordering targets.

- [ ] **Step 2: Check it against the code**

Copy each example into `crates/ecs/tests/ordering.rs` temporarily and run `cargo test -p ecs --test ordering`; revert the file afterwards. A doc example that does not compile is worse than no document.

- [ ] **Step 3: Commit**

```bash
git add docs/scheduling.md plans/ecs-todo.md
git commit -m "Document the system scheduling API"
```

---

## Notes for the implementer

- `System::system_type()` returns `TypeId::of::<FunctionSystem<F, Input>>()`. For a named `fn`, `F` is that function's unique zero-sized item type, so the id is stable across calls and crates. For a closure it is also unique, but there is no way to name it again, so closures cannot be ordering targets. This is inherent, not a limitation to work around.
- `Interned<dyn SystemSet>` is `Copy` and compares by pointer, so `HashMap<InternedSystemSet, _>` is cheap. The interner leaks one boxed label per distinct set value, once, for the process lifetime.
- The existing `SyncPoint` is `pub(crate)` in `crates/ecs/src/system/sync_point.rs` and has `write_world` access, so it conflicts with everything and acts as a barrier purely through implicit edges. Do not give it explicit ones.
- `crates/director`, `crates/physics` and `crates/app/tests` call `Schedule::compile` directly. They should need no change, but they are the first place to look if `compile`'s signature drifts.
