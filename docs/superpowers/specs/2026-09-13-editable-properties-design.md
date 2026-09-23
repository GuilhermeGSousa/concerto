# Editable Component Properties — Design

**Status:** approved, ready for an implementation plan
**Branch:** `editor`
**Scope:** `Transform` only; the mechanism is built to take more components and
richer leaf types (e.g. `AssetId`) later without changing its interfaces.

## Problem

The inspector (`crates/editor/src/inspector.rs`) is read-only. It serializes each
component to `serde_json::Value` through `TypeInfo::read` and renders the whole
value as one text blob through a string-keyed `ComponentFormatter`. There is no
way to change a value, and there is no field-level model to hang an editing
widget off: a formatter produces a `String`, not rows.

## Goals

- A user can edit `Transform`'s translation, rotation and scale in the inspector,
  and the change applies to the live world immediately.
- A type opts in to editing by implementing an `Editable` trait. A derive macro
  implements it for any struct whose fields all implement it.
- Crates outside the editor can define their own leaf types and their own
  widgets for them.
- Registration is keyed by `TypeId`, never by type-name strings.

## Non-goals

- Persisting edits. Edits mutate the live `World` only; nothing is written back
  to the scene file, and reopening a scene discards them.
- Undo/redo.
- Drag-to-scrub number fields. Numbers are edited as text.
- Editing `Camera`, `Light` or any component other than `Transform`. Non-editable
  components show a card with their name and no body.
- Derive attributes (`skip`, `label`), nested-struct group headers, collections,
  enums, and `EditorValue` variants beyond what `Transform` needs.

## Architecture

```
editable (new)          trait, EditorValue, visitors, collect/apply, leaf impls
editable/macros (new)   #[derive(Editable)]
ecs                     + World::component_ids, World::type_info
ui                      + UITextInputSubmitted / UITextInputCancelled
essential               Transform: #[derive(Editable)]
editor                  registry, rows, PropertyWidget, NumericFields, commit flow
```

`editable` depends only on `glam` and its macro crate. `ecs` does not depend on
`editable`. `essential` and `editor` do.

## 1. The `editable` crate

New crate at `crates/editable`, edition 2024.

