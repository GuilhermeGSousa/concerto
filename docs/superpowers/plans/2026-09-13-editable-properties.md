# Editable Component Properties Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let the editor's inspector edit `Transform`'s translation, rotation and scale on the live world, through an `Editable` trait and derive that later components and leaf types can plug into.

**Architecture:** A new `editable` crate defines one object-safe trait (`Editable`), an owned presentation value (`EditorValue`), and two visitor-based tree walks (`collect`, `apply`). A derive implements the walks for structs. The editor keeps a `TypeId`-keyed registry of monomorphized per-component `collect`/`apply` functions and per-leaf widgets; inspector rows carry their target, widgets push commits to a queue, and an exclusive system applies them.

**Tech Stack:** Rust (toolchain 1.96), the in-repo `ecs`/`app`/`ui` crates, `glam` 0.30, `syn` 2 / `quote` 1 for the derive.

**Spec:** `docs/superpowers/specs/2026-09-13-editable-properties-design.md`

## Global Constraints

- Scope is `Transform` only. Do not make any other component `Editable`.
- Edits mutate the live `World` only; no persistence, no undo.
- Registration is keyed by `TypeId`; no type-name strings as keys.
- `EditorValue` has exactly two variants: `Number(f64)` and `Vec3([f64; 3])`.
- Never use `.after()` / `.before()` when adding systems: the scheduler re-registers their arguments as duplicate systems. Order comes from registration order.
- `crates/editor` and `crates/essential` are edition 2021: no `if let ... && let` chains there. `crates/ecs`, `crates/ui` and the new `crates/editable` are edition 2024.
- Comments: short `///` docs on public API and comments for non-obvious constraints only. No file banners, no narration.
- Version control: **do not commit.** The user commits and rewrites history themselves. Each task ends by staging its files with `git add`.
- Every task must leave `cargo build -p editor` and the task's tests passing.

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/ecs/src/world.rs` (modify) | `World::component_ids`, `World::type_info` |
| `crates/ecs/tests/component_reflection.rs` (modify) | tests for the above |
| `crates/editable/Cargo.toml` (create) | new crate manifest |
| `crates/editable/src/lib.rs` (create) | `Editable`, `EditorValue`, visitor traits, re-exports, derive doctests |
| `crates/editable/src/leaves.rs` (create) | `Editable` for `f32`, `f64`, `Vec3`, `Quat` |
| `crates/editable/src/property.rs` (create) | `PropertyPath`, `Property`, `ApplyError`, `collect`, `apply` |
| `crates/editable/macros/Cargo.toml` (create) | proc-macro manifest |
| `crates/editable/macros/src/lib.rs` (create) | `#[derive(Editable)]` |
| `crates/editable/tests/derive.rs` (create) | derive behavior tests |
| `crates/essential/Cargo.toml`, `crates/essential/src/transform/mod.rs` (modify) | `Transform: Editable` |
| `crates/ui/src/text_input.rs`, `crates/ui/src/plugin.rs` (modify) | submit/cancel events |
| `crates/editor/src/inspector.rs` → `crates/editor/src/inspector/mod.rs` (move + modify) | panel, collect, rebuild, refresh |
| `crates/editor/src/inspector/rows.rs` (create) | `PropertyRow`, `PropertyRowValue`, `PropertyCommit`, `PropertyCommits`, `PropertyWidget` |
| `crates/editor/src/inspector/registry.rs` (create) | `InspectorRegistry`, `EditableApp`, `apply_property_commits` |
| `crates/editor/src/inspector/numeric.rs` (create) | `NumericFields` widget and its systems |

---

### Task 1: `World::component_ids` and `World::type_info`

**Files:**
- Modify: `crates/ecs/src/world.rs` (the `component_types` method, ~line 599)
- Test: `crates/ecs/tests/component_reflection.rs`

**Interfaces:**
- Produces: `World::component_ids(&self, entity: Entity) -> &[ComponentId]`; `World::type_info(&self, id: ComponentId) -> Option<&TypeInfo>`. `ComponentId` is `std::any::TypeId`.

- [ ] **Step 1: Write the failing tests**

Append to `crates/ecs/tests/component_reflection.rs` (the file already defines `Health`, `Armour` and `Plumbing`):

```rust
#[test]
fn component_ids_include_components_that_are_not_scene_components() {
    let mut world = World::default();
    world.register_component_type::<Health>();

    let entity = world.spawn((Health { current: 3, max: 10 }, Plumbing));

    let mut ids = world.component_ids(entity).to_vec();
    ids.sort();
    let mut expected = vec![
        std::any::TypeId::of::<Health>(),
        std::any::TypeId::of::<Plumbing>(),
    ];
    expected.sort();
    assert_eq!(ids, expected);
}

#[test]
fn a_stale_entity_has_no_component_ids() {
    let mut world = World::default();
    let entity = world.spawn((Plumbing,));
    world.despawn(entity);

    assert!(world.component_ids(entity).is_empty());
}

#[test]
fn type_info_is_only_known_for_scene_components() {
    let mut world = World::default();
    world.register_component_type::<Health>();
    world.spawn((Health { current: 1, max: 1 }, Plumbing));

    assert_eq!(
        world
            .type_info(std::any::TypeId::of::<Health>())
            .map(|info| info.short()),
        Some("Health")
    );
    assert!(world.type_info(std::any::TypeId::of::<Plumbing>()).is_none());
}
```

- [ ] **Step 2: Run the tests and confirm they fail**

Run: `cargo test -p ecs --test component_reflection`
Expected: compile error, `no method named component_ids` / `type_info` on `World`.

- [ ] **Step 3: Implement**

In `crates/ecs/src/world.rs`, replace the body of `component_types` and add the two methods directly above it:

```rust
    /// Every component `entity` carries, including ones that are not scene
    /// components. Empty for a stale entity.
    pub fn component_ids(&self, entity: Entity) -> &[ComponentId] {
        self.entity_store
            .find_location(entity)
            .map(|location| self.archetypes[location.archetype_index as usize].component_ids())
            .unwrap_or(&[])
    }

    /// The read side of a registered scene component, or `None` if `id` is not one.
    pub fn type_info(&self, id: ComponentId) -> Option<&TypeInfo> {
        self.component_registry.type_info(&id)
    }
```

and change `component_types`' body (keep its doc comment) to:

```rust
    pub fn component_types(&self, entity: Entity) -> impl Iterator<Item = &TypeInfo> {
        self.component_ids(entity)
            .iter()
            .filter_map(|component_id| self.component_registry.type_info(component_id))
    }
```

`ComponentId` and `TypeInfo` are already imported in `world.rs` (used by `component_types`); if the compiler says otherwise, add `use crate::component::{ComponentId, registry::TypeInfo};`.

- [ ] **Step 4: Run the tests and confirm they pass**

Run: `cargo test -p ecs --test component_reflection`
Expected: all tests PASS, including the pre-existing ones.

- [ ] **Step 5: Stage**

```bash
git add crates/ecs/src/world.rs crates/ecs/tests/component_reflection.rs
```

---

### Task 2: The `editable` crate core

**Files:**
- Create: `crates/editable/Cargo.toml`, `crates/editable/src/lib.rs`, `crates/editable/src/leaves.rs`, `crates/editable/src/property.rs`
- Create (stub so the crate compiles; filled in by Task 3): `crates/editable/macros/Cargo.toml`, `crates/editable/macros/src/lib.rs`

**Interfaces:**
- Produces:
  - `pub enum EditorValue { Number(f64), Vec3([f64; 3]) }` — `Clone, Debug, PartialEq`
  - `pub trait Editable: Any { fn read(&self) -> Option<EditorValue>; fn write(&mut self, value: &EditorValue) -> bool; fn visit(&self, visitor: &mut dyn PropertyVisitor); fn visit_mut(&mut self, visitor: &mut dyn PropertyVisitorMut); }` (all defaulted)
  - `pub trait PropertyVisitor { fn field(&mut self, name: &'static str, value: &dyn Editable); }`
  - `pub trait PropertyVisitorMut { fn field(&mut self, name: &'static str, value: &mut dyn Editable); }`
  - `pub struct PropertyPath` — `Clone, Debug, Default, PartialEq, Eq, Hash`; `PropertyPath::new(impl IntoIterator<Item = &'static str>)`, `.segments() -> &[&'static str]`, `.name() -> &'static str`
  - `pub struct Property { pub path: PropertyPath, pub type_id: TypeId, pub value: EditorValue }` — `Clone, Debug, PartialEq`
  - `pub enum ApplyError { NotFound, Rejected }` — `Debug, PartialEq, Eq`
  - `pub fn collect(root: &dyn Editable) -> Vec<Property>`
  - `pub fn apply(root: &mut dyn Editable, path: &PropertyPath, value: &EditorValue) -> Result<(), ApplyError>`

- [ ] **Step 1: Create the manifests and the macro stub**

`crates/editable/Cargo.toml`:

```toml
[package]
name = "editable"
version = "0.1.0"
edition = "2024"

[dependencies]
editable-macros = { path = "macros" }
glam = "0.30.1"
```

`crates/editable/macros/Cargo.toml`:

```toml
[package]
name = "editable-macros"
version = "0.1.0"
edition = "2024"

[lib]
proc-macro = true

[dependencies]
syn = "2.0.11"
quote = "1.0.37"
```

`crates/editable/macros/src/lib.rs` (stub; Task 3 replaces it):

```rust
use proc_macro::TokenStream;

#[proc_macro_derive(Editable)]
pub fn derive_editable(_input: TokenStream) -> TokenStream {
    TokenStream::new()
}
```

`crates/editable` is picked up by the root workspace's `crates/*` glob; `crates/editable/macros` becomes a member as a path dependency, as `crates/ecs/macros` does.

- [ ] **Step 2: Write `lib.rs`**

`crates/editable/src/lib.rs`:

```rust
extern crate self as editable;

mod leaves;
mod property;

use std::any::Any;

pub use editable_macros::Editable;
pub use property::{ApplyError, Property, PropertyPath, apply, collect};

/// The value of one leaf in the form the editor shows and edits, which is not
/// necessarily its Rust form: a `Quat` is presented as euler degrees.
#[derive(Clone, Debug, PartialEq)]
pub enum EditorValue {
    Number(f64),
    Vec3([f64; 3]),
}

/// A type the editor can show and change. A leaf answers `read`/`write`; a
/// struct answers `visit`/`visit_mut`, usually through `#[derive(Editable)]`.
pub trait Editable: Any {
    /// `Some` for a leaf, `None` for a struct.
    fn read(&self) -> Option<EditorValue> {
        None
    }

    /// Returns `false` when `value` has the wrong shape or is not finite; the
    /// leaf is then left unchanged.
    fn write(&mut self, _value: &EditorValue) -> bool {
        false
    }

    fn visit(&self, _visitor: &mut dyn PropertyVisitor) {}

    fn visit_mut(&mut self, _visitor: &mut dyn PropertyVisitorMut) {}
}

pub trait PropertyVisitor {
    fn field(&mut self, name: &'static str, value: &dyn Editable);
}

pub trait PropertyVisitorMut {
    fn field(&mut self, name: &'static str, value: &mut dyn Editable);
}
```

- [ ] **Step 3: Write the failing leaf tests**

`crates/editable/src/leaves.rs` (tests first; the impls go above them in Step 5):

```rust
#[cfg(test)]
mod tests {
    use glam::{EulerRot, Quat, Vec3};

    use crate::{Editable, EditorValue};

    #[test]
    fn scalars_round_trip() {
        let mut value = 0.0_f32;
        assert!(value.write(&EditorValue::Number(2.5)));
        assert_eq!(value.read(), Some(EditorValue::Number(2.5)));

        let mut value = 0.0_f64;
        assert!(value.write(&EditorValue::Number(-7.25)));
        assert_eq!(value.read(), Some(EditorValue::Number(-7.25)));
    }

    #[test]
    fn vec3_round_trips() {
        let mut value = Vec3::ZERO;
        assert!(value.write(&EditorValue::Vec3([1.0, 2.0, 3.0])));
        assert_eq!(value, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(value.read(), Some(EditorValue::Vec3([1.0, 2.0, 3.0])));
    }

    #[test]
    fn quat_is_presented_as_euler_degrees() {
        let quarter_turn = Quat::from_rotation_y(90_f32.to_radians());
        let Some(EditorValue::Vec3([x, y, z])) = quarter_turn.read() else {
            panic!("a Quat must read as a Vec3 of degrees");
        };
        assert!(x.abs() < 1e-3 && (y - 90.0).abs() < 1e-3 && z.abs() < 1e-3);
    }

    #[test]
    fn quat_round_trips_through_degrees() {
        let original = Quat::from_euler(EulerRot::XYZ, 0.3, -1.1, 2.0);
        let mut copy = Quat::IDENTITY;
        assert!(copy.write(&original.read().unwrap()));
        assert!(
            original.dot(copy).abs() > 1.0 - 1e-5,
            "q and -q are the same rotation; compare with |dot|"
        );
    }

    #[test]
    fn non_finite_values_are_rejected_and_leave_the_leaf_unchanged() {
        let mut scalar = 1.0_f32;
        assert!(!scalar.write(&EditorValue::Number(f64::NAN)));
        assert!(!scalar.write(&EditorValue::Number(1e300)), "overflows f32");
        assert_eq!(scalar, 1.0);

        let mut vector = Vec3::ONE;
        assert!(!vector.write(&EditorValue::Vec3([0.0, f64::INFINITY, 0.0])));
        assert_eq!(vector, Vec3::ONE);

        let mut rotation = Quat::IDENTITY;
        assert!(!rotation.write(&EditorValue::Vec3([f64::NAN, 0.0, 0.0])));
        assert_eq!(rotation, Quat::IDENTITY);
    }

    #[test]
    fn wrong_shapes_are_rejected() {
        let mut scalar = 1.0_f32;
        assert!(!scalar.write(&EditorValue::Vec3([0.0; 3])));
        let mut vector = Vec3::ONE;
        assert!(!vector.write(&EditorValue::Number(0.0)));
        assert_eq!(vector, Vec3::ONE);
    }
}
```

- [ ] **Step 4: Write the failing `collect`/`apply` tests**

Create `crates/editable/src/property.rs` containing only the test module given in Step 7's "Tests" block below. (Both modules are needed before the crate can compile, so both test suites are written before either implementation.)

- [ ] **Step 5: Run and confirm failure**

Run: `cargo test -p editable`
Expected: compile errors — `unresolved imports property::{ApplyError, Property, PropertyPath, apply, collect}` and `the trait Editable is not implemented for f32`.

- [ ] **Step 6: Implement the leaves**

Put above the test module in `crates/editable/src/leaves.rs`:

```rust
use glam::{EulerRot, Quat, Vec3};

use crate::{Editable, EditorValue};

fn to_vec3(value: [f64; 3]) -> Vec3 {
    Vec3::new(value[0] as f32, value[1] as f32, value[2] as f32)
}

fn from_vec3(value: Vec3) -> EditorValue {
    EditorValue::Vec3([value.x as f64, value.y as f64, value.z as f64])
}

impl Editable for f32 {
    fn read(&self) -> Option<EditorValue> {
        Some(EditorValue::Number(*self as f64))
    }

    fn write(&mut self, value: &EditorValue) -> bool {
        let EditorValue::Number(number) = value else {
            return false;
        };
        // Checked after narrowing: a finite f64 can overflow f32.
        let number = *number as f32;
        if !number.is_finite() {
            return false;
        }
        *self = number;
        true
    }
}

impl Editable for f64 {
    fn read(&self) -> Option<EditorValue> {
        Some(EditorValue::Number(*self))
    }

    fn write(&mut self, value: &EditorValue) -> bool {
        let EditorValue::Number(number) = value else {
            return false;
        };
        if !number.is_finite() {
            return false;
        }
        *self = *number;
        true
    }
}

impl Editable for Vec3 {
    fn read(&self) -> Option<EditorValue> {
        Some(from_vec3(*self))
    }

    fn write(&mut self, value: &EditorValue) -> bool {
        let EditorValue::Vec3(components) = value else {
            return false;
        };
        let vector = to_vec3(*components);
        if !vector.is_finite() {
            return false;
        }
        *self = vector;
        true
    }
}

impl Editable for Quat {
    fn read(&self) -> Option<EditorValue> {
        let (x, y, z) = self.to_euler(EulerRot::XYZ);
        Some(from_vec3(Vec3::new(x, y, z).map(f32::to_degrees)))
    }

    fn write(&mut self, value: &EditorValue) -> bool {
        let EditorValue::Vec3(degrees) = value else {
            return false;
        };
        let radians = to_vec3(*degrees).map(f32::to_radians);
        if !radians.is_finite() {
            return false;
        }
        *self = Quat::from_euler(EulerRot::XYZ, radians.x, radians.y, radians.z).normalize();
        true
    }
}
```

- [ ] **Step 7: Implement `property.rs`**

**Tests** (this is the module Step 4 created the file with; hand-written impls, since the derive arrives in Task 3):

```rust
#[cfg(test)]
mod tests {
    use std::any::TypeId;

    use glam::Vec3;

    use super::*;
    use crate::{PropertyVisitor, PropertyVisitorMut};

    struct Inner {
        weight: f32,
    }

    impl Editable for Inner {
        fn visit(&self, visitor: &mut dyn PropertyVisitor) {
            visitor.field("weight", &self.weight);
        }
        fn visit_mut(&mut self, visitor: &mut dyn PropertyVisitorMut) {
            visitor.field("weight", &mut self.weight);
        }
    }

    struct Outer {
        position: Vec3,
        inner: Inner,
    }

