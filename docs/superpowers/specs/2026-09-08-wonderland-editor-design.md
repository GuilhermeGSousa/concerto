# Wonderland Editor — Design

**Status:** approved, ready for an implementation plan
**Branch:** `editor`
**Replaces:** the editor built in `crates/editor` (kept working at every
milestone; this is a reshaping, not a rewrite from zero)

## Problem

The editor works, but three things about it fight the goal of a minimal,
ergonomic engine.

**It inspects a copy of the world, not the world.** `crates/editor/src/document.rs`
parses a scene file into a `SceneDocument` — nodes, `parents`, `roots`, an
`InstanceId` per spawn, a `node_entities` map — and the hierarchy and inspector
panels read *that*, not the entities `spawn_scene` actually created. The
inspector shows `SerializedComponent` JSON straight out of the file. So the
editor's picture of the scene and the world that is actually rendering are two
parallel structures kept in step by hand, and nothing the editor shows is
necessarily what the game sees.

This was not a mistake — it was the only thing possible. `ComponentRegistry`
(`crates/ecs/src/component/registry.rs`) is write-only: it maps a type name to
an `ErasedApply` that deserializes JSON *into* the world. There is no reverse
direction, so nothing can ask "what components does this entity have, and what
are their values?"

**The shell is a monolith.** `shell.rs` spawns every panel by hand and threads
every slot through one 250-line `spawn_shell`. Adding a panel means editing the
shell. Removing one means editing the shell. The panels also reach into each
other through shared editor-specific state rather than through a common
selection.

**Import is in-process.** `import_queue.rs` runs glTF import on a worker thread,
shells out to `zenity` for a file dialog, and handles dropped files — 272 lines
duplicating what `cargo run -p import` already does, in a tool that otherwise
only reads.

## Goals

- The editor inspects the live `World`: real entities, real component values.
- Panels are plugins. Adding, removing or reordering one does not touch a shell.
- Keyboard shortcuts are bindings on named actions, resolvable per context,
  configurable in one table.
- The editor gets smaller. `crates/editor` is ~2850 lines today; the deletions
  below remove roughly 1000 before the new panel machinery is added back.

## Non-goals

- **Writing.** Open scenes are read-only: no inspector edits, no scene saving,
  no import. The verb list is open a project, spawn a scene, navigate, select,
  inspect, frame. Everything in this design must survive editing being added
  later, but nothing here implements it.
- **Multi-select.** One selected thing at a time.
- **Drag-and-drop docking.** Slot topology is a constant (see Section 3).
- **Layout persistence to disk.** Split ratios live in a resource for the
  session; writing them to a config file is a separate concern.

## Decisions

Five forks were settled before this document, with the reasoning worth keeping:

| Decision | Chosen | Because |
| --- | --- | --- |
| What the editor inspects | The live ECS `World` | One source of truth; the same machinery a runtime debugger needs; deletes the parallel document tree |
| Extension points | Panel registry **and** per-type inspector registry | Two independent axes; together they are what lets `shell.rs` stop being a monolith |
| Shortcuts | Engine-level action map | A game wanting rebindable input would otherwise rebuild it; scattered `is_just_pressed` calls cannot be listed or rebound |
| Layout | Fixed slots, resizable, tabbed | A constant topology is what makes shortcuts and muscle memory work; docking costs more code than the rest of the editor |
| Reflection | Serde round-trip | Reuses the `Serialize` bound `SceneComponent` already requires; read-only is exactly the case it handles well |

The reflection decision has a known ceiling, recorded here so it is not
rediscovered: writing a single field back means re-serializing the whole
component. When editing lands, that is the moment to consider a real `Reflect`
trait with typed field access. Until then the round-trip is enough and costs
almost nothing.

---

## Section 1 — What we delete