```rust
/// The value of one leaf in the form the editor shows and edits, which is not
/// necessarily its Rust form: a `Quat` is presented as euler degrees.
pub enum EditorValue {
    Number(f64),
    Vec3([f64; 3]),
}

/// A type the editor can show and change. A leaf answers `read`/`write`; a
/// struct answers `visit`/`visit_mut`, usually through `#[derive(Editable)]`.
pub trait Editable: Any {
    /// `Some` for a leaf, `None` for a struct.
    fn read(&self) -> Option<EditorValue> { None }
    /// Returns `false` when `value` has the wrong shape or is not finite; the
    /// leaf is then left unchanged.
    fn write(&mut self, _value: &EditorValue) -> bool { false }
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

### Why one trait

Whether a type is a leaf is an answer given at runtime (`read` returning `Some`),
not a separate trait. The derive cannot inspect field types, so with separate
leaf and struct traits it could not choose between calling a leaf method and a
nested one; a blanket impl bridging the two traits conflicts with the derive's
impls under coherence. With one trait, every field is passed to the visitor
the same way.

Being a leaf is a presentation decision the impl makes. `Vec3` has fields but is
a leaf because it is edited as one row of three boxes. `Quat` is a leaf that
presents as euler degrees.

### Why `visit` and `visit_mut` are separate

The display pass runs every frame. Mutable component access stamps the changed
tick (`World::get_component_for_entity_mut`), so a display pass through `&mut`
would make the selected entity report `Changed<Transform>` every frame. Display
goes through `visit` (`&self`); only a commit goes through `visit_mut`.

### Why `Editable: Any`

A leaf's widget is looked up by the leaf's concrete `TypeId`, obtained by
upcasting `&dyn Editable` to `&dyn Any` (stable since Rust 1.86; the toolchain is
1.96). The upcast must be explicit — `(value as &dyn Any).type_id()` — because
calling `.type_id()` on a `&&dyn Editable` yields the reference's own `TypeId`.

### Leaf impls

In `editable` (which owns the trait, so glam impls satisfy the orphan rule):

| Type | `read` | `write` |
|---|---|---|
| `f32`, `f64` | `Number` | from `Number` |
| `glam::Vec3` | `Vec3` | from `Vec3` |
| `glam::Quat` | `Vec3` of euler degrees, `EulerRot::XYZ` | from `Vec3` degrees, normalized |

Every `write` rejects non-finite components. `"nan".parse::<f64>()` succeeds, so
without this a typed `nan` would reach a transform.

### `EditorValue`

It is an owned, presentation-form snapshot of one leaf. It exists, rather than
handing widgets `&dyn Any` to the leaf, because:

1. It outlives the world borrow: rows are collected in an exclusive system,
   stored in a resource, and read by UI systems later; commits are queued and
   applied later still.
2. It lets the default widget render leaves whose types it has never seen, by
   matching on the value's shape.
3. It is the only thing a widget reads and produces, so widgets cannot bypass a
   leaf's `write` or the change-tick path.

It is deliberately closed (no `Custom(Box<dyn Any>)`), because a payload only its
own widget can read would break the default-widget guarantee. Variants are added
when a component needs them; `AssetId` support is expected to add an
`Asset(AssetId)` variant.

### Paths, `collect` and `apply`

```rust
/// Where a leaf sits inside its component: `["translation"]`, or
/// `["bar", "weight"]` for a nested struct field.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PropertyPath(Vec<&'static str>);

pub struct Property {
    pub path: PropertyPath,
    /// The leaf's concrete type, which widgets are registered against.
    pub type_id: TypeId,
    pub value: EditorValue,
}

pub fn collect(root: &dyn Editable) -> Vec<Property>;
pub fn apply(
    root: &mut dyn Editable,
    path: &PropertyPath,
    value: &EditorValue,
) -> Result<(), ApplyError>;

pub enum ApplyError {
    /// No leaf at that path.
    NotFound,
    /// The leaf refused the value: wrong shape or not finite.
    Rejected,
}
```

Both functions are implemented with private visitor structs:

- **Collect** pushes the field name onto its path, then either records a
  `Property` (when `read` returns `Some`) or recurses with `visit`, then pops.
  Properties come out in field order.
- **Apply** carries the target path and a depth. At each field it returns
  immediately unless the name matches the segment at the current depth; on a
  match it descends. It returns `NotFound` if the path ends on a struct or has
  segments left past a leaf, and stops after the first write.

Segments are `&'static str` because the derive emits field names as literals.
The `PropertyPath` newtype keeps a future index segment (for collections) an
internal change.

## 2. `#[derive(Editable)]`

New crate at `crates/editable/macros`, mirroring `crates/ecs/macros`, re-exported
from `editable`.

- Supported on structs with named fields. Tuple structs and enums produce a
  `compile_error!` naming the limitation.
- Emits `visit` and `visit_mut`, each calling `visitor.field("name", &self.name)`
  (or `&mut self.name`) once per field, in declaration order. `read`/`write` keep
  their defaults.
- A field whose type does not implement `Editable` fails to compile at that
  field.
- No attributes.

Generated code for `Transform`:

```rust
impl Editable for Transform {
    fn visit(&self, v: &mut dyn PropertyVisitor) {
        v.field("translation", &self.translation);
        v.field("rotation", &self.rotation);
        v.field("scale", &self.scale);
    }
    fn visit_mut(&mut self, v: &mut dyn PropertyVisitorMut) {
        v.field("translation", &mut self.translation);
        v.field("rotation", &mut self.rotation);
        v.field("scale", &mut self.scale);
    }
}
```

## 3. Changes to existing crates

**`ecs`:** add

```rust
/// Every component `entity` carries, including ones that are not scene
/// components. Empty for a stale entity.
pub fn component_ids(&self, entity: Entity) -> &[ComponentId];

/// The read side of a registered scene component, or `None` if `id` is not one.
pub fn type_info(&self, id: ComponentId) -> Option<&TypeInfo>;
```

`ComponentId` is already an alias for `TypeId`, and archetypes already store the
list; this exposes it read-only. `type_info` exposes the existing
`ComponentRegistry::type_info` lookup. `World::component_types` is unchanged.

**`essential`:** depend on `editable`; add `#[derive(Editable)]` to `Transform`.

**`ui`:** `UITextInput` gains two events, registered by the UI plugin:

```rust
/// Enter was pressed while the field had focus.
#[derive(Event)]
pub struct UITextInputSubmitted { pub entity: Entity, pub value: String }

/// Escape was pressed while the field had focus.
#[derive(Event)]
pub struct UITextInputCancelled { pub entity: Entity }
```

The decision (key states → submitted / cancelled / neither) is a pure function
so it can be unit-tested.

## 4. Editor: registry

`InspectorRegistry` is rewritten, keyed by `TypeId`:

```rust
#[derive(Resource, Default)]
pub struct InspectorRegistry {
    components: HashMap<TypeId, EditableComponent>,
    widgets: HashMap<TypeId, Box<dyn PropertyWidget>>,
}

struct EditableComponent {
    /// Last segment of `std::any::type_name::<T>()`. Display only.
    name: &'static str,
    collect: fn(&World, Entity) -> Option<Vec<Property>>,
    apply: fn(&mut World, Entity, &PropertyPath, &EditorValue)
        -> Option<Result<(), ApplyError>>,
}
```

`collect` and `apply` are monomorphized per registered type:

```rust
fn collect_typed<T: Component + Editable>(world: &World, entity: Entity) -> Option<Vec<Property>> {
    world.get_component_for_entity::<T>(entity).map(|c| editable::collect(c))
}
```

The outer `Option` means the entity does not carry the component.

Registration is an `App` extension trait, replacing `InspectedApp`:

```rust
pub trait EditableApp {
    /// Makes a component's properties editable in the inspector.
    fn register_editable<T: Component + Editable>(&mut self) -> &mut Self;
    /// Replaces the default widget for leaves of type `T`.
    fn register_property_widget<T: Editable>(&mut self, widget: Box<dyn PropertyWidget>) -> &mut Self;
}
```

Registering the same type twice overwrites the entry. `InspectorPlugin` calls
`register_editable::<Transform>()`.

Known imprecision: `apply` takes the mutable borrow before the leaf decides, so a
`Rejected` write still stamps the changed tick. The only cost is recomputing a
matrix that comes out identical, so this is accepted rather than avoided with a
second walk or a clone.

## 5. Editor: rows and widgets

### Cards

A component gets a card if it is registered as editable or is a scene component
(has a `TypeInfo`); anything else is not listed, matching today's panel. Names
come from the registry for editable components and from `TypeInfo::short()` for
the rest. A card for a non-editable component has no body. An editable
component's body is a column of rows, one per `Property`: a fixed-width label
(the last path segment with its first letter capitalized) followed by the
widget's UI. The header's placeholder toggle is unchanged.

### The row contract

At rebuild, each row entity is spawned with:

```rust
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

/// Commits waiting for `apply_property_commits`.
#[derive(Resource, Default)]
pub struct PropertyCommits(pub Vec<PropertyCommit>);

pub trait PropertyWidget: Send + Sync + 'static {
    /// Spawns this widget's UI under `row` for a property currently holding
    /// `value`. The widget's own systems read the row's `PropertyRowValue` and
    /// push to `PropertyCommits`.
    fn build(&self, cmd: &mut CommandQueue, row: Entity, value: &EditorValue, theme: &UITheme);
}
```

Commits are a resource queue rather than an event because the applying system
is exclusive (`&mut World`) and cannot hold an `EventReader` cursor; the editor
already queues work this way (`SceneCommands`, `EditorCommands`). `build`
receives the value so a widget can size itself (one field or three).

A widget is a stateless builder: one instance serves every row of its type.
Per-row state lives on the row's entities. A widget that needs behavior (a
future asset picker) adds its own systems in its own plugin.

