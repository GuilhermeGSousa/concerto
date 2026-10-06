# Entity Operations — Design

**Status:** awaiting review
**Branch:** `functional-editor`
**Builds on:** `2026-10-06-scene-dirty-tracking-and-save-design.md`

## Problem

A scene's set of entities is fixed at whatever was imported. The editor can
change component values and add or remove components, but it cannot add an
entity, remove one, copy one, or name one. A level cannot be built from that.

## Goals

- Create an empty entity, delete an entity, duplicate an entity with its
  subtree, and rename an entity.
- Each is reachable from the hierarchy with the mouse and from the keyboard.
- Each marks the scene dirty and survives a save and reload.

## Non-goals

- **Reparenting.** Deferred.
- **Reordering siblings.** New and duplicated entities go last.
- **Multi-select.** Every operation acts on the one selected entity.
- **Undo/redo.** All four operations are applied by one listener so undo has
  a single point to hook into later, but nothing here records history.
- **Viewport interaction.** Picking and gizmos are the next two steps.

## Decisions

| Decision | Chosen | Because |
| --- | --- | --- |
| How operations are applied | An `EntityEdit` signal with one global listener that takes `&mut World` | Panels trigger it from their command queue and it is applied at that system's sync point, so there is no queue resource to drain and no schedule slot to choose; one listener is still the single place that mutates and marks the document |
| How duplicate copies | Capture the subtree to a `Scene`, spawn it beside the source | Reuses the save path, so every scene component and every entity reference inside the subtree is handled already |
| Where rename happens | A name field at the top of the inspector | Tree rows are pooled labels; an inline field would need a second widget tracking a scrolling row |
| What a new entity carries | `Name`, `Transform`, `SyncWithRenderWorld` | Without the last, a `Light` added afterwards through "add component" would never reach the renderer |
| The scene root | Cannot be deleted, duplicated or renamed | It is the editor's wrapper, not a node in the scene |

## Section 1 — The edit signal

A new module, `crates/editor/src/entity_ops.rs`:

```rust
pub enum EntityEdit {
    Create { parent: Entity },
    Delete(Entity),
    Duplicate(Entity),
    Rename { entity: Entity, name: String },
}

impl Signal for EntityEdit {}

fn apply_entity_edit(on: On<EntityEdit>, world: &mut World);
```

Panels submit an edit with `cmd.trigger(EntityEdit::Delete(entity))`. The
plugin registers `apply_entity_edit` once as a global listener. This is the
first global listener outside the ECS crate's own tests, so `App` gains an
`add_listener` that forwards to the main world.

For an edit that applies, the listener calls `mark_entity_edited` and, where
the edit names an entity to select, updates `Selection`. An edit that is
rejected changes nothing and writes `"<Operation> failed: <reason>"` to the
owning document's status, which the status line already shows.

The inspector's `ComponentEdits` and `PropertyCommits` stay queue resources in
this step. Moving them to signals is a separate change.

Every edit is checked against the same rule first: its target must be a live
entity inside a scene, meaning it is a `SceneRoot` or has one as an ancestor.

**Create.** `parent` may be the scene root. Spawns an entity named `Entity`
with an identity `Transform` and `SyncWithRenderWorld`, adds it as the last
child of `parent`, and selects it.

**Delete.** Rejected for the scene root. The document is marked through the
parent before the entity is despawned. `World::despawn` already removes
descendants. Selection moves to the parent.

**Duplicate.** Rejected for the scene root. The subtree rooted at the entity is
captured, spawned as the last child of the same parent, and the copy of the
entity is selected. The copy keeps the source's name. If the capture fails,
nothing is spawned: the usual cause is a component that refers to an entity
outside the subtree, such as a skinned mesh duplicated without its bones.

**Rename.** Rejected for the scene root and for a name that is empty after
trimming. Sets `Name`, inserting it when the entity has none.

## Section 2 — Scene crate additions

Duplicate needs two things `concerto-scene` does not have yet.