| Removed | Size | Note |
| --- | --- | --- |
| `import_queue.rs` | 272 lines | Import moves to the CLI (`cargo run -p import -- assets/x.glb`), already how the examples work. Takes the `zenity` dialog and the dropped-file handler with it. |
| Most of `document.rs` | ~400 of 534 | `SceneDocument`, `validate`, `parents`, `roots`, `InstanceId`, `SceneInstance`, `node_entities`, `session_tree_rows`. |
| Tree bookkeeping in `hierarchy.rs` | ~150 lines | `expanded_instances` / `expanded_nodes` keyed by `(InstanceId, node)` become one set keyed by `Entity`. |
| Hard-coded shell in `shell.rs` | ~300 lines | Replaced by the dock registry. |

The `import` crate itself is untouched. Only the editor's in-process import path
goes, and with it the "Import glTF…" toolbar button and `ImportState`.

What survives from `document.rs` becomes `scene.rs`: read a `.gasset`, check the
header kind, deserialize the `Scene`, hand it to `spawn_scene`. The `validate`
forest check goes with the rest — `spawn_scene` is the consumer and owns its own
correctness.

`Selection` collapses from `Instance | Node { instance, node } | Asset` to
`Entity | Asset`.

---

## Section 2 — Engine additions

### 2a. The read side of the component registry (`crates/ecs`)

`register_scene_component::<T>` already requires `T: Serialize + DeserializeOwned`,
so the read function costs nothing extra to produce:

```rust
pub struct TypeInfo {
    pub name:  &'static str,   // "essential::transform::Transform"
    pub short: &'static str,   // "Transform"
    read: fn(&World, Entity) -> Option<serde_json::Value>,
}

impl World {
    pub fn component_types(&self, entity: Entity) -> impl Iterator<Item = &TypeInfo>;
    pub fn read_component(&self, entity: Entity, name: &str) -> Option<serde_json::Value>;
}
```

Implementation: `ComponentRegistry` gains `type_info: HashMap<TypeId, TypeInfo>`,
populated by the existing `register_scene_component`. `component_types` reads the
entity's archetype, walks `Archetype::component_ids()` — `ComponentId` *is*
`TypeId` — and yields the registered entries. The erased `read` calls
`World::get_component_for_entity::<T>()` and `serde_json::to_value`.

**Deliberate limit:** only types registered through `register_scene_component`
are visible. Types registered by the plain `register_component` path — UI
internals, render-world mirrors, `RenderEntity` — stay hidden. The inspector
shows game components, not engine plumbing.

**Failure mode:** a component whose `Serialize` fails (`SceneEntityRef::Entity`
refuses to serialize by design) yields `None` rather than propagating an error.
The inspector renders such a component by name with an "unavailable" note, so an
entity is never silently misrepresented as having fewer components than it does.

### 2b. Action map (`crates/window/src/input/actions.rs`)

Lives next to `Input` because it is a thin layer over it; a new crate for ~200
lines is not worth it and promoting it later is easy.

```rust
pub struct Modifiers { ctrl: bool, shift: bool, alt: bool, super_: bool }
pub struct Shortcut  { key: KeyCode, mods: Modifiers }

#[derive(Resource, Default)]
pub struct ActionMap {
    bindings: Vec<(Shortcut, ActionId, ContextId)>,
    contexts: Vec<ContextId>,   // stack, innermost last
}

#[derive(Event)]
pub struct ActionFired { pub action: ActionId }
```

`ActionId` and `ContextId` follow the same pattern as `ScheduleLabel`: the
`define_label!` macro in `crates/ecs/src/label.rs` generates the trait, the
`dyn` impls, and `Internable for dyn ActionLabel`, so `Interned<dyn ActionLabel>`
is `Copy` with pointer equality and hashing.

```rust
define_label!(ActionLabel);
define_label!(ContextLabel);

define_action!(FrameSelected);      // crates/editor/src/actions.rs
define_context!(Viewport);

bindings.bind(FrameSelected, Key::F, Viewport);
```

