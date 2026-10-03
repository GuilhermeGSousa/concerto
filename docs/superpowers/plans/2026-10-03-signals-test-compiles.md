# Signals Test Compiles Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `cargo test -p concerto-ecs --test signals --no-run` succeeds, with listeners going through `IntoSystem<Args, Marker>`, and without leaving the rest of the workspace broken.

**Architecture:** `IntoSystem<Args, Marker>` is the single conversion API: it yields a concrete `System<In = Args>` through its `System` associated type. Three impls cover it: an existing `System` (marker `AlreadySystem`), a plain function (`Args = ()`, marker = the parameter tuple, yields `FunctionSystem<F, T>`), and a function with a leading argument (generic `Args`, marker `HasArgs<T>`, yields the new `ArgFunctionSystem<F, Args, T>`). A listener is just `impl IntoSystem<On<'static, S>, M>`, so `IntoListenerSystem` and `ListenerSystem` are deleted. Code that stores a `BoxedSystem` boxes the result of `into_system()` itself.

**Tech Stack:** Rust 1.96, edition 2024, `typle` 0.10 for tuple-arity impls.

**Spec:** none. The requirements are the line marked `// This needs to compile` in `crates/ecs/tests/signals.rs` and the `IntoSystem<Args, Marker>` trait as drafted in `crates/ecs/src/system/mod.rs`.

## Global Constraints

- Compile only. `World::add_listener` and `World::trigger` remain empty; no listener storage, no dispatch.
- The marked line in `crates/ecs/tests/signals.rs` stays verbatim: `world.add_listener(|_: On<TestSignal>, a: Query<&Transform>| {});`
- `IntoSystem` keeps the drafted shape: `Args` and `Marker` type parameters, a `System` associated type, `into_system`, `with_args`.
- No narrative comments in source. Only one-line `///` on `pub` items where the surrounding code already has them.
- The package is `concerto-ecs` (hyphen), not `ecs` or `concerto_ecs`.
- Line numbers refer to the working tree on 2026-10-03 (commit `1b24618` plus the uncommitted `IntoSystem` draft in `crates/ecs/src/system/mod.rs`). If they have drifted, match on the quoted code.
- Tasks 1 and 2 were compiled and run in an isolated copy of `crates/ecs`: all ecs test targets passed. Task 3 is unverified beyond its boxing pattern.

## Review Focus

Nothing here is exercised by a test, on purpose (compile-only scope). These are the things the next step will hit:

- `On<'static, S>` cannot be built soundly from a local `&mut S`, so `trigger` is not implementable as-is. `System::In` needs a lifetime-generic form (a GAT on `SystemArg`) before dispatch can exist.
- `with_args` needs `Args: Clone` to run more than once, and `On` holds `&mut S`, so a listener can never go through `SystemWithArgs`. Dispatch will have to call `run(On { .. }, world)` directly.
- `FunctionSystem` and `ArgFunctionSystem` duplicate `initialize` / `apply` / `fill_access`. A helper trait over the function (one impl per calling shape) would collapse them later.
- A function whose first parameter type implements both `SystemArg` and `SystemInput` is ambiguous for `IntoSystem`. Today only `()` is both.
- `into_system()` no longer returns `BoxedSystem`, so mixing two systems in one array or `Vec` needs an explicit `Box::new(..) as BoxedSystem`.

---

### Task 1: Finish `IntoSystem<Args, Marker>` and route listeners through it

**Files:**
- Modify: `crates/ecs/src/system/mod.rs:94-97` (struct), `:141` (`run_unsafe`), `:156` (insert after), `:158-164` (trait), `:175-196` (`SystemWithArgs` impl), `:206-224` (impls)
- Modify: `crates/ecs/src/system/config.rs:5-8`, `:30`, `:181-184`
- Modify: `crates/ecs/src/signal/mod.rs` (whole file)
- Modify: `crates/ecs/src/world.rs:16`, `:649-651`
- Modify: `crates/ecs/src/system/sync_point.rs:21`
- Test: `crates/ecs/tests/signals.rs:1`

**Interfaces:**
- Consumes: `System` (with `type In: SystemArg`), `SystemArg`, `SystemInput`, `SystemWithArgs<S, Args>` as they exist in the working tree.
- Produces:
  - `pub trait IntoSystem<Args: SystemArg, Marker>: Sized { type System: System<In = Args>; fn into_system(self) -> Self::System; fn with_args(self, args: Args) -> SystemWithArgs<Self::System, Args>; }`
  - `pub struct FunctionSystem<F, Input: SystemInput>` (`In = ()`)
  - `pub struct ArgFunctionSystem<F, Args: SystemArg, Input: SystemInput>` (`In = Args`) with `pub fn new(func: F) -> Self`
  - `pub struct HasArgs<T>` (marker)
  - `World::add_listener<T: Signal, M>(&mut self, _system: impl IntoSystem<On<'static, T>, M>)`

