# Scene Dirty Tracking and Save — Design

**Status:** implemented
**Branch:** `functional-editor`
**Builds on:** `2026-09-08-wonderland-editor-design.md` (which deferred editing
and saving) and `2026-09-13-editable-properties-design.md`

## Problem

The editor can change a scene but cannot keep the change. Inspector edits land
on live entities and are lost when the scene is replaced, the tab is closed,
the project is switched, or the window is closed. None of those paths warn.
There is no way to turn the live entities back into a `Scene`, no save command,
and nothing that records whether a document differs from what is on disk.

## Goals

- A document knows whether it has unsaved changes, and the tab shows it.
- Ctrl+S writes the active scene to disk and clears the dirty state.
- Unsaved changes are never discarded without the user choosing to.
- Saving an imported scene cannot be undone by re-running `import`.

## Non-goals

- **Multiple scene tabs.** There is one Scene tab. Opening another scene
  replaces it, as today. Per-asset tabs and per-tab worlds come later.
- **New ways to edit.** Entity create, delete, duplicate, rename and reparent,
  viewport picking, transform gizmos and asset placement are separate work.
  Anything they add only has to call the dirty hook defined here.
- **Undo/redo.**
- **Recomputing asset references.** No edit available today can add a
  reference, so a saved scene carries the list it was loaded with.

## Decisions

| Decision | Chosen | Because |
| --- | --- | --- |
| How dirtiness is detected | A revision counter bumped where edits are applied | Engine systems also write components, and `Changed<T>` is frame-exact, so change ticks cannot tell a user edit from the engine |
| Where a saved import goes | An authored copy beside it | `import` rewrites the outputs it owns; a copy without provenance is out of its reach |
| Entity references on capture | A scoped entity-to-index map consulted by `SceneEntityRef` | Keeps `SceneComponent` unchanged; a resolved reference outside a capture still refuses to serialize |
| A component that cannot be read back | Capture fails | A save that silently drops data is worse than one that reports an error |
| Registry on save | The editor persists its own complete registry | `save_content_asset` upserts into the file on disk; where no file exists it would write a one-entry registry the runtime then trusts |

## Section 1 — Dirty state

`EditorDocument` gains two counters, `revision` and `saved_revision`, with
`mark_edited()`, `mark_saved(revision)` and `is_dirty()`. A document is dirty
when the counters differ. Both reset when a load or replacement succeeds.

`mark_saved` takes the revision that was captured, not the current one, so an
edit that lands between capture and write leaves the document dirty.

Edits reach the world through two exclusive systems in the inspector,
`apply_property_commits` and `apply_component_edits`. After each edit that
applies successfully they call one helper:

```rust
pub fn mark_entity_edited(world: &mut World, entity: Entity)
```

It walks `ChildOf` upward from `entity` to the first ancestor carrying
`EditorOwned(document)` and bumps that document's revision. An entity with no
owning document, such as an editor helper, is ignored. Every future edit path
calls the same helper. A custom asset editor that does not edit entities calls
`mark_edited()` on its own document.

The tab label shows a dot before the title while the document is dirty.

The `SceneRoot` entity is the editor's wrapper around a scene, not a node in
it, so nothing on it can be saved. The inspector shows no component cards when
it is selected.

## Section 2 — Capturing a scene from the world

A new function in `concerto-scene`, the inverse of `spawn_scene`:

```rust
pub fn capture_scene(world: &World, root: Entity) -> anyhow::Result<Scene>
```

- **Nodes.** Every descendant of `root`, depth-first, children in `Children`
  order. `root` itself is not a node; its children are the scene's roots.
- **Names.** From `Name`, or empty when the entity has none.
- **Components.** Each `TypeInfo` from `World::component_types`, read with
  `TypeInfo::to_json`, written under its full type name and sorted by that name
  so two captures of the same world are identical. `to_json` is new: `read`
  goes through `serde_json::Value`, which widens `f32` to `f64` and would write
  `0.1` as `0.10000000149011612`.
- **Failure.** A listed component whose `to_json` returns `None` fails the
  capture with the entity's name and the component's type.
- **References.** `referenced_assets` is left empty; the caller fills it.

`SceneEntityRef::Entity` refuses to serialize today, which would fail the
capture for any skinned mesh. `concerto-ecs` gains a scoped map:

```rust
pub fn with_entity_indices<R>(map: &HashMap<Entity, usize>, f: impl FnOnce() -> R) -> R
```

While `f` runs on the calling thread, a resolved reference serializes as the
index the map gives it. Outside a scope, or for an entity the map does not
contain, serialization fails as it does now. `capture_scene` builds the map
from its node order and reads every component inside the scope, so a reference
to an entity outside the captured tree fails the capture.

## Section 3 — Saving

**Command.** A `Save` action bound globally to Ctrl+S. It adds a
`SaveRequested` marker to the active document if that document is dirty and not
loading. The editor that owns the document handles the marker and removes it.
This is the extension point for custom asset editors.

**Scene save.** An exclusive system handles `SaveRequested` on scene documents:

1. Capture the scene from its `SceneRoot`, recording the document's revision.
2. Set `referenced_assets` to the list the scene was loaded with, which
   `SceneEditorState` now retains.