Typed and namespaced by Rust module rather than by string convention, so a typo
is a compile error and no `Internable for str` impl is needed.

Resolution, once per frame in `LateUpdate`, before the UI's own input systems:

1. Build the pressed `Shortcut` from `Input`.
2. Walk the context stack innermost-first. The first binding whose shortcut
   matches and whose context is the current one or the global context wins.
3. Fire one `ActionFired`.

A `TextEntry` context, pushed while a `UITextInput` holds focus, claims every
unmodified printable key, so typing in a filter can never fire an action while
`Ctrl+O` still can. This is the rule that makes contexts worth having.

Panels read `EventReader<ActionFired>`. Nothing calls `is_just_pressed` directly.

### 2c. UI work this depends on (`crates/ui`)

Four concrete gaps, in dependency order:

1. **Tab bodies.** `UITabStrip` / `UITab` exist and `update_widgets` sets
   `strip.selected`, but nothing shows or hides the bodies. The dock cannot work
   without this. Small.
2. **Focus ring.** `FocusedWidget` exists and nothing draws it. Keyboard
   navigation is unusable without a visible focus indicator. Small.
3. **Tab-order navigation.** Moving focus between widgets with the keyboard.
   Moderate, and the difference between "ergonomic" as a claim and as a fact.
4. **Scrollbars.** `UIScrollArea` gives no visual indication of position or
   extent. An editor with three scrolling panels needs them. Moderate.

---

## Section 3 — Editor architecture

```
crates/editor/src/
  lib.rs          EditorPlugin: dock + selection + project + built-in panels
  dock.rs         DockSlot, PanelDescriptor, PanelRegistry, shell build, tabs
  selection.rs    Selection resource + SelectionChanged event
  project.rs      open project, catalogue                     (trimmed)
  scene.rs        .gasset -> Scene -> spawn_scene             (was document.rs)
  inspector/
    mod.rs        InspectorRegistry
    generic.rs    serde_json::Value -> property rows
    builtin.rs    Transform, Camera, Light widgets
  panels/
    hierarchy.rs  inspector.rs  content.rs  viewport.rs
  actions.rs      editor action ids + default bindings
```

### The dock

```rust
pub enum DockSlot { Left, Center, Right, Bottom }

pub struct PanelDescriptor {
    pub id:    &'static str,   // "wonderland.hierarchy"
    pub title: &'static str,   // "Hierarchy"
    pub slot:  DockSlot,
    pub build: fn(PanelContext<'_>),
}

app.add_panel(PanelDescriptor { .. });
```

`PanelContext` carries the panel's body `Entity`, a `&mut CommandQueue` and
`&UITheme`. One `build_dock` system at `Startup`:

1. spawns the frame — toolbar, a `Left | Center | Right` row, a `Bottom` split,
   a status bar, wired with the `UISplitPane`s;
2. per slot, spawns a `UITabStrip` header plus one body per panel when the slot
   holds more than one, otherwise a single body;
3. calls each panel's `build` with its body entity.

`fn` pointers rather than trait objects: panel state belongs in resources and
components, ECS-style, so nothing needs to live on the descriptor, and the
registry stays trivially `Send + Sync`. A panel needing configuration registers
a resource from its own plugin, and registers its own systems there too.

Split ratios live in an `EditorLayout` resource for the session.

### Selection

```rust
pub enum SelectionKind { Entity(Entity), Asset(AssetId) }

#[derive(Resource, Default)]
pub struct Selection(pub Option<SelectionKind>);

#[derive(Event)]
pub struct SelectionChanged;
```

Panels never reference each other. Hierarchy writes on click, viewport writes on
pick, content writes on click; inspector and viewport read. That is the property
that makes a panel genuinely removable — delete its plugin registration and
nothing else breaks.

### Inspector registry

```rust
type InspectorWidget = fn(InspectorContext<'_>, &serde_json::Value);

#[derive(Resource, Default)]
pub struct InspectorRegistry(HashMap<&'static str, InspectorWidget>);

app.inspect::<Transform>(transform_widget);
```