### Widget selection

For each `Property`, the widget registered for `property.type_id` is used. On a
miss, the default is chosen by an exhaustive `match` on the `EditorValue`
variant. Both current variants map to `NumericFields`.

### `NumericFields`

The default widget, implemented as a `PropertyWidget`. It spawns one
`UITextInput` for `Number` or three in a row for `Vec3`, each tagged
`NumericSlot { row: Entity, slot: usize, displayed: String }` — its row entity,
its index, and the text it last displayed. Enter commits and keeps focus; the
committed text becomes `displayed`, so the focus loss that follows does not
commit it again.

- **Refresh:** writes each slot's formatted value into its `UITextInput`, except
  the focused field. The focused field's `UITextInput::value` is the edit buffer;
  skipping it is what keeps the euler presentation (not an identity round-trip)
  from overwriting typing.
- **Commit** on `UITextInputSubmitted` or `UIFocusLost`, through a pure function
  of (current value, slot, text, displayed text):
  - text equal to the displayed text → no commit, so display rounding never
    truncates the stored value;
  - text that does not parse as `f64` → no commit; the field reverts to the
    displayed text;
  - otherwise → a copy of the row's value with that slot replaced, pushed as a
    `PropertyCommit`.
- **Cancel** on `UITextInputCancelled`: revert the text to the displayed text and
  clear focus.