    impl Editable for Outer {
        fn visit(&self, visitor: &mut dyn PropertyVisitor) {
            visitor.field("position", &self.position);
            visitor.field("inner", &self.inner);
        }
        fn visit_mut(&mut self, visitor: &mut dyn PropertyVisitorMut) {
            visitor.field("position", &mut self.position);
            visitor.field("inner", &mut self.inner);
        }
    }

    fn outer() -> Outer {
        Outer {
            position: Vec3::new(1.0, 2.0, 3.0),
            inner: Inner { weight: 0.5 },
        }
    }

    #[test]
    fn collect_flattens_nested_leaves_in_field_order() {
        assert_eq!(
            collect(&outer()),
            vec![
                Property {
                    path: PropertyPath::new(["position"]),
                    type_id: TypeId::of::<Vec3>(),
                    value: EditorValue::Vec3([1.0, 2.0, 3.0]),
                },
                Property {
                    path: PropertyPath::new(["inner", "weight"]),
                    type_id: TypeId::of::<f32>(),
                    value: EditorValue::Number(0.5),
                },
            ]
        );
    }

    #[test]
    fn apply_writes_exactly_one_nested_leaf() {
        let mut value = outer();
        let result = apply(
            &mut value,
            &PropertyPath::new(["inner", "weight"]),
            &EditorValue::Number(2.0),
        );
        assert_eq!(result, Ok(()));
        assert_eq!(value.inner.weight, 2.0);
        assert_eq!(value.position, Vec3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn a_path_ending_on_a_struct_is_not_found() {
        let result = apply(
            &mut outer(),
            &PropertyPath::new(["inner"]),
            &EditorValue::Number(2.0),
        );
        assert_eq!(result, Err(ApplyError::NotFound));
    }

    #[test]
    fn a_path_continuing_past_a_leaf_is_not_found() {
        let result = apply(
            &mut outer(),
            &PropertyPath::new(["position", "x"]),
            &EditorValue::Number(2.0),
        );
        assert_eq!(result, Err(ApplyError::NotFound));
    }

    #[test]
    fn an_unknown_or_empty_path_is_not_found() {
        let value = EditorValue::Number(2.0);
        assert_eq!(
            apply(&mut outer(), &PropertyPath::new(["missing"]), &value),
            Err(ApplyError::NotFound)
        );
        assert_eq!(
            apply(&mut outer(), &PropertyPath::default(), &value),
            Err(ApplyError::NotFound)
        );
    }

    #[test]
    fn a_rejected_value_leaves_the_leaf_unchanged() {
        let mut value = outer();
        let result = apply(
            &mut value,
            &PropertyPath::new(["position"]),
            &EditorValue::Number(2.0),
        );
        assert_eq!(result, Err(ApplyError::Rejected));
        assert_eq!(value.position, Vec3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn path_name_is_the_last_segment() {
        assert_eq!(PropertyPath::new(["inner", "weight"]).name(), "weight");
        assert_eq!(PropertyPath::default().name(), "");
    }
}
```

**Implementation**, above the test module:

```rust
use std::any::{Any, TypeId};

use crate::{Editable, EditorValue, PropertyVisitor, PropertyVisitorMut};

/// Where a leaf sits inside its component: `["translation"]`, or
/// `["bar", "weight"]` for a nested struct field.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct PropertyPath(Vec<&'static str>);

impl PropertyPath {
    pub fn new(segments: impl IntoIterator<Item = &'static str>) -> Self {
        Self(segments.into_iter().collect())
    }

    pub fn segments(&self) -> &[&'static str] {
        &self.0
    }

    /// The last segment, or `""` for the empty path.
    pub fn name(&self) -> &'static str {
        self.0.last().copied().unwrap_or("")
    }
}

/// One leaf, flattened out of a component.
#[derive(Clone, Debug, PartialEq)]
pub struct Property {
    pub path: PropertyPath,
    /// The leaf's concrete type, which widgets are registered against.
    pub type_id: TypeId,
    pub value: EditorValue,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ApplyError {
    /// No leaf at that path.
    NotFound,
    /// The leaf refused the value: wrong shape or not finite.
    Rejected,
}

pub fn collect(root: &dyn Editable) -> Vec<Property> {
    let mut collector = Collect {
        path: Vec::new(),
        properties: Vec::new(),
    };
    root.visit(&mut collector);
    collector.properties
}

pub fn apply(
    root: &mut dyn Editable,
    path: &PropertyPath,
    value: &EditorValue,
) -> Result<(), ApplyError> {
    let mut applier = Apply {
        target: path.segments(),
        depth: 0,
        value,
        result: None,
    };
    root.visit_mut(&mut applier);
    applier.result.unwrap_or(Err(ApplyError::NotFound))
}

struct Collect {
    path: Vec<&'static str>,
    properties: Vec<Property>,
}

impl PropertyVisitor for Collect {
    fn field(&mut self, name: &'static str, value: &dyn Editable) {
        self.path.push(name);
        match value.read() {
            Some(leaf) => self.properties.push(Property {
                path: PropertyPath(self.path.clone()),
                // Must upcast: `.type_id()` on a `&&dyn Editable` is the reference's own id.
                type_id: (value as &dyn Any).type_id(),
                value: leaf,
            }),
            None => value.visit(self),
        }
        self.path.pop();
    }
}

struct Apply<'a> {
    target: &'a [&'static str],
    depth: usize,
    value: &'a EditorValue,
    result: Option<Result<(), ApplyError>>,
}

impl PropertyVisitorMut for Apply<'_> {
    fn field(&mut self, name: &'static str, value: &mut dyn Editable) {
        if self.result.is_some() || self.target.get(self.depth) != Some(&name) {
            return;
        }
        let at_end = self.depth + 1 == self.target.len();
        match (at_end, value.read().is_some()) {
            (true, true) => {
                self.result = Some(if value.write(self.value) {
                    Ok(())
                } else {
                    Err(ApplyError::Rejected)
                });
            }
            (false, false) => {
                self.depth += 1;
                value.visit_mut(self);
                self.depth -= 1;
            }
            _ => self.result = Some(Err(ApplyError::NotFound)),
        }
    }
}
```

- [ ] **Step 8: Run all crate tests**

Run: `cargo test -p editable`
Expected: every test in `leaves` and `property` PASSES.

- [ ] **Step 9: Stage**

```bash
git add crates/editable
```

---

### Task 3: `#[derive(Editable)]` and `Transform`

**Files:**
- Modify: `crates/editable/macros/src/lib.rs` (replace the stub)
- Modify: `crates/editable/src/lib.rs` (doctests on the derive re-export)
- Create: `crates/editable/tests/derive.rs`
- Modify: `crates/essential/Cargo.toml`, `crates/essential/src/transform/mod.rs`

**Interfaces:**
- Consumes: everything Task 2 produces.
- Produces: `#[derive(editable::Editable)]` for named-field structs; `essential::transform::Transform: Editable` with properties `translation` (`Vec3`), `rotation` (`Quat`), `scale` (`Vec3`).

- [ ] **Step 1: Write the failing derive tests**

`crates/editable/tests/derive.rs`:

```rust
use std::any::TypeId;

use editable::{Editable, EditorValue, Property, PropertyPath, apply, collect};
use glam::{Quat, Vec3};

#[derive(Editable)]
struct Inner {
    weight: f32,
}

#[derive(Editable)]
struct Outer {
    position: Vec3,
    rotation: Quat,
    inner: Inner,
}

fn outer() -> Outer {
    Outer {
        position: Vec3::X,
        rotation: Quat::IDENTITY,
        inner: Inner { weight: 1.0 },
    }
}

#[test]
fn derived_structs_collect_nested_leaves_in_declaration_order() {
    let properties = collect(&outer());
    let shape: Vec<_> = properties
        .iter()
        .map(|property| (property.path.segments().to_vec(), property.type_id))
        .collect();
    assert_eq!(
        shape,
        vec![
            (vec!["position"], TypeId::of::<Vec3>()),
            (vec!["rotation"], TypeId::of::<Quat>()),
            (vec!["inner", "weight"], TypeId::of::<f32>()),
        ]
    );
    assert_eq!(
        properties[2],
        Property {
            path: PropertyPath::new(["inner", "weight"]),
            type_id: TypeId::of::<f32>(),
            value: EditorValue::Number(1.0),
        }
    );
}

#[test]
fn derived_structs_apply_through_visit_mut() {
    let mut value = outer();
    apply(
        &mut value,
        &PropertyPath::new(["inner", "weight"]),
        &EditorValue::Number(4.0),
    )
    .unwrap();
    assert_eq!(value.inner.weight, 4.0);
}

#[test]
fn a_derived_struct_is_not_a_leaf() {
    assert_eq!(outer().read(), None);
}
```

- [ ] **Step 2: Run and confirm failure**

Run: `cargo test -p editable --test derive`
Expected: FAIL — compile errors, `the trait Editable is not implemented for Outer` (the stub derive emits nothing).

- [ ] **Step 3: Implement the derive**

Replace `crates/editable/macros/src/lib.rs`:

```rust
use proc_macro::TokenStream;
use quote::{quote, quote_spanned};
use syn::{Data, DataStruct, DeriveInput, Fields, spanned::Spanned};

#[proc_macro_derive(Editable)]
pub fn derive_editable(input: TokenStream) -> TokenStream {
    let ast = syn::parse_macro_input!(input as DeriveInput);
    let name = &ast.ident;
    let Data::Struct(DataStruct {
        fields: Fields::Named(fields),
        ..
    }) = &ast.data
    else {
        return syn::Error::new_spanned(
            name,
            "#[derive(Editable)] supports structs with named fields only",
        )
        .to_compile_error()
        .into();
    };

    // Spanned on each field's type so a non-`Editable` field is reported there.
    let visits = fields.named.iter().map(|field| {
        let ident = field.ident.as_ref().expect("named field");
        let label = ident.to_string();
        quote_spanned!(field.ty.span()=> visitor.field(#label, &self.#ident);)
    });
    let visits_mut = fields.named.iter().map(|field| {
        let ident = field.ident.as_ref().expect("named field");
        let label = ident.to_string();
        quote_spanned!(field.ty.span()=> visitor.field(#label, &mut self.#ident);)
    });

    let (impl_generics, type_generics, where_clause) = ast.generics.split_for_impl();
    quote! {
        impl #impl_generics ::editable::Editable for #name #type_generics #where_clause {
            fn visit(&self, visitor: &mut dyn ::editable::PropertyVisitor) {
                #(#visits)*
            }

            fn visit_mut(&mut self, visitor: &mut dyn ::editable::PropertyVisitorMut) {
                #(#visits_mut)*
            }
        }
    }
    .into()
}
```

- [ ] **Step 4: Run the derive tests**

Run: `cargo test -p editable --test derive`
Expected: PASS.

- [ ] **Step 5: Add the compile-fail doctests**

In `crates/editable/src/lib.rs`, replace `pub use editable_macros::Editable;` with:

```rust
/// Implements [`Editable`] for a struct with named fields by visiting each
/// field in declaration order. Every field must itself be `Editable`.
///
/// ```compile_fail
/// #[derive(editable::Editable)]
/// struct Tuple(f32);
/// ```
///
/// ```compile_fail
/// #[derive(editable::Editable)]
/// enum Choice { A, B }
/// ```
///
/// ```compile_fail
/// struct Opaque;
///
/// #[derive(editable::Editable)]
/// struct HasOpaque { inner: Opaque }
/// ```
pub use editable_macros::Editable;
```

- [ ] **Step 6: Run the doctests**

Run: `cargo test -p editable --doc`
Expected: 3 doctests PASS (each fails to compile, as required).

- [ ] **Step 7: Write the failing `Transform` test**

In `crates/essential/Cargo.toml` `[dependencies]`, add:

```toml
editable = { path = "../editable" }
```

Append to `crates/essential/src/transform/mod.rs`:

```rust
#[cfg(test)]
mod editable_tests {
    use std::any::TypeId;