Keyed by full type name. Unregistered types fall back to `generic::render`, which
walks the `Value`: objects become `UICollapsibleSection`s, scalar fields become
`UIPropertyRow`s, and arrays of numbers become a row of read-only fields. Both
widgets already exist in `crates/ui/src/widgets.rs` and are currently unused.

### The retained-UI constraint

This UI is retained — entities plus a Taffy solve — not immediate. Editor
inspectors are conventionally immediate-mode precisely because their contents
change shape constantly. On a retained UI the pattern must be **rebuild the
subtree when the shape changes, refresh values every frame**, which is what
`hierarchy.rs` already does with its pooled rows.

Consequences every panel must honour:

- Each panel declares what invalidates it (selection change, project revision,
  document generation) and rebuilds only then.
- Per-frame work is limited to writing text and colours into existing nodes.
- An inspector widget is a *builder*: it spawns UI entities. A companion refresh
  system updates their values.

This is a real cost of the architecture and is recorded so it is designed for
rather than discovered.

---

## Section 4 — Panels and theme

**Hierarchy** (Left). The world tree, rooted at entities carrying `SceneRoot`,
descending through `ChildOf` / `Children`. Rooting at scene roots rather than
"every entity without a parent" matters: the editor's own UI entities, helper
camera, light and grid live in the same `World` and would otherwise flood the
tree. A toolbar toggle reveals everything else — which is also the seed of the
runtime debugger. Virtualized (already built), filterable, keyboard navigable.

**Inspector** (Right). The components of the selected entity, via the registry.

**Content** (Bottom, tabbed). The project asset catalogue: virtualized list,
filter, click a Scene to spawn it.

The brainstorm chose to keep the `PAGE_SIZE` pager, but that decision assumed a
fixed-height sidebar. Once Content became a resizable dock panel, paging plus a
clipped rows area meant rows below the fold were simply unreachable — the panel
is now scrolled and virtualized like the hierarchy, and the pager is gone.

**Viewport** (Center). Mostly survives from today's `viewport.rs`: render-target
camera, orbit / pan / zoom, click-to-pick, frame-selected.

Deferred, with slots already available: a Log panel and a **Diagnostics** panel —
the latter nearly free, since `UILayoutDiagnostics` and `UIRenderDiagnostics` are
populated every frame and nothing reads them.

**Wonderland.** `UITheme::looking_glass()` is already the palette, so this is
mostly naming discipline: `wonderland.*` for panel and action ids, the shell
title, and adding `font_size_sm` / `font_size_md` / `font_size_lg` to `UITheme`
so panels stop hard-coding `14.0` and `12.0`. `UITheme` stays a resource so a
game can reskin the same widgets.

---

## Section 5 — Milestones

Each milestone leaves the editor working and is shippable on its own.

**M1 — Simplify and reflect.** *Done.* Deleted `import_queue.rs` and the
document model. Added the registry read side to `crates/ecs`. The hierarchy
shows live entities rooted at `SceneRoot`; the inspector shows real component
values.

Two additions the design did not anticipate, both forced by implementation:

- `SceneComponent` required only `DeserializeOwned`. Reading a component back
  needs `Serialize`, so it is now a supertrait bound. Every real implementor
  already derived it, so only two test fixtures changed.
- `spawn_scene` discarded `SceneNode.name`, and no `Name` component existed, so
  a tree of live entities had nothing to display but indices. `ecs` now has
  `Name`, and the spawner attaches it.

Editor line count went from 2850 to 2648 — less than the ~1000 the deletions
suggested, because the inspector arrived as a new 369-line module at the same
time. `document.rs` (534) and `import_queue.rs` (272) are gone; `selection.rs`,
`scene.rs` and `inspector.rs` (656 together) replace them.