3. Choose the address (below) and write the content asset.
4. Insert the asset into the project's registry and catalogue, write the
   registry file, and publish the registry to the asset server. The project
   generation does not change, so open documents stay valid.
5. Call `mark_saved` with the recorded revision and set the status line to
   `Saved <address>`.

Any failure leaves the document dirty, writes nothing further, and sets the
status line to `Save failed: <error>`.

**Address.** A scene with import provenance is never overwritten. Its first
save writes `<folder>/<name>-level.<extension>`, with `-2`, `-3` and so on
appended while that address is taken. The new asset has a freshly minted id
and no provenance. The document and its `SceneRoot` then rebind to the new
asset, so the tab title changes and later saves overwrite the copy in place,
reusing its id. The imported scene stays in the catalogue untouched.

**Catalogue.** `discover_project` currently skips every asset without
provenance, which would hide the copy just written. It lists them instead, and
`AssetEntry::provenance` becomes `Option<ImportProvenance>`.

**Foundation.** Writing the asset file is split out of `save_content_asset`
into a function that takes the registry to update and returns the asset id.
`save_content_asset` keeps its current behaviour by calling it.

## Section 4 — Guarding unsaved changes

Four actions destroy a document's live state: replacing the scene, closing a
tab, switching project, and closing the window. Each is checked where it is
requested. When it would destroy a dirty document, it is held as a pending
intent and a prompt opens instead.

A project switch is checked when it is requested, before discovery starts. The
close-all it issues once the new project is loaded is therefore not checked
again. An edit made while a replacement scene is still loading cancels the
replacement, so the load cannot overwrite it.

The prompt is a full-window scrim that blocks input to the rest of the editor,
with a centred dialog naming the asset (or the count, when several are dirty)
and three buttons:

- **Save** requests a save on every affected document. When all are clean the
  intent runs. If any save fails, the intent is dropped and the prompt closes
  with the error in the status line.
- **Discard** marks the affected documents saved at their current revision and
  runs the intent.
- **Cancel** drops the intent.

Escape cancels. While the prompt is open, further guarded actions are ignored.

**Window close.** The window crate exits as soon as the OS requests a close.
It gains an `InterceptClose` resource, off by default. When on, the request is
delivered as the usual `WindowEvent` and the application exits by setting the
existing `CloseRequest`. The editor turns it on and routes both the OS request
and its own title-bar close button through the guard.

## Files

| File | Change |
| --- | --- |
| `crates/ecs/src/component/scene.rs` | `with_entity_indices`; `SceneEntityRef` serialization consults it |
| `crates/ecs/src/component/registry.rs` | `TypeInfo::to_json` |
| `crates/scene/src/capture.rs` | New: `capture_scene` |
| `crates/foundation/src/assets/content.rs` | Split the file write out of `save_content_asset` |
| `crates/window/src/lib.rs`, `plugin.rs` | `InterceptClose` |
| `crates/editor/src/asset_editor.rs` | Revision counters, `mark_entity_edited`, `SaveRequested`, guard checks on replace and close |
| `crates/editor/src/guard.rs` | New: pending intent, prompt UI, resolution |
| `crates/editor/src/scene.rs` | Retain `referenced_assets`; save system; rebind after first save |
| `crates/editor/src/project.rs` | Optional provenance; list authored assets; catalogue insert; guard on project switch |
| `crates/editor/src/inspector/` | Call `mark_entity_edited`; no cards for `SceneRoot` |
| `crates/editor/src/tabs.rs` | Dirty dot |
| `crates/editor/src/actions.rs` | `Save` action and binding |
| `crates/editor/src/window_chrome.rs` | Close button goes through the guard |
| `crates/editor/README.md` | Edits are no longer temporary; document save and the guard |

## Testing

Unit tests, headless, in the style already used in each crate:

- **ecs:** a resolved `SceneEntityRef` serializes to its index inside a scope,
  fails outside one, and fails for an entity missing from the map.
- **scene:** spawn a scene, capture it, and compare node names, hierarchy and
  component payloads; a scene with a `SkeletonComponent` round-trips; an
  unreadable component fails the capture; two captures are identical.
- **foundation:** the split write returns the id and updates the registry it
  is given; `save_content_asset` behaves as before.
- **editor, dirty:** a property commit and a component add each mark the
  owning document; an edit to an unowned entity marks nothing; a successful
  load resets the counters; an edit between capture and `mark_saved` leaves the
  document dirty.
- **editor, save:** saving an imported scene writes a copy with no provenance
  and a new id, leaves the import byte-identical, rebinds the document, and the
  copy appears in the catalogue; a second save reuses the id and address; a
  taken address gets a numeric suffix; a failed write leaves the document dirty.
- **editor, guard:** replacing, closing, close-all and quit are each held when
  a document is dirty and run immediately when none is; Discard runs the
  intent; Cancel drops it; Save runs it only after the documents are clean.
- **editor, discovery:** authored assets are listed.

Then one pass in the running editor: edit a transform, see the dot, save, see
it clear, reopen the copy and confirm the edit, and confirm the prompt on tab
close and on window close.

## Deferred

- Per-asset tabs, then a world per tab.
- Entity operations, viewport picking, transform gizmos, asset placement.
- Recomputing `referenced_assets`, which asset placement will need.
- Choosing the name of the authored copy, and Save As.
- Undo/redo.