    use editable::{EditorValue, PropertyPath, collect};
    use glam::{Quat, Vec3};

    use super::Transform;

    #[test]
    fn transform_exposes_translation_rotation_and_scale() {
        let transform = Transform::from_translation_rotation_scale(
            Vec3::new(1.0, 2.0, 3.0),
            Quat::from_rotation_y(90_f32.to_radians()),
            Vec3::ONE,
        );
        let properties = collect(&transform);

        let paths: Vec<_> = properties.iter().map(|p| p.path.clone()).collect();
        assert_eq!(
            paths,
            vec![
                PropertyPath::new(["translation"]),
                PropertyPath::new(["rotation"]),
                PropertyPath::new(["scale"]),
            ]
        );
        assert_eq!(properties[1].type_id, TypeId::of::<Quat>());
        assert_eq!(properties[0].value, EditorValue::Vec3([1.0, 2.0, 3.0]));
        let EditorValue::Vec3([_, yaw, _]) = properties[1].value else {
            panic!("rotation must be presented as a Vec3 of degrees");
        };
        assert!((yaw - 90.0).abs() < 1e-3);
    }
}
```

Note `essential` is edition 2021: `use editable::{..., collect}` works as written.

- [ ] **Step 8: Run and confirm failure**

Run: `cargo test -p essential editable_tests`
Expected: compile error, `the trait bound Transform: Editable is not satisfied`.

- [ ] **Step 9: Derive on `Transform`**

In `crates/essential/src/transform/mod.rs` add `use editable::Editable;` to the imports and change the derive line on `Transform` to:

```rust
#[derive(Clone, Blendable, Editable, serde::Serialize, serde::Deserialize)]
```

- [ ] **Step 10: Run the tests and the editor build**

Run: `cargo test -p essential editable_tests && cargo test -p editable && cargo build -p editor`
Expected: PASS, and the editor builds.

- [ ] **Step 11: Stage**

```bash
git add crates/editable crates/essential/Cargo.toml crates/essential/src/transform/mod.rs Cargo.lock
```

---

### Task 4: `UITextInputSubmitted` and `UITextInputCancelled`

**Files:**
- Modify: `crates/ui/src/text_input.rs`
- Modify: `crates/ui/src/plugin.rs` (event registration, ~line 90)

**Interfaces:**
- Produces: `ui::text_input::UITextInputSubmitted { pub entity: Entity, pub value: String }`, `ui::text_input::UITextInputCancelled { pub entity: Entity }`, both registered by `UIPlugin`, fired by `update_text_inputs` for the focused field on Enter/NumpadEnter and Escape respectively.

- [ ] **Step 1: Write the failing test**

Add to the existing `mod tests` in `crates/ui/src/text_input.rs`:

```rust
    #[test]
    fn enter_submits_and_escape_cancels() {
        assert_eq!(finish_key(|key| key == KeyCode::Enter), Some(Finish::Submit));
        assert_eq!(
            finish_key(|key| key == KeyCode::NumpadEnter),
            Some(Finish::Submit)
        );
        assert_eq!(finish_key(|key| key == KeyCode::Escape), Some(Finish::Cancel));
        assert_eq!(finish_key(|key| key == KeyCode::KeyA), None);
    }
```

- [ ] **Step 2: Run and confirm failure**

Run: `cargo test -p ui text_input`
Expected: compile error, `cannot find function finish_key`.

- [ ] **Step 3: Implement the decision function and events**

In `crates/ui/src/text_input.rs`, after `UITextInputChanged`:

```rust
/// Fired when Enter is pressed in a focused [`UITextInput`].
#[derive(Event)]
pub struct UITextInputSubmitted {
    pub entity: Entity,
    pub value: String,
}

/// Fired when Escape is pressed in a focused [`UITextInput`].
#[derive(Event)]
pub struct UITextInputCancelled {
    pub entity: Entity,
}

#[derive(Debug, PartialEq, Eq)]
enum Finish {
    Submit,
    Cancel,
}

fn finish_key(just_pressed: impl Fn(KeyCode) -> bool) -> Option<Finish> {
    if just_pressed(KeyCode::Enter) || just_pressed(KeyCode::NumpadEnter) {
        Some(Finish::Submit)
    } else if just_pressed(KeyCode::Escape) {
        Some(Finish::Cancel)
    } else {
        None
    }
}
```

Add two parameters to `update_text_inputs`, after `mut writer: EventWriter<UITextInputChanged>`:

```rust
    mut submitted: EventWriter<UITextInputSubmitted>,
    mut cancelled: EventWriter<UITextInputCancelled>,
```

Inside `if is_focused { ... }`, directly after the existing `if changed { writer.write(...); }` block:

```rust
            match finish_key(|key| input.is_just_pressed(PhysicalKey::Code(key))) {
                Some(Finish::Submit) => {
                    submitted.write(UITextInputSubmitted {
                        entity,
                        value: text_input.value.clone(),
                    });
                }
                Some(Finish::Cancel) => {
                    cancelled.write(UITextInputCancelled { entity });
                }
                None => {}
            }
```

In `crates/ui/src/plugin.rs`, after `app.register_event::<UITextInputChanged>();`:

```rust
        app.register_event::<UITextInputSubmitted>();
        app.register_event::<UITextInputCancelled>();
```

and extend that file's `text_input::{...}` import with `UITextInputCancelled, UITextInputSubmitted`.

- [ ] **Step 4: Run the tests and the editor build**

Run: `cargo test -p ui text_input && cargo build -p editor`
Expected: PASS; editor builds.

- [ ] **Step 5: Stage**

```bash
git add crates/ui/src/text_input.rs crates/ui/src/plugin.rs
```

---

### Task 5: Editor registry, row contract and commit application

**Files:**
- Move: `crates/editor/src/inspector.rs` → `crates/editor/src/inspector/mod.rs` (`git mv`, no content change in this task beyond declaring submodules)
- Create: `crates/editor/src/inspector/rows.rs`, `crates/editor/src/inspector/registry.rs`
- Modify: `crates/editor/Cargo.toml`

**Interfaces:**
- Consumes: `editable::{Editable, EditorValue, Property, PropertyPath, ApplyError, collect, apply}`; `World::get_component_for_entity{,_mut}`, `World::was_component_changed`.
- Produces (all `pub`, in `crate::inspector::{rows, registry}`, re-exported from `crate::inspector`):
  - `#[derive(Component, Clone)] pub struct PropertyRow { pub entity: Entity, pub component: TypeId, pub path: PropertyPath }`
  - `#[derive(Component)] pub struct PropertyRowValue(pub EditorValue)`
  - `pub struct PropertyCommit { pub row: PropertyRow, pub value: EditorValue }`
  - `#[derive(Resource, Default)] pub struct PropertyCommits(pub Vec<PropertyCommit>)`
  - `pub trait PropertyWidget: Send + Sync + 'static { fn build(&self, cmd: &mut CommandQueue, row: Entity, value: &EditorValue, theme: &UITheme); }`
  - `#[derive(Resource, Default)] pub struct InspectorRegistry` with `register_component<T: Component + Editable>(&mut self)`, `register_widget<T: Editable>(&mut self, widget: Box<dyn PropertyWidget>)`, `pub(crate) fn component(&self, id: TypeId) -> Option<EditableComponent>`, `pub(crate) fn widget(&self, id: TypeId) -> Option<&dyn PropertyWidget>`
  - `#[derive(Clone, Copy)] pub(crate) struct EditableComponent { pub name: &'static str, pub collect: fn(&World, Entity) -> Option<Vec<Property>>, pub apply: fn(&mut World, Entity, &PropertyPath, &EditorValue) -> Option<Result<(), ApplyError>> }`
  - `pub trait EditableApp { fn register_editable<T: Component + Editable>(&mut self) -> &mut Self; fn register_property_widget<T: Editable>(&mut self, widget: Box<dyn PropertyWidget>) -> &mut Self; }` for `App`
  - `pub(crate) fn apply_property_commits(world: &mut World)`

- [ ] **Step 1: Move the module and add the dependency**

```bash
mkdir -p crates/editor/src/inspector
git mv crates/editor/src/inspector.rs crates/editor/src/inspector/mod.rs
```

In `crates/editor/Cargo.toml` `[dependencies]` add:

```toml
editable = { path = "../editable" }
```

The old string-keyed `InspectorRegistry` in `mod.rs` collides with the new one. In this task, rename the old one in `mod.rs` to `FormatterRegistry` (every occurrence: the struct, its `impl`, `InspectedApp`'s body, `InspectorPlugin::build`, `refresh_inspector`'s parameter, and the tests). Task 7 deletes it.

Create empty `crates/editor/src/inspector/rows.rs` and `crates/editor/src/inspector/registry.rs`, and at the top of `mod.rs`, below the imports:

```rust
mod registry;
mod rows;
```

Run: `cargo build -p editor`
Expected: builds.

- [ ] **Step 2: Write `rows.rs`**

`crates/editor/src/inspector/rows.rs`:

```rust
use std::any::TypeId;

use ecs::{command::CommandQueue, Component, Entity, Resource};
use editable::{EditorValue, PropertyPath};
use ui::theme::UITheme;

/// What a commit on this row edits. Captured at rebuild and never re-derived at
/// commit time, so an edit lands on the entity it was typed against.
#[derive(Component, Clone)]
pub struct PropertyRow {
    pub entity: Entity,
    pub component: TypeId,
    pub path: PropertyPath,
}

/// The leaf's current value, refreshed by the inspector when it changes.
#[derive(Component)]
pub struct PropertyRowValue(pub EditorValue);

/// A widget's finished edit. The inspector applies these; widgets never touch
/// the world.
pub struct PropertyCommit {
    pub row: PropertyRow,
    pub value: EditorValue,
}

/// Commits waiting for the inspector to apply them. A queue rather than an
/// event because the applying system is exclusive and cannot hold a reader.
#[derive(Resource, Default)]
pub struct PropertyCommits(pub Vec<PropertyCommit>);

/// Builds the UI for one property row. One instance serves every row of its
/// leaf type, so per-row state belongs on the entities it spawns.
pub trait PropertyWidget: Send + Sync + 'static {
    /// Spawns this widget's UI under `row` for a property holding `value`. The
    /// widget's own systems read the row's [`PropertyRowValue`] and push to
    /// [`PropertyCommits`].
    fn build(&self, cmd: &mut CommandQueue, row: Entity, value: &EditorValue, theme: &UITheme);
}
```

- [ ] **Step 3: Write the failing registry tests**

`crates/editor/src/inspector/registry.rs` test module:

```rust
#[cfg(test)]
mod tests {
    use std::any::TypeId;

    use ecs::{Entity, World};
    use editable::{EditorValue, PropertyPath};
    use essential::transform::Transform;
    use glam::Vec3;

    use super::*;
    use crate::inspector::rows::{PropertyCommit, PropertyCommits, PropertyRow};

    fn world_with_two_transforms() -> (World, Entity, Entity) {
        let mut world = World::default();
        let mut registry = InspectorRegistry::default();
        registry.register_component::<Transform>();
        world.insert_resource(registry);
        world.insert_resource(PropertyCommits::default());
        let a = world.spawn(Transform::IDENTITY);
        let b = world.spawn(Transform::IDENTITY);
        world.tick();
        (world, a, b)
    }

    fn commit(entity: Entity, path: &'static str, value: EditorValue) -> PropertyCommit {
        PropertyCommit {
            row: PropertyRow {
                entity,
                component: TypeId::of::<Transform>(),
                path: PropertyPath::new([path]),
            },
            value,
        }
    }

    fn translation(world: &World, entity: Entity) -> Vec3 {
        world
            .get_component_for_entity::<Transform>(entity)
            .unwrap()
            .translation
    }

    #[test]
    fn a_commit_lands_on_the_entity_its_row_was_built_for() {
        let (mut world, a, b) = world_with_two_transforms();
        // The row was built while A was selected; the selection has since moved
        // to B. Applying must not consult the selection.
        let mut selection = crate::selection::Selection::default();
        selection.select_entity(b);
        world.insert_resource(selection);
        world
            .get_resource_mut::<PropertyCommits>()
            .unwrap()
            .0
            .push(commit(a, "translation", EditorValue::Vec3([1.0, 2.0, 3.0])));

        apply_property_commits(&mut world);

        assert_eq!(translation(&world, a), Vec3::new(1.0, 2.0, 3.0));
        assert!(world.was_component_changed(a, TypeId::of::<Transform>()));
        assert_eq!(translation(&world, b), Vec3::ZERO);
        assert!(!world.was_component_changed(b, TypeId::of::<Transform>()));
        assert!(world.get_resource::<PropertyCommits>().unwrap().0.is_empty());
    }

    #[test]
    fn collecting_properties_does_not_mark_the_component_changed() {
        let (world, a, _) = world_with_two_transforms();
        let collect = world
            .get_resource::<InspectorRegistry>()
            .unwrap()
            .component(TypeId::of::<Transform>())
            .unwrap()
            .collect;

        let properties = collect(&world, a).expect("A carries a Transform");

        assert_eq!(properties.len(), 3);
        assert!(!world.was_component_changed(a, TypeId::of::<Transform>()));
    }

    #[test]
    fn a_rejected_commit_leaves_the_component_unchanged() {
        let (mut world, a, _) = world_with_two_transforms();
        world
            .get_resource_mut::<PropertyCommits>()
            .unwrap()
            .0
            .push(commit(a, "translation", EditorValue::Number(5.0)));

        apply_property_commits(&mut world);

        assert_eq!(translation(&world, a), Vec3::ZERO);
    }

    #[test]
    fn a_commit_for_a_despawned_entity_is_dropped() {
        let (mut world, a, _) = world_with_two_transforms();
        world.despawn(a);
        world
            .get_resource_mut::<PropertyCommits>()
            .unwrap()
            .0
            .push(commit(a, "translation", EditorValue::Vec3([1.0, 1.0, 1.0])));

        apply_property_commits(&mut world);

        assert!(world.get_resource::<PropertyCommits>().unwrap().0.is_empty());
    }

    #[test]
    fn the_display_name_is_the_last_path_segment_of_the_type() {
        let registry = {
            let mut registry = InspectorRegistry::default();
            registry.register_component::<Transform>();
            registry
        };
        assert_eq!(
            registry.component(TypeId::of::<Transform>()).unwrap().name,
            "Transform"
        );
    }
}
```

- [ ] **Step 4: Run and confirm failure**

Run: `cargo test -p editor inspector::registry`
Expected: compile errors, `cannot find type InspectorRegistry in this scope` (and `apply_property_commits`).

- [ ] **Step 5: Implement the registry**

Above the test module in `registry.rs`:

```rust
use std::{any::TypeId, collections::HashMap};

use app::App;
use ecs::{Component, Entity, Resource, World};
use editable::{ApplyError, Editable, EditorValue, Property, PropertyPath};

use crate::inspector::rows::{PropertyCommits, PropertyWidget};

#[derive(Clone, Copy)]
pub(crate) struct EditableComponent {
    /// Display only.
    pub name: &'static str,
    /// `None` when the entity does not carry the component.
    pub collect: fn(&World, Entity) -> Option<Vec<Property>>,
    /// `None` when the entity does not carry the component.
    pub apply: fn(&mut World, Entity, &PropertyPath, &EditorValue) -> Option<Result<(), ApplyError>>,
}

/// What the inspector can edit, keyed by `TypeId`: components it opens into
/// rows, and leaf types with a widget other than the default for their value.
#[derive(Resource, Default)]
pub struct InspectorRegistry {
    components: HashMap<TypeId, EditableComponent>,
    widgets: HashMap<TypeId, Box<dyn PropertyWidget>>,
}

impl InspectorRegistry {
    pub fn register_component<T: Component + Editable>(&mut self) {
        let path = std::any::type_name::<T>();
        self.components.insert(
            TypeId::of::<T>(),
            EditableComponent {
                name: path.rsplit("::").next().unwrap_or(path),
                collect: collect_typed::<T>,
                apply: apply_typed::<T>,
            },
        );
    }

    pub fn register_widget<T: Editable>(&mut self, widget: Box<dyn PropertyWidget>) {
        self.widgets.insert(TypeId::of::<T>(), widget);
    }

    pub(crate) fn component(&self, id: TypeId) -> Option<EditableComponent> {
        self.components.get(&id).copied()
    }

    pub(crate) fn widget(&self, id: TypeId) -> Option<&dyn PropertyWidget> {
        self.widgets.get(&id).map(|widget| widget.as_ref())
    }
}

fn collect_typed<T: Component + Editable>(world: &World, entity: Entity) -> Option<Vec<Property>> {
    world
        .get_component_for_entity::<T>(entity)
        .map(|component| editable::collect(component))
}

fn apply_typed<T: Component + Editable>(
    world: &mut World,
    entity: Entity,
    path: &PropertyPath,
    value: &EditorValue,
) -> Option<Result<(), ApplyError>> {
    world
        .get_component_for_entity_mut::<T>(entity)
        .map(|component| editable::apply(component, path, value))
}

pub trait EditableApp {
    /// Makes a component's properties editable in the inspector.
    fn register_editable<T: Component + Editable>(&mut self) -> &mut Self;
    /// Replaces the default widget for leaves of type `T`.
    fn register_property_widget<T: Editable>(&mut self, widget: Box<dyn PropertyWidget>) -> &mut Self;
}

impl EditableApp for App {
    fn register_editable<T: Component + Editable>(&mut self) -> &mut Self {
        registry(self).register_component::<T>();
        self
    }

    fn register_property_widget<T: Editable>(&mut self, widget: Box<dyn PropertyWidget>) -> &mut Self {
        registry(self).register_widget::<T>(widget);
        self
    }
}

fn registry(app: &mut App) -> &mut InspectorRegistry {
    app.get_resource_mut::<InspectorRegistry>()
        .expect("InspectorPlugin must be registered first")
}

/// Applies queued commits. A `Rejected` write still stamps the changed tick:
/// the mutable borrow is taken before the leaf decides.
pub(crate) fn apply_property_commits(world: &mut World) {
    let Some(commits) = world.get_resource_mut::<PropertyCommits>() else {
        return;
    };
    let commits = std::mem::take(&mut commits.0);
    for commit in commits {
        let row = &commit.row;
        let Some(component) = world
            .get_resource::<InspectorRegistry>()
            .and_then(|registry| registry.component(row.component))
        else {
            log::warn!("Property commit for an unregistered component at {:?}", row.path);
            continue;
        };
        match (component.apply)(world, row.entity, &row.path, &commit.value) {
            Some(Ok(())) => {}
            Some(Err(ApplyError::Rejected)) => log::warn!(
                "{}.{:?} rejected {:?}",
                component.name,
                row.path,
                commit.value
            ),
            Some(Err(ApplyError::NotFound)) => {
                log::warn!("{} has no property at {:?}", component.name, row.path)
            }
            None => log::debug!(
                "Dropped a commit to {}: the entity no longer carries it",
                component.name
            ),
        }
    }
}
```

`editor` already depends on `essential` and `glam` (tests use them) and `log`.

Then in `mod.rs`, below `mod rows;`:

```rust
pub use registry::{EditableApp, InspectorRegistry};
pub use rows::{PropertyCommit, PropertyCommits, PropertyRow, PropertyRowValue, PropertyWidget};
```

- [ ] **Step 6: Run the tests**

Run: `cargo test -p editor inspector::registry`
Expected: 5 tests PASS. (`EditableApp`, `PropertyWidget`, `PropertyRowValue` are unused outside tests until Task 6/7; `editor` does not deny warnings, so dead-code warnings are acceptable at this point.)

- [ ] **Step 7: Stage**

```bash
git add crates/editor/Cargo.toml crates/editor/src/inspector Cargo.lock
```

---

### Task 6: `NumericFields`, the default widget

**Files:**
- Create: `crates/editor/src/inspector/numeric.rs`
- Modify: `crates/editor/src/inspector/mod.rs` (declare the module only)

**Interfaces:**
- Consumes: `PropertyRow`, `PropertyRowValue`, `PropertyCommit`, `PropertyCommits`, `PropertyWidget` (Task 5); `UITextInputSubmitted`, `UITextInputCancelled` (Task 4); `ui::focus::{FocusedWidget, UIFocusLost, UIFocusable}`.
- Produces (in `crate::inspector::numeric`):
  - `pub struct NumericFields;` implementing `PropertyWidget`
  - `pub(crate) static NUMERIC_FIELDS: NumericFields`
  - `pub(crate) fn commit_numeric_fields(...)`, `pub(crate) fn cancel_numeric_fields(...)`, `pub(crate) fn refresh_numeric_fields(...)` — systems, registered by Task 7

- [ ] **Step 1: Declare the module**

In `crates/editor/src/inspector/mod.rs`, next to `mod registry;`:

```rust
mod numeric;
```

- [ ] **Step 2: Write the failing tests**

`crates/editor/src/inspector/numeric.rs` test module:

```rust
#[cfg(test)]
mod tests {
    use editable::EditorValue;

    use super::*;

    #[test]
    fn unchanged_text_does_not_commit() {
        let value = EditorValue::Vec3([1.23456, 0.0, 0.0]);
        let displayed = format_slot(&value, 0);
        assert_eq!(displayed, "1.235");
        assert_eq!(
            numeric_commit(&value, 0, &displayed, &displayed),
            None,
            "tabbing through a rounded field must not truncate the stored value"
        );
    }

    #[test]
    fn unparseable_text_does_not_commit() {
        let value = EditorValue::Number(1.0);
        assert_eq!(numeric_commit(&value, 0, "abc", "1.000"), None);
        assert_eq!(numeric_commit(&value, 0, "", "1.000"), None);
    }

    #[test]
    fn a_slot_is_replaced_within_a_vec3() {
        let value = EditorValue::Vec3([1.0, 2.0, 3.0]);
        assert_eq!(
            numeric_commit(&value, 1, " -4.5 ", "2.000"),
            Some(EditorValue::Vec3([1.0, -4.5, 3.0]))
        );
    }

    #[test]
    fn a_number_has_one_slot() {
        let value = EditorValue::Number(1.0);
        assert_eq!(
            numeric_commit(&value, 0, "7", "1.000"),
            Some(EditorValue::Number(7.0))
        );
        assert_eq!(numeric_commit(&value, 1, "7", "1.000"), None);
        assert_eq!(slot_count(&value), 1);
        assert_eq!(slot_count(&EditorValue::Vec3([0.0; 3])), 3);
    }
}
```

- [ ] **Step 3: Run and confirm failure**

Run: `cargo test -p editor inspector::numeric`
Expected: compile error, `cannot find function format_slot` (and `numeric_commit`, `slot_count`).

- [ ] **Step 4: Implement**

Above the test module in `numeric.rs`:

```rust
use ecs::{
    command::CommandQueue, events::event_reader::EventReader, Component, Entity, Query, Res,
    ResMut,
};
use editable::EditorValue;
use ui::{
    focus::{FocusedWidget, UIFocusLost, UIFocusable},
    interaction::Interactable,
    material::UIMaterial,
    node::{UINode, UIRect},
    text::TextComponent,
    text_input::{UITextInput, UITextInputCancelled, UITextInputSubmitted},
    theme::UITheme,
    transform::UIValue,
};

use crate::inspector::rows::{
    PropertyCommit, PropertyCommits, PropertyRow, PropertyRowValue, PropertyWidget,
};

/// One text field per number: one for `Number`, three for `Vec3`.
pub struct NumericFields;

pub(crate) static NUMERIC_FIELDS: NumericFields = NumericFields;

/// One field of a [`NumericFields`] row.
#[derive(Component)]
pub(crate) struct NumericSlot {
    row: Entity,
    slot: usize,
    /// The text last written into the field, which is what a commit compares
    /// against so display rounding never becomes an edit.
    displayed: String,
}

impl PropertyWidget for NumericFields {
    fn build(&self, cmd: &mut CommandQueue, row: Entity, value: &EditorValue, theme: &UITheme) {
        for slot in 0..slot_count(value) {
            let field = cmd
                .spawn((
                    UINode {
                        flex_grow: 1.0,
                        height: UIValue::Px(theme.control_height),
                        padding: UIRect::axes(0.0, theme.spacing_xs),
                        ..Default::default()
                    }
                    .clipped(),
                    TextComponent {
                        color: theme.text,
                        font_size: theme.font_size_sm,
                        line_height: theme.line_height(theme.font_size_sm),
                        wrap: false,
                        ..Default::default()
                    },
                    UITextInput::new(""),
                    UIMaterial {
                        corner_radius: theme.radius_md,
                        ..UIMaterial::with_border(theme.canvas, theme.border, 1.0)
                    },
                    Interactable,
                    UIFocusable,
                    NumericSlot {
                        row,
                        slot,
                        displayed: String::new(),
                    },
                ))
                .entity();
            cmd.add_child(row, field);
        }
    }
}

fn slot_count(value: &EditorValue) -> usize {
    match value {
        EditorValue::Number(_) => 1,
        EditorValue::Vec3(_) => 3,
    }
}

fn format_slot(value: &EditorValue, slot: usize) -> String {
    let number = match value {
        EditorValue::Number(number) => (slot == 0).then_some(*number),
        EditorValue::Vec3(components) => components.get(slot).copied(),
    };
    number.map(|n| format!("{n:.3}")).unwrap_or_default()
}

/// The value to commit when `slot` finishes editing with `text`, or `None` to
/// commit nothing.
fn numeric_commit(
    current: &EditorValue,
    slot: usize,
    text: &str,
    displayed: &str,
) -> Option<EditorValue> {
    if text == displayed {
        return None;
    }
    let number: f64 = text.trim().parse().ok()?;
    let mut value = current.clone();
    match &mut value {
        EditorValue::Number(existing) if slot == 0 => *existing = number,
        EditorValue::Vec3(components) => *components.get_mut(slot)? = number,
        EditorValue::Number(_) => return None,
    }
    Some(value)
}

pub(crate) fn commit_numeric_fields(
    mut submitted: EventReader<UITextInputSubmitted>,
    mut lost: EventReader<UIFocusLost>,
    rows: Query<(&PropertyRow, &PropertyRowValue)>,
    fields: Query<(&mut NumericSlot, &UITextInput)>,
    mut commits: ResMut<PropertyCommits>,
) {
    let finished: Vec<Entity> = submitted
        .read()
        .map(|event| event.entity)
        .chain(lost.read().map(|event| event.0))
        .collect();
    for entity in finished {
        let Some((mut slot, input)) = fields.get_entity(entity) else {
            continue;
        };
        let Some((row, current)) = rows.get_entity(slot.row) else {
            continue;
        };
        if let Some(value) = numeric_commit(&current.0, slot.slot, &input.value, &slot.displayed) {
            commits.0.push(PropertyCommit {
                row: row.clone(),
                value,
            });
            // Enter keeps focus; without this the focus loss that follows would
            // commit the same text again.
            slot.displayed = input.value.clone();
        }
    }
}

pub(crate) fn cancel_numeric_fields(
    mut cancelled: EventReader<UITextInputCancelled>,
    fields: Query<(&NumericSlot, &mut UITextInput)>,
    mut focused: ResMut<FocusedWidget>,
) {
    for event in cancelled.read() {
        let Some((slot, mut input)) = fields.get_entity(event.entity) else {
            continue;
        };
        input.value = slot.displayed.clone();
        input.cursor = input.value.len();
        input.selection_anchor = None;
        if **focused == Some(event.entity) {
            **focused = None;
        }
    }
}

/// Writes each row's value into its fields, except the focused one: its text is
/// the user's edit buffer, and the value re-derived from the component (e.g.
/// euler angles from a quaternion) need not match what they typed.
pub(crate) fn refresh_numeric_fields(
    focused: Res<FocusedWidget>,
    rows: Query<&PropertyRowValue>,
    fields: Query<(Entity, &mut NumericSlot, &mut UITextInput)>,
) {
    for (entity, mut slot, mut input) in fields.iter() {
        if **focused == Some(entity) {
            continue;
        }
        let Some(value) = rows.get_entity(slot.row) else {
            continue;
        };
        let text = format_slot(&value.0, slot.slot);
        if input.value != text {
            input.value = text.clone();
            input.cursor = input.value.len();
            input.selection_anchor = None;
        }
        if slot.displayed != text {
            slot.displayed = text;
        }
    }
}
```

If `Query::get_entity` or the item types differ from what this code assumes, match the usage in `crates/ui/src/widgets.rs` (`collapsibles.get_entity(click.entity)` yields a mutable item directly) and in `crates/editor/src/inspector/mod.rs`.

- [ ] **Step 5: Run the tests and build**

Run: `cargo test -p editor inspector::numeric && cargo build -p editor`
Expected: 4 tests PASS; editor builds.

- [ ] **Step 6: Stage**

```bash
git add crates/editor/src/inspector/numeric.rs crates/editor/src/inspector/mod.rs
```

---

### Task 7: Inspector rows, wiring, and removing the formatters

**Files:**
- Modify: `crates/editor/src/inspector/mod.rs`

**Interfaces:**
- Consumes: `World::component_ids`, `World::type_info` (Task 1); `InspectorRegistry`, `EditableApp`, `apply_property_commits` (Task 5); `NUMERIC_FIELDS`, `commit_numeric_fields`, `cancel_numeric_fields`, `refresh_numeric_fields` (Task 6); rows types (Task 5).
- Produces: a working editable inspector. `InspectedComponent` becomes `{ pub name: String, pub type_id: TypeId, pub properties: Option<Vec<Property>> }`; `InspectorData` gains `pub entity: Option<Entity>`.

- [ ] **Step 1: Write the failing test**

Replace the whole `#[cfg(test)] mod tests` in `mod.rs` with:

```rust
#[cfg(test)]
mod tests {
    use ecs::{
        component::scene::{SceneComponent, SceneSpawnContext},
        Component, World,
    };
    use essential::transform::Transform;
    use serde::{Deserialize, Serialize};

    use super::*;

    #[derive(Component, Serialize, Deserialize)]
    struct Tag;

    impl SceneComponent for Tag {
        fn apply(self, entity: Entity, ctx: &mut SceneSpawnContext<'_>) {
            ctx.insert(self, entity);
        }
    }

    #[derive(Component)]
    struct Plumbing;

    #[test]
    fn lists_editable_and_scene_components_and_hides_plumbing() {
        let mut world = World::default();
        world.register_component_type::<Tag>();
        let mut registry = InspectorRegistry::default();
        registry.register_component::<Transform>();
        world.insert_resource(registry);
        let entity = world.spawn((Transform::IDENTITY, Tag, Plumbing));

        let components = inspect_components(&world, entity);

        let names: Vec<_> = components.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["Tag", "Transform"]);
        assert!(components[0].properties.is_none(), "Tag is not editable");
        assert_eq!(components[1].properties.as_ref().map(Vec::len), Some(3));
    }

    #[test]
    fn the_shape_changes_when_the_properties_do() {
        let mut world = World::default();
        let mut registry = InspectorRegistry::default();
        registry.register_component::<Transform>();
        world.insert_resource(registry);
        let entity = world.spawn((Transform::IDENTITY,));

        let with_rows = inspect_components(&world, entity);
        let mut without_rows = inspect_components(&world, entity);
        without_rows[0].properties = None;

        assert_ne!(shape_of(entity, &with_rows), shape_of(entity, &without_rows));
    }

    #[test]
    fn labels_capitalise_the_field_name() {
        assert_eq!(label_for(&PropertyPath::new(["translation"])), "Translation");
        assert_eq!(label_for(&PropertyPath::new(["inner", "weight"])), "Weight");
    }
}
```

- [ ] **Step 2: Run and confirm failure**

Run: `cargo test -p editor inspector::tests`
Expected: compile errors, `cannot find function inspect_components` / `shape_of` / `label_for`.

- [ ] **Step 3: Delete the formatter path**

In `mod.rs` delete: `ComponentFormatter`, `FormatterRegistry` and its `impl`, `InspectedApp` and its `impl`, `number`, `vector`, `field`, `euler_degrees`, `format_transform`, `format_camera`, `format_light`, `format_generic`, the `ComponentBody` struct, and the `use serde_json::Value;` import.

- [ ] **Step 4: Replace the data types**

Replace `InspectedComponent` and `InspectorData` with:

```rust
/// One component on the inspected entity.
pub struct InspectedComponent {
    /// Short name, for display.
    pub name: String,
    pub type_id: TypeId,
    /// `None` for a component that is listed but not editable.
    pub properties: Option<Vec<Property>>,
}

#[derive(Resource, Default)]
pub struct InspectorData {
    pub heading: String,
    pub entity: Option<Entity>,
    pub components: Vec<InspectedComponent>,
    /// Set when the selected entity is a scene root, so it can be closed.
    pub closable_scene: Option<Entity>,
    /// The selection revision this snapshot was taken at.
    revision: Option<u64>,
    /// Hash of what the cards and rows are built from, as opposed to the values
    /// they show.
    shape: u64,
}
```

Update the imports at the top of `mod.rs`: add `use std::any::TypeId;`, `use std::hash::{Hash, Hasher};`, `use editable::{Property, PropertyPath};`, and `use numeric::{cancel_numeric_fields, commit_numeric_fields, refresh_numeric_fields, NUMERIC_FIELDS};`, `use registry::apply_property_commits;`. Keep the existing `ui` imports that remain in use.

- [ ] **Step 5: Add the collection helpers**

Below `InspectorData`:

```rust
const PROPERTY_LABEL_WIDTH: f32 = 72.0;

/// Editable components with their properties, then scene components by name.
/// Anything else (engine plumbing such as `GlobalTransform`) is not listed.
fn inspect_components(world: &World, entity: Entity) -> Vec<InspectedComponent> {
    let registry = world.get_resource::<InspectorRegistry>();
    let mut components: Vec<_> = world
        .component_ids(entity)
        .iter()
        .filter_map(|&type_id| {
            if let Some(editable) = registry.and_then(|registry| registry.component(type_id)) {
                return Some(InspectedComponent {
                    name: editable.name.to_string(),
                    type_id,
                    properties: (editable.collect)(world, entity),
                });
            }
            world.type_info(type_id).map(|info| InspectedComponent {
                name: info.short().to_string(),
                type_id,
                properties: None,
            })
        })
        .collect();
    components.sort_by(|a, b| a.name.cmp(&b.name));
    components
}

fn shape_of(entity: Entity, components: &[InspectedComponent]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    entity.hash(&mut hasher);
    for component in components {
        component.type_id.hash(&mut hasher);
        component.properties.is_some().hash(&mut hasher);
        for property in component.properties.iter().flatten() {
            property.path.hash(&mut hasher);
            property.type_id.hash(&mut hasher);
        }
    }
    hasher.finish()
}

fn label_for(path: &PropertyPath) -> String {
    let mut chars = path.name().chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}
```

- [ ] **Step 6: Rewrite `collect_inspector_data`'s component section**

In `collect_inspector_data`, replace everything from `let types: Vec<_> = world` through the end of the `data.shape = { ... };` block with:

```rust
            data.entity = Some(entity);
            data.components = inspect_components(world, entity);
            data.shape = shape_of(entity, &data.components);
```

The heading / children / scene-root code after it stays unchanged.

- [ ] **Step 7: Build rows in `rebuild_components`**

Add `registry: Res<InspectorRegistry>,` to `rebuild_components`' parameters (after `theme`). Replace the final `let body = cmd.spawn((... ComponentBody { index },)).entity(); cmd.add_child(card, body);` block inside the per-component loop with:

```rust
        let (Some(entity), Some(properties)) = (data.entity, &component.properties) else {
            continue;
        };
        let body = cmd
            .spawn(UINode {
                flex_shrink: 0.0,
                flex_direction: FlexDirection::Column,
                gap: glam::Vec2::new(0.0, theme.spacing_xs),
                padding: UIRect {
                    top: theme.spacing_xs,
                    ..Default::default()
                },
                ..Default::default()
            })
            .entity();
        cmd.add_child(card, body);

        for property in properties {
            let row = cmd
                .spawn((
                    UINode {
                        flex_shrink: 0.0,
                        flex_direction: FlexDirection::Row,
                        align_items: Some(taffy::AlignItems::Center),
                        gap: glam::Vec2::new(theme.spacing_xs, 0.0),
                        ..Default::default()
                    },
                    PropertyRow {
                        entity,
                        component: component.type_id,
                        path: property.path.clone(),
                    },
                    PropertyRowValue(property.value.clone()),
                ))
                .entity();
            cmd.add_child(body, row);

            let label = cmd
                .spawn((
                    UINode {
                        width: UIValue::Px(PROPERTY_LABEL_WIDTH),
                        flex_shrink: 0.0,
                        ..Default::default()
                    },
                    TextComponent {
                        color: theme.text_muted,
                        font_size: theme.font_size_sm,
                        line_height: theme.line_height(theme.font_size_sm),
                        wrap: false,
                        ellipsis: true,
                        ..text(&theme, &label_for(&property.path))
                    },
                ))
                .entity();
            cmd.add_child(row, label);

            registry
                .widget(property.type_id)
                .unwrap_or(default_widget(&property.value))
                .build(&mut cmd, row, &property.value, &theme);
        }
```

`index` from `enumerate()` is now unused: change the loop header to `for component in data.components.iter() {`.

Below `rebuild_components` add:

```rust
/// Exhaustive on purpose: a new `EditorValue` variant must choose a default.
fn default_widget(value: &EditorValue) -> &'static dyn PropertyWidget {
    match value {
        EditorValue::Number(_) | EditorValue::Vec3(_) => &NUMERIC_FIELDS,
    }
}
```

and add `EditorValue` to the `editable` import.

- [ ] **Step 8: Refresh row values; trim `refresh_inspector`**

In `refresh_inspector` remove the `registry` and `bodies` parameters and the `for (body, mut component) in bodies.iter()` loop. Add:

```rust
fn refresh_property_rows(
    data: Res<InspectorData>,
    rows: Query<(&PropertyRow, &mut PropertyRowValue)>,
) {
    for (row, mut value) in rows.iter() {
        let current = data
            .components
            .iter()
            .filter(|component| component.type_id == row.component)
            .flat_map(|component| component.properties.iter().flatten())
            .find(|property| property.path == row.path);
        if let Some(property) = current {
            if value.0 != property.value {
                value.0 = property.value.clone();
            }
        }
    }
}
```

- [ ] **Step 9: Wire the plugin**

Replace `InspectorPlugin::build` with:

```rust
impl Plugin for InspectorPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(InspectorData::default());
        app.insert_resource(InspectorScroll::default());
        app.insert_resource(InspectorRegistry::default());
        app.insert_resource(InspectorShape::default());
        app.insert_resource(PropertyCommits::default());
        app.register_editable::<Transform>();
        app.add_panel(PanelDescriptor {
            id: PANEL_ID,
            title: "Looking Glass",
            region: Region::Side,
        });
        app.add_system(Startup, build_panel);
        // Registration order is execution order. Commits must be applied before
        // rows are rebuilt, because a rebuild despawns the fields that produced them.
        app.add_system(LateUpdate, commit_numeric_fields)
            .add_system(LateUpdate, cancel_numeric_fields)
            .add_system(LateUpdate, apply_property_commits)
            .add_system(LateUpdate, collect_inspector_data)
            .add_system(LateUpdate, rebuild_components)
            .add_system(LateUpdate, refresh_inspector)
            .add_system(LateUpdate, refresh_property_rows)
            .add_system(LateUpdate, refresh_numeric_fields)
            .add_system(LateUpdate, sync_inspector_scroll);
    }
}
```

Add `use essential::transform::Transform;` and `use registry::EditableApp;` (already re-exported; import it for the call).

- [ ] **Step 10: Run the tests, the whole editor suite, and the build**

Run: `cargo test -p editor && cargo build -p editor`
Expected: all editor tests PASS (including `inspector::tests`, `inspector::registry`, `inspector::numeric`), no errors. Fix any warnings introduced by these tasks (unused imports, dead code).

- [ ] **Step 11: Run the wider test suites touched by the plan**

Run: `cargo test -p ecs -p editable -p essential -p ui -p editor`
Expected: PASS.

- [ ] **Step 12: Manual verification**

Run: `cargo run -p editor -- --project <a project directory with a scene>` (ask the user for the project path if unknown). Open a scene, select a mesh entity, and check:

1. The Transform card shows Translation, Rotation, Scale rows with three fields each; other scene components show a name-only card.
2. Typing a new X translation and pressing Enter moves the object in the viewport.
3. Typing a rotation (e.g. `45` in Y) rotates it; the field keeps what you typed while focused.
4. Tab commits and moves to the next field; tabbing across an untouched field changes nothing.
5. Escape reverts the field to the current value.
6. Typing `abc` and pressing Tab reverts the field; typing `nan` logs a `rejected` warning and the value is unchanged.
7. Typing into a field and then clicking a different entity in the hierarchy applies the edit to the first entity.

Report any failure with what was observed.

- [ ] **Step 13: Stage**

```bash
git add crates/editor/src/inspector/mod.rs
```