**M2 — Dock.** *Done.* `PanelDescriptor`, `PanelRegistry`, `DockSlot` and
`UITabBody` in `crates/ui`. All panels register themselves; `shell.rs` is now
just the toolbar and status line.

Two departures from the sketch:

- `PanelDescriptor` carries no `build` function. A panel declares id, title and
  slot; building is its own `Startup` system, which looks its body up with
  `registry.body(PANEL_ID)`. The viewport needs `EditorViewport` to build, which
  a fixed `fn(&mut CommandQueue, Entity, &UITheme)` pointer could not supply —
  and a system gets whatever resources it wants for free.
- A **Diagnostics** panel was pulled forward from M4, because with one panel per
  slot the tab machinery was unexercised outside unit tests. It shares the
  bottom slot with Content and reads the counters that already existed.

Tab titles are sized from their text with an estimated advance width. Text
contributes nothing to Taffy's intrinsic sizing — the font system is a
render-world resource while layout runs in the main world — so an auto-width
node holding only a label collapses to its padding. Giving text nodes a real
measure function is the proper fix and is not scheduled.

**M3 — Keyboard.** *Done.* `ActionMap` in `crates/window/src/input/actions.rs`
with `ActionLabel`/`ContextLabel` from `define_label!`, plus `define_action!`
and `define_context!` for downstream crates. `UIFocusable`, the focus ring and
Tab navigation are in `crates/ui`; the editor's binding table is one function in
`actions.rs`.

Notes from the implementation:

- Tab navigation already existed but polled the key directly and ringed through
  *every* `Interactable`, which meant tree rows and split handles. It is now
  driven by `UIFocusNext`/`UIFocusPrevious` actions over `UIFocusable` widgets
  only.
- Panels push their own contexts rather than a central system knowing about
  them: the hierarchy pushes `TreeContext` while focus is inside it, the
  viewport pushes `ViewportContext` while the pointer is over it. That keeps
  panels self-contained and is what removed the `focus is a TreeRegion` guard
  from tree navigation.
- The focus ring borrows the widget's own border and restores it on blur, rather
  than adding an overlay entity that would have to track the widget's rect every
  frame. A consequence worth knowing: while arrow keys move the tree
  *selection*, the ring stays on the row that was clicked, because that is the
  widget holding keyboard focus.

**M4 — Polish.** *Done.* Scrollbars, per-type inspector formatting for
`Transform`, `Camera` and `Light`, and a theme pass. (The diagnostics panel
landed early, in M2.)

One deviation worth recording. The design had inspector extensions spawn UI:
`fn(InspectorContext<'_>, &Value)`. They format text instead:

```rust
pub type ComponentFormatter = fn(&Value) -> Option<String>;
app.inspect("essential::transform::Transform", format_transform);
```

Real property rows need a per-component entity pool and a fixed-width label
column, because text still cannot measure itself — the same limitation that
forces estimated tab widths and the inspector's `CHARS_PER_LINE` guess. Giving
text nodes a Taffy measure function is the change that unblocks all three, and
it is the natural first item if this continues. Until then a formatter registry
buys most of the readability for a fraction of the work, keyed on the canonical
type path so two types sharing a short name cannot borrow each other's
formatter, and returning `Option` so a formatter can decline a shape it does not
recognise rather than misreport it.

Scrollbars overlay their content by absolute positioning rather than taking a
column, so gaining one does not reflow the panel. The thumb is the visible
fraction of the content, clamped to a grabbable minimum with the travel rescaled
so it still lands flush at both ends, and it can be dragged.

**M10 — Typography.** *Done.* Fonts became a registry rather than a call to
`FontSystem::new()` in two places. `UIFonts` collects faces during plugin
`build`; both font systems — the one that measures during layout and the one
that renders — are built from it in `finish`, which is what makes it impossible
for them to disagree about a face. The editor registers Inter (400/500/600) and
Phosphor, and points the default sans-serif family at Inter.