- [ ] **Step 1: Confirm the starting failure**

Run: `cargo test -p concerto-ecs --test signals --no-run`
Expected: FAIL in the lib with `E0107: trait takes 2 generic arguments but 1 generic argument was supplied` in `crates/ecs/src/system/mod.rs` and `crates/ecs/src/system/config.rs`, because `IntoSystem` now takes two type parameters and its impls still supply one. The `E0119: conflicting implementations of trait IntoSystemConfig` errors are fallout from that and disappear with it.

- [ ] **Step 2: Make `FunctionSystem` public with a private `func`**

`into_system` now returns it by name, and a `pub(crate)` type in a public associated type is rejected (E0446). Replace `crates/ecs/src/system/mod.rs:94-97`:

```rust
pub struct FunctionSystem<F, Input: SystemInput> {
    func: F,
    system_state: Option<Input::State>,
}
```

On line 141, rename the unused argument in `FunctionSystem`'s `run_unsafe`:

```rust
    unsafe fn run_unsafe(&mut self, _args: Self::In, world: UnsafeWorldCell) {
```

- [ ] **Step 3: Add `ArgFunctionSystem` after the `FunctionSystem` impl**

Insert after line 156 (the closing `}` of `impl ... System for FunctionSystem<F, Inputs>`), before the `/// Converts a function ...` doc line:

```rust

pub struct ArgFunctionSystem<F, Args: SystemArg, Input: SystemInput> {
    func: F,
    system_state: Option<Input::State>,
    _marker: PhantomData<Args>,
}

impl<F, Args, Input> ArgFunctionSystem<F, Args, Input>
where
    Args: SystemArg,
    Input: SystemInput + 'static,
{
    pub fn new(func: F) -> Self {
        Self {
            func,
            system_state: None,
            _marker: PhantomData,
        }
    }
}

#[allow(unused_variables, unused_mut, clippy::unit_arg)]
#[typle(Tuple for 0..=12)]
impl<F, Args, Inputs> System for ArgFunctionSystem<F, Args, Inputs>
where
    F: Send + Sync + 'static,
    Args: SystemArg + 'static,
    Inputs: Tuple,
    Inputs<_>: SystemInput + 'static,
    for<'w, 's> F: FnMut(Args, typle_args!(i in .. => Inputs<{i}>))
        + FnMut(Args, typle_args!(i in .. => Inputs<{i}>::Data<'w, 's>)),
{
    type In = Args;

    fn name(&self) -> &'static str {
        std::any::type_name::<F>()
    }

    fn initialize(&mut self, world: &mut World) {
        self.system_state = Some(Inputs::init_state(world));
    }

    fn apply(&mut self, world: &mut World) {
        for typle_index!(i) in 0..Inputs::LEN {
            let state = self
                .system_state
                .as_mut()
                .expect("Attempted to run uninitialized system.");
            <Inputs<{ i }>>::apply(&mut state[[i]], world);
        }
    }

    unsafe fn run_unsafe(&mut self, args: Self::In, world: UnsafeWorldCell) {
        let state = self
            .system_state
            .as_mut()
            .expect("Attempted to run uninitialized system.");
        (self.func)(args, typle_args!(i in .. =>  {
            <Inputs<{i}>>::get_data(&mut state[[i]], world)
        }));
    }

    fn fill_access(&self, meta: &mut SystemMetadata, access: &mut SystemAccess) {
        for typle_index!(i) in 0..Inputs::LEN {
            <Inputs<{ i }>>::fill_access(meta, access);
        }
    }
}
```

`PhantomData` is already imported on line 12.

- [ ] **Step 4: Complete the `IntoSystem` trait**

`with_args` has to receive the value it binds, and it can be a provided method once the trait is `Sized`. Replace the doc line and trait (lines 158-164):

```rust
/// Converts a function, closure or [`System`] into a [`System`] taking `Args` as its input.
pub trait IntoSystem<Args: SystemArg, Marker>: Sized {
    type System: System<In = Args>;

    fn into_system(self) -> Self::System;

    /// Binds `args` to the system, so it runs without an input.
    fn with_args(self, args: Args) -> SystemWithArgs<Self::System, Args> {
        SystemWithArgs {
            system: self.into_system(),
            args,
        }
    }
}
```