```rust
pub fn capture_subtree(world: &World, entity: Entity) -> anyhow::Result<Scene>
```

Like `capture_scene`, but `entity` itself is node 0. The two share their
implementation.

```rust
pub fn spawn_scene_in_world(world: &mut World, scene: &Scene, parent: Entity) -> SpawnedScene
```

`spawn_scene` takes a `CommandQueue`, which an exclusive system does not have.
This applies the same steps directly through `World`. The existing spawn test
is run against both.

## Section 3 — Hierarchy

**Add button.** A `+` in the panel header adds a child to the selected entity,
or to the scene root when nothing in the scene is selected. It is disabled when
no scene is open.

**Context menu.** Right-clicking a row selects it and opens a menu at the
pointer: Add child, Duplicate, Rename, Delete. For the scene root only Add
child is enabled. One menu entity is shared by all rows, as the inspector's
component menu is.

**Keyboard.** Three new actions:

| Action | Binding | Contexts |
| --- | --- | --- |
| `DeleteEntity` | Delete | tree, viewport |
| `DuplicateEntity` | Ctrl+D | tree, viewport |
| `RenameEntity` | F2 | tree |

Delete and duplicate are bound in the viewport context now so they are already
in place when picking lands. None of them fire while a text field has focus,
because focus outside the tree pops the tree context.

**Following the change.** When an edit selects an entity, the hierarchy expands
its ancestors and scrolls its row into view.

## Section 4 — Inspector name field

A single-line text field sits above the component stack and shows the selected
entity's `Name`. Enter or loss of focus triggers a `Rename` when the text
differs; Escape restores the current name. The field is hidden when nothing is
selected or the selection is the scene root.

`RenameEntity` and the menu's Rename item give the field keyboard focus with
its text selected.

## Files

| File | Change |
| --- | --- |
| `crates/scene/src/capture.rs` | `capture_subtree` |
| `crates/scene/src/spawner.rs` | `spawn_scene_in_world` |
| `crates/app/src/lib.rs` | `App::add_listener` |
| `crates/editor/src/entity_ops.rs` | New: the `EntityEdit` signal and its listener |
| `crates/editor/src/actions.rs` | Three actions and their bindings |
| `crates/editor/src/hierarchy.rs` | Add button, row context menu, keyboard handling, reveal on selection |
| `crates/editor/src/inspector/` | Name field |
| `crates/editor/src/lib.rs` | Register the listener |
| `crates/editor/README.md` | Document the operations and shortcuts |

## Testing

Unit tests, headless:

- **scene:** `capture_subtree` puts the entity at node 0 with its descendants
  after it; a subtree whose skeleton and bones are both inside round-trips with
  references remapped; a reference to an entity outside fails.
  `spawn_scene_in_world` produces the same entities, components and hierarchy
  as `spawn_scene`.
- **editor, create:** the new entity is the last child of the parent, carries
  the three components, is selected, and the document is dirty; the scene root
  is accepted as a parent; an entity outside any scene is rejected.
- **editor, delete:** the subtree is gone, selection is the parent, the
  document is dirty; the scene root is rejected and nothing changes.
- **editor, duplicate:** the copy has the same components and descendants, is
  the last child of the same parent and is selected; a subtree with an outside
  reference is rejected, nothing is spawned, and the status reports it.
- **editor, rename:** the name changes and the document is dirty; an empty
  name and the scene root are rejected.
- **editor, save:** a scene with a created, a duplicated and a renamed entity
  saves and loads back with all three.
- **editor, hierarchy:** right-clicking a row selects it and opens the menu;
  each menu item and each action triggers the matching edit; the root's
  restricted items trigger nothing.
- **editor, inspector:** submitting the name field triggers a rename; an
  unchanged name triggers nothing; Escape restores the text.

Then one pass in the running editor, driven in code as before: create, rename,
duplicate and delete in one session, save, reopen, and confirm the tree.