Icons are glyphs, not images: they shape, measure, clip and colour like any
other text, and a row's icon is simply a text node in another family. The
codepoints live in `editor::fonts::glyph`, taken from Phosphor 2.1's own
mapping, with a test that shapes every constant and fails on `.notdef` — a
wrong codepoint otherwise shows up as a blank box nobody traces back.

Two layout rules fell out of it, both invisible until text could measure itself:

- A label that ellipsises opts out of the flex automatic minimum size, exactly
  as `min-width: 0` does in CSS. Without it the label's min-content width is the
  whole string, so it widens its row past the panel and spills instead of
  ending in an ellipsis.
- Only the label in a row may shrink. The icon columns are `flex_shrink: 0`, or
  the overflow is shared out and every column ends up slightly wrong.

`TextComponent::ellipsis` also needs `wrap: false` to mean anything, which is
easy to forget: a wrapping label silently keeps its full text.

Two more surfaced when the icons went in:

- **A scroll area's content is absolutely positioned**, pinned left and right,
  and offset through `inset.top` rather than a negative margin. In flow, the
  content's height fed back into the viewport's own size: a taller list grew its
  card, a scrolled one shrank it, and the panel collapsed as the two chased each
  other frame after frame. A viewport's size must come from the panel, never
  from what it is scrolling.
- **Icon columns cannot borrow the row's padding helper.** A 20px box with 8px
  of padding a side leaves 4px of content, which clips a glyph to a sliver or,
  for a centred one, to nothing at all.

Tree indentation is capped at six levels. A skeleton nests twenty deep, and a
row you cannot read is worse than one whose depth you infer from its parents.

**M11 — Cleanup.** *Done.* The project bar is gone (the foot is reserved for
logs); opening a project is `--project` until then, and with it went the folder
chooser. Selecting an entity no longer moves the camera — framing is something
you ask for, with `F` and `Shift+F`. Instructional labels ("scroll to browse",
"select an entity to inspect it") are gone, leaving the counts. Focus is shown
by the caret alone: both the generic focus ring and the text field's own
hard-coded blue border are removed.

The window is undecorated, which needed an explicit `focus_window()` — a
borderless window is not always given focus by the window manager, and without
focus it receives no keys at all. It has no drag handle of its own yet.

**Navigation is Unreal's, not Blender's.** Right button held: mouse look, `WASD`
in camera space, `E`/`Q` along world up, `Shift` to boost, wheel to set the fly
speed. Wheel with no button dollies. Middle drag pans. Nothing moves the camera
unless a button is held, which is what keeps `W` typed into a search field from
flying the view across the level.

Two things surfaced while checking the new clear colour against the palette: the
editor's grid and its light were spawned without `SyncWithRenderWorld`, so
neither had ever reached the render world. The viewport had been showing an
unlit scene against a flat clear colour, with no ground at all.

**M12 — Window chrome.** *Done.* Undecorated leaves the application owing the
user everything a title bar gave them, so `window_chrome.rs` provides it: the
brand is the drag handle (double click maximises), three buttons sit beside the
stats readout, and eight invisible grips — four edges, four corners, above every
panel — start a resize.

The drag handle is the whole top band — `TOP_STRIP`, the strip plus its margin,
spanning the window — not just the brand. Everything standing in that band is a
label except the window buttons, which are lifted above it by `z_index`; the
resize grips sit higher still, so a corner stays a corner.

Move and resize are *requests*: winit's `drag_window` and `drag_resize_window`
send `_NET_WM_MOVERESIZE` (X11) or `xdg_toplevel::move` (Wayland) and the
compositor takes the gesture from there. A compositor may ignore them, and no
fallback is possible on Wayland, where a client cannot position itself. Hence
`--decorated`, which hands the frame back to the window manager.

Closing needed a way to ask the event loop to stop: `CloseRequest` in the window
crate, checked once per iteration in `about_to_wait`. `CloseRequested` from the
window manager was the only exit before.