- [ ] **Step 5: Make `SystemWithArgs` runnable more than once**

`run_unsafe` takes `&mut self`, so `self.args` cannot be moved out (E0507). In `impl<S, Args> System for SystemWithArgs<S, Args>`, replace the `where` clause:

```rust
where
    S: System<In = Args>,
    Args: SystemArg + Clone + 'static,
```

and the body of its `run_unsafe`:

```rust
    unsafe fn run_unsafe(&mut self, _args: Self::In, world: UnsafeWorldCell) {
        unsafe { self.system.run_unsafe(self.args.clone(), world) };
    }
```

The struct definition keeps its bounds as they are.

- [ ] **Step 6: Replace the two `IntoSystem` impls and add the leading-argument one**

Replace from `impl<S: System<In = ()> + 'static> IntoSystem<AlreadySystem> for S {` (line 206) through the closing `}` of the `#[typle]` impl (line 224). The `#[doc(hidden)] pub struct AlreadySystem;` above it stays.

```rust
impl<S: System> IntoSystem<S::In, AlreadySystem> for S {
    type System = S;

    fn into_system(self) -> Self::System {
        self
    }
}

#[typle(Tuple for 0..=12)]
impl<F, T> IntoSystem<(), T> for F
where
    F: Send + Sync + 'static,
    T: Tuple,
    T<_>: SystemInput + 'static,
    for<'w, 's> F:
        FnMut(typle_args!(i in .. => T<{i}>)) + FnMut(typle_args!(i in .. => T<{i}>::Data<'w, 's>)),
{
    type System = FunctionSystem<F, T>;

    fn into_system(self) -> Self::System {
        FunctionSystem::new(self)
    }
}

#[doc(hidden)]
pub struct HasArgs<T>(PhantomData<T>);

#[typle(Tuple for 0..=12)]
impl<F, Args, T> IntoSystem<Args, HasArgs<T>> for F
where
    F: Send + Sync + 'static,
    Args: SystemArg + 'static,
    T: Tuple,
    T<_>: SystemInput + 'static,
    for<'w, 's> F: FnMut(Args, typle_args!(i in .. => T<{i}>))
        + FnMut(Args, typle_args!(i in .. => T<{i}>::Data<'w, 's>)),
{
    type System = ArgFunctionSystem<F, Args, T>;

    fn into_system(self) -> Self::System {
        ArgFunctionSystem::new(self)
    }
}
```

`HasArgs<T>` is what keeps the last impl from overlapping the plain-function one: the two differ in the `Marker` parameter, so the compiler never has to compare their `FnMut` bounds.

- [ ] **Step 7: Update `crates/ecs/src/system/config.rs`**

Schedules only take systems without an input, and `SystemEntry::system` is a `BoxedSystem`. Replace the import block (lines 5-8):

```rust
use crate::system::{
    BoxedSystem, IntoSystem, System,
    set::{InternedSystemSet, SystemSet},
};
```

Line 30:

```rust
impl<M, S: IntoSystem<(), M> + 'static> IntoDependencyTarget<SystemTarget<M>> for S {
```

Lines 181-184:

```rust
impl<M, F: IntoSystem<(), M> + 'static> IntoSystemConfig<M> for F {
    fn into_config(self) -> SystemConfig {
        SystemConfig(SystemNode::Single(SystemEntry {
            system: Box::new(self.into_system()),
```

- [ ] **Step 8: Reduce `crates/ecs/src/signal/mod.rs` to the signal types**

Replace the whole file. `ListenerSystem` and `IntoListenerSystem` are gone; `IntoSystem<On<'static, S>, M>` replaces both.

```rust
use crate::system::input::SystemArg;

pub trait Signal: Send + Sync + 'static {}

pub struct On<'w, S: Signal> {
    signal: &'w mut S,
}

impl<S: Signal> SystemArg for On<'_, S> {}
```

- [ ] **Step 9: Point `World::add_listener` at `IntoSystem`**

In `crates/ecs/src/world.rs`, replace line 16:

```rust
use crate::signal::{On, Signal};
use crate::system::IntoSystem;
```

Replace line 649:

```rust
    pub fn add_listener<T: Signal, M>(&mut self, _system: impl IntoSystem<On<'static, T>, M>) {}
```

Replace the `trigger` signature on line 651 (body stays empty):

```rust
    pub fn trigger<T: Signal>(&mut self, _signal: T)
```

- [ ] **Step 10: Silence the unused argument in `crates/ecs/src/system/sync_point.rs:21`**

```rust
    unsafe fn run_unsafe(&mut self, _args: Self::In, _world: crate::world::UnsafeWorldCell) {}
```