## 6. Editor: data flow and ordering

System order:

- `Update`: `apply_property_commits` — exclusive; for each `PropertyCommit`,
  calls the registry's `apply` for `row.component` on `row.entity`. It runs in
  `Update` so an edit is stamped before `TransformPlugin`'s `LateUpdate`
  propagation, whose `update_simple_entities` only sees `Changed<Transform>`
  within the same frame.
- `LateUpdate`, in order:
  1. `select_numeric_field_on_focus`, `NumericFields` commit/cancel systems (and
     any registered widget's systems) — commits pushed here apply next frame
  2. `collect_inspector_data` — exclusive; for each id in
     `World::component_ids(selected)`, calls the registry's `collect` if
     registered; otherwise, if the id has a `TypeInfo` (a scene component),
     records its name only; otherwise skips it, so engine plumbing such as
     `GlobalTransform` stays hidden as it is today
  3. `rebuild_components` — respawns cards and rows when the shape changes
  4. refresh systems — update `PropertyRowValue` and the fields

A numeric field selects its whole text when it gains focus, so typing replaces
the displayed value rather than appending to it.

The shape hash that gates rebuilds covers the entity, the component `TypeId`s,
each property path, and each leaf `TypeId`.

Systems run in registration order (no `.after()`/`.before()`: the scheduler
re-registers their arguments as duplicate systems). The editor's plugins are
registered before `UIPlugin`, so UI events (`UIClick`, `UIFocusLost`,
`UITextInputSubmitted`) written in frame N are read by the editor in frame N+1;
events are double-buffered, so none are lost.

Ordering requirement: a field being edited when the user clicks another entity
loses focus in the same frame the click is reported, and both are handled by the
editor in the following frame. The commit captures its `PropertyRow` when it is
pushed, before the rebuild despawns the row, so the edit lands on the
previously selected entity even though it is applied a frame later.

## 7. Error handling

| Failure | Caught in | Result |
|---|---|---|
| Text does not parse | `NumericFields` commit | No commit; field reverts |
| Text unchanged from display | `NumericFields` commit | No commit |
| Non-finite or wrong-shape value | leaf `write` → `Rejected` | `log::warn!`; component unchanged; refresh shows the real value |
| Path does not lead to a leaf | `apply` → `NotFound` | `log::warn!` (indicates a bug) |
| Entity or component gone before apply | registry `apply` → `None` | `log::debug!`; commit dropped |
| Field type not `Editable` | derive | compile error |
| No widget for a leaf | fallback by shape | exhaustive match; cannot miss at runtime |

## 8. Removed

From `inspector.rs`: `ComponentFormatter`, the string-keyed `InspectorRegistry`,
`InspectedApp`, `format_generic`, `format_transform`, `format_camera`,
`format_light`, their helpers and tests, and `InspectedComponent::value`.

## 9. Testing

Inline `#[cfg(test)]` unit tests on pure functions and `World`-level tests where
ECS behavior is the point. No new test dependencies.

- **`editable`:** read/write round-trips for `f32`, `f64`, `Vec3`; `Quat` through
  euler degrees, compared as rotations (`q` ≡ `-q`); non-finite and wrong-shape
  writes rejected with the value unchanged; `collect` on a derived struct
  containing a derived struct yields the right paths, leaf `TypeId`s and order;
  `apply` returns `NotFound` for a path ending on a struct and for segments past
  a leaf.
- **`editable/macros`:** `compile_fail` doctests for a tuple struct, an enum, and
  a non-`Editable` field.
- **`ecs`:** `component_ids` includes non-scene components; empty for a stale
  entity.
- **`ui`:** the Enter/Escape decision function.
- **`editor`:**
  - the `NumericFields` commit function: parse failure, unchanged text, slot
    replacement in a `Vec3`;
  - `World`-level: with `Transform` registered, a commit captured for entity A is
    applied after the selection moved to B — A changes and reports `Changed`, B
    does not;
  - `World`-level: running the collect pass does not mark `Transform` changed.
- **Manual:** open a scene in the editor; edit position, rotation and scale and
  see the object move in the viewport; confirm Enter commits, Escape reverts,
  Tab commits, and tabbing across an untouched field changes nothing.