Two traps worth remembering. An icon node must be big enough for its *line box*,
not just its glyph: a line taller than its content box is dropped rather than
clipped, which is a button that renders nothing at all. And a node is only
hit-tested with `Interactable` — grips laid out perfectly but silently received
no presses without it.

**M13 — Viewport resolution.** *Done.* The viewport had been rendering at
800×600 and stretching, whatever size the panel was. `sync_viewport_size` was
writing to a texture asset that was no longer there: opening a project calls
`publish_content(clear_loaded)`, which cleared *every* typed asset store,
including the render target the editor had created in memory. Content assets
reload from the new snapshot; a runtime-created one cannot, so it was simply
gone, and the camera kept its original allocation forever.

The asset server now tracks ids created through `add` and keeps them across a
content switch — they belong to no snapshot. Two consequences had to follow it:
the camera publishes its render target under its `render_target_generation`
rather than a constant revision, and the UI material signature hashes that
revision, so a bind group built against a resized target is rebuilt rather than
left holding the old texture.

**M14 — Marks, not icons.** *Done.* The design has no icon set — no `<svg>`,
no icon font, nothing in the Nocturne bundle. Rows say what they hold with 7–11px
shapes in the palette, so `editor::marks` draws exactly those: an outlined
rounded square for a group, a filled diamond for a mesh, an outlined rect for a
camera, a filled dot for a light or material, a filled square for a texture.
Each mark is one node whose material is the shape, updated per pooled row.
Phosphor stays only where the design has no mark of its own: the window buttons
and the status glyph. The brand is the design's burrow mouth, drawn.

The diamond needed the UI's first transform: `UIMaterial::rotation`, applied in
the SDF shader by rotating the sample point, with the shape shrunk by
`|cos| + |sin|` so it stays inside a node that has not moved.

Also from 3a: expanders are the design's `▸`/`▾`, groups show a trailing child
count, selection is a 20% accent wash across the whole row, and Curiosities
gained the `⌕` search pill and kind tags wired to `filtered_assets`.

One trap, found by shaping the arrows: **Inter has no `▸` or `▾`**, and text
was shaped with `Shaping::Basic`, which has no font fallback — so any character
the UI font lacks drew as a .notdef box. Text now shapes with
`Shaping::Advanced`, in both the measuring and rendering font systems.

**M15 — Inspector read-outs.** *Done.* The inspector draws the design's
card, still read-only. The header is one line: kind mark · name · `#id` · `⋯`.
Each component is a card on a 7% wash with a `▾` chevron, an accent diamond, its
name, and the design's enable dot (always on: components have no enabled flag
yet). The body is fields rather than text: a label, then a recessed box per
value, so a vector is three boxes and a uniform scale is one box noted
"uniform". A card with nothing to show, a marker or a value that would not
serialize, says so in mono after its name instead of opening a body.

`ComponentFormatter` changed accordingly, from `fn(&Value) -> Option<String>` to
`fn(&Value) -> Option<Vec<Field>>`. The shape a card is rebuilt on now includes
each field's label and box count, so a scale turning uniform respawns its row
while a moving transform only refreshes text.

Clicking a card's header folds it. Folds are kept by type path, not entity, so
a folded `Transform` stays folded as the selection moves. The design's
"add component" row is drawn and inert; its `⌘⇧A` hint is left off, since
nothing is bound to it and no installed font has those symbols.

Folding exposed a picking bug. The scene is full-bleed, so its rect contains
every click in the window, and `pick_viewport` gated on that rect — a click on
a card that missed every mesh cleared the selection. Picking now requires the
hovered node to be the scene itself, and card bodies are `Interactable` so their
padding and titles stop the pointer.

Not drawn: the design's "systems touching this entity" footer. Nothing can
answer which systems touch an entity yet.