- [ ] **Step 11: Give the test a `Transform` component**

`Transform` lives in `concerto-foundation`, which depends on `concerto-ecs`, so the test declares its own. Replace line 1 of `crates/ecs/tests/signals.rs`:

```rust
use concerto_ecs::{Component, Query, World, signal::{On, Signal}};

#[derive(Component)]
struct Transform;
```

- [ ] **Step 12: Verify the lib and the signals test compile**

Run: `cargo test -p concerto-ecs --test signals --no-run`
Expected: `Finished` and `Executable tests/signals.rs (...)`. Two warnings are expected: `field signal is never read` (lib) and `unused variable: a` (test).

Run: `cargo build --workspace`
Expected: `Finished`. Outside `crates/ecs`, non-test code only uses `IntoSystemConfig`, which is unchanged.

- [ ] **Step 13: Commit**

```bash
git add crates/ecs/src/system/mod.rs crates/ecs/src/system/config.rs crates/ecs/src/signal/mod.rs crates/ecs/src/world.rs crates/ecs/src/system/sync_point.rs crates/ecs/tests/signals.rs
git commit -m "Convert functions with a leading argument through IntoSystem"
```

---

### Task 2: Update the remaining `concerto-ecs` test code to the new `System` signatures

`run`, `run_and_apply` and `run_unsafe` take `args: Self::In` first. Six places inside the ecs crate's own tests still use the old shape.

**Files:**
- Modify: `crates/ecs/src/system/executor/multi_thread.rs:326`
- Modify: `crates/ecs/src/system/input/component_reader.rs:111`
- Modify: `crates/ecs/tests/component_change_history.rs:43`
- Modify: `crates/ecs/tests/entity_recycling.rs:35`
- Modify: `crates/ecs/tests/entity_structure.rs:59`, `:69`

**Interfaces:**
- Consumes: `System::run(&mut self, args: Self::In, world: &mut World)`, `System::run_and_apply(&mut self, args: Self::In, world: &mut World)`, `unsafe fn run_unsafe(&mut self, args: Self::In, world: UnsafeWorldCell)`.
- Produces: nothing new.

- [ ] **Step 1: Confirm the failure**

Run: `cargo test -p concerto-ecs --no-run`
Expected: FAIL with `E0061: this method takes 2 arguments but 1 argument was supplied` at the five call sites and `E0050: method run_unsafe has 2 parameters but the declaration in trait ... has 3` at `multi_thread.rs:326`.

- [ ] **Step 2: Fix `NonSendProbe::run_unsafe` in `crates/ecs/src/system/executor/multi_thread.rs:326`**

```rust
        unsafe fn run_unsafe(&mut self, _args: Self::In, _world: UnsafeWorldCell) {
```

- [ ] **Step 3: Pass `()` at the five call sites**

`crates/ecs/src/system/input/component_reader.rs:111`
```rust
        read.run_and_apply((), &mut world);
```

`crates/ecs/tests/component_change_history.rs:43`
```rust
    system.run_and_apply((), &mut world);
```

`crates/ecs/tests/entity_recycling.rs:35`
```rust
    probe.run_and_apply((), &mut world);
```

`crates/ecs/tests/entity_structure.rs:59`
```rust
    add.run((), &mut world);
```

`crates/ecs/tests/entity_structure.rs:69`
```rust
    remove.run_and_apply((), &mut world);
```

- [ ] **Step 4: Verify the whole crate**

Run: `cargo test -p concerto-ecs`
Expected: every target `ok`, 0 failed (69 lib tests, `signals` 1 passed).

- [ ] **Step 5: Commit**

```bash
git add crates/ecs/src/system/executor/multi_thread.rs crates/ecs/src/system/input/component_reader.rs crates/ecs/tests/component_change_history.rs crates/ecs/tests/entity_recycling.rs crates/ecs/tests/entity_structure.rs
git commit -m "Pass the unit input to systems run by hand in ecs tests"
```

---

### Task 3: Update test and example code in the other crates

Not needed for the stated goal, but without it `cargo test --workspace` stops compiling. Two kinds of change, all inside `#[cfg(test)]` modules, integration tests, or one example:

- 37 `.run_and_apply(world)` calls need the `()` input.
- 4 arrays that mix several `into_system()` results need boxing, because each function now converts to its own concrete type.