**M16 — PostUpdate retired.** *Done.* Master's event refactor (#61) double
buffers every channel and gives each reader its own cursor, advancing the
buffers in a new `First` schedule at the top of the frame. An event written
after a reader has already run is therefore delivered next frame instead of
being dropped, which is what `PostUpdate` was working around: the old flush ran
at the end of `LateUpdate`, so layout had to sit after every mutation but before
that flush.

The layout pass now ends `LateUpdate` instead. Ordering comes from registration
order, so `UIPlugin` is registered *last* in the editor — after the panels that
build UI — or layout would resolve what they built on the previous frame.

## Testing

Pure logic, covered by unit tests:

- **ecs:** register a scene component, spawn an entity, read it back and assert
  the JSON matches; an unregistered type is invisible; an entity with no
  registered components yields an empty iterator; a component whose `Serialize`
  fails yields `None` rather than panicking.
- **window:** binding resolution walks the context stack innermost-first; the
  `TextEntry` context swallows unmodified printable keys but not modified ones;
  modifier matching is exact, so `Ctrl+O` does not fire a bare `O` binding.
- **ui:** tab-body visibility follows `strip.selected`; tab order wraps at both
  ends; scrollbar thumb geometry from offset and extent.
- **editor:** panels land in the slot they declare and share a tab strip when
  they collide; the generic renderer shapes nested objects and numeric arrays as
  expected; the hierarchy roots only at `SceneRoot` unless the toggle is on.

The interactive half — picking, scrolling, focus movement, a shortcut actually
firing — is verified by driving the running editor under XWayland with XTest and
reading back screenshots, the method that caught the split-pane delta and resize
bugs and that confirmed the scroll rework.

## Deferred, deliberately

- Editing and saving scenes; the `Reflect` trait that would make per-field
  editing pleasant.
- Multi-select, drag-and-drop docking, layout persistence to disk.
- Undo/redo. Worth noting that a command-queue editor with a single `Selection`
  resource is a reasonable base for it, but nothing here anticipates it further.

### A separate world for the edited scene

Designed and costed, then deferred. The editor would keep its UI and state in
the application world while the scene being displayed lives in its own `World`,
reached through a param the editor crate defines for itself.

The findings are worth keeping, because the costing was better than it looked:

- The primitive is *park-and-run*, which `extract()` already implements: park
  world A inside world B as a resource, run a schedule in B, swap back. Making
  the access param generic over the parked slot — `Parked<Slot, T>`, with
  `Extracted<T> = Parked<Source, T>` — lets any crate declare its own slot, so
  the editor's param never has to be known to `app`, `render` or `ui`.
- `SubApps` would become an ordered `Vec<SubApp>`, each optionally declaring a
  target and an extract function, rather than today's two hard-coded fields.
- Splitting `Extract` into `Extract` (application content: window, asset stores,
  UI) and `ExtractWorld` (scene content: cameras, lights, meshes, skeletons,
  materials) means five **registration** moves in `crates/render`, not param
  changes — those systems keep using `Extracted<T>`, which during that pass is
  the scene world. A plain game extracts with both schedules from its one world.
- Only two sites need a global resource from inside a scene-content system:
  `camera.rs` (`AssetStore<Texture>`) and `skeleton.rs` (`AssetStore<Skeleton>`).
- **Trap:** `despawn_stale_render_entities` decides staleness with
  `!main.entity_is_valid(**main_entity)`. With more than one source world feeding
  one render world, entity ids collide across worlds, so `MainEntity` must record
  which source it came from and staleness must be checked against that world.

What it buys beyond a hierarchy that needs no UI filtering: closing a scene
becomes dropping a world instead of recursive despawn bookkeeping, and play-mode
becomes ticking that world with game schedules while the editor keeps running.

What it does not buy: the viewport camera, light and grid must live in the scene
world to be extracted, so the hierarchy keeps a filter either way — which is why
the `SceneRoot` rooting and `EditorHelper` filter below are not throwaway work.