**Files:**
- Modify: `crates/animation/src/target.rs:109`
- Modify: `crates/foundation/src/transform/systems.rs:92`
- Modify: `crates/scene/tests/spawn_scene.rs:97`, `:148`, `:281`
- Modify: `crates/ui/tests/tabs.rs:20`
- Modify: `crates/ui/tests/anchored_panel.rs:335`
- Modify: `crates/editor/examples/custom_asset.rs:195`, `:198`, `:201`, `:228`
- Modify: `crates/editor/tests/custom_property.rs:295`, `:319`, `:342`
- Modify: `crates/editor/src/dock.rs:338`
- Modify: `crates/editor/src/scene.rs:207`, `:212`, `:219`
- Modify: `crates/editor/src/asset_editor.rs:251`, `:341`
- Modify: `crates/editor/src/workspace.rs:156`, `:161`, `:164-167`, `:169`, `:204-208`, `:290`, `:343`, `:357`
- Modify: `crates/editor/src/inspector/numeric.rs:531`, `:534`, `:554`, `:572`, `:592`, `:611`
- Modify: `crates/editor/src/inspector/registry.rs:557`
- Modify: `crates/editor/src/inspector/tests.rs:2-6`, `:12-16`, `:18`, `:25-29`, `:42`, `:49`, `:52`, `:593`

**Interfaces:**
- Consumes: `System::run_and_apply(&mut self, args: Self::In, world: &mut World)`; `IntoSystem::into_system(self) -> Self::System`; `concerto_ecs::system::BoxedSystem` (`Box<dyn System<In = ()>>`).
- Produces: nothing new.

- [ ] **Step 1: Rewrite every `run_and_apply` call**

Each site changes from `x.run_and_apply(w)` to `x.run_and_apply((), w)`. Run from the repo root:

```bash
sed -i 's/\.run_and_apply(/.run_and_apply((), /' \
  crates/animation/src/target.rs \
  crates/foundation/src/transform/systems.rs \
  crates/scene/tests/spawn_scene.rs \
  crates/ui/tests/tabs.rs \
  crates/ui/tests/anchored_panel.rs \
  crates/editor/examples/custom_asset.rs \
  crates/editor/tests/custom_property.rs \
  crates/editor/src/dock.rs \
  crates/editor/src/scene.rs \
  crates/editor/src/asset_editor.rs \
  crates/editor/src/workspace.rs \
  crates/editor/src/inspector/numeric.rs \
  crates/editor/src/inspector/registry.rs \
  crates/editor/src/inspector/tests.rs
```

Run: `git diff --stat -- crates/animation crates/foundation crates/scene crates/ui crates/editor`
Expected: 14 files changed, 37 insertions(+), 37 deletions(-).

- [ ] **Step 2: Box the system arrays in `crates/editor/src/workspace.rs`**

Line 156, inside `mod tests`:

```rust
    use concerto_ecs::{IntoSystem, System, World, system::BoxedSystem};
```

Lines 164-167:

```rust
        for mut system in [
            Box::new(super::sync_workspace.into_system()) as BoxedSystem,
            Box::new(super::reset_workspace_input.into_system()),
        ] {
```

Lines 204-208:

```rust
        for system in [
            Box::new(super::create_editor_hosts.into_system()) as BoxedSystem,
            Box::new(super::sync_workspace.into_system()),
            Box::new(super::reset_workspace_input.into_system()),
        ] {
```

- [ ] **Step 3: Box the system arrays in `crates/editor/src/inspector/tests.rs`**

Lines 2-6:

```rust
use concerto_ecs::{
    IntoSystem, System, World,
    component::scene::{SceneComponent, SceneSpawnContext},
    entity::hierarchy::{ChildOf, Children},
    system::BoxedSystem,
};
```

Lines 12-16:

```rust
    for mut system in [
        Box::new(collect_inspector_data.into_system()) as BoxedSystem,
        Box::new(sync_inspected_components.into_system()),
        Box::new(build_property_widgets.into_system()),
    ] {
```

Lines 25-29:

```rust
    for system in [
        Box::new(collect_inspector_data.into_system()) as BoxedSystem,
        Box::new(sync_inspected_components.into_system()),
        Box::new(build_property_widgets.into_system()),
    ] {
```

- [ ] **Step 4: Verify every target in the workspace**

Run: `cargo check --workspace --all-targets`
Expected: `Finished`. Two leftovers are possible, both mechanical:
- `E0061: this method takes 2 arguments but 1 argument was supplied`: a `.run(&mut world)` on a system rather than a `Schedule`; change it to `.run((), &mut world)`.
- `E0308: mismatched types` inside an array or `Vec` of systems: box the elements as in Steps 2 and 3.

- [ ] **Step 5: Commit**

```bash
git add crates/animation crates/foundation crates/scene crates/ui crates/editor
git commit -m "Follow the IntoSystem and System input changes in tests"
```
