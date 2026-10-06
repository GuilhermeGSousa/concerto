# Transform Gizmo — Design

**Status:** implemented
**Branch:** `functional-editor`
**Builds on:** `2026-10-06-viewport-picking-design.md`

## Problem

An entity can be selected in the viewport but only moved by typing numbers
into the inspector. Placing things in a level needs direct manipulation.

Debug gizmo lines are one pixel wide and cannot be made wider, which is too
thin for a handle that has to be seen and grabbed.

## Goals

- Move and rotate the selected entity by dragging handles in the viewport.
- Handles are thick, colour-coded by axis, and stay the same size on screen.
- A drag marks the scene dirty once, when it ends.

## Non-goals

- **Scale.** Translate and rotate only.
- **Plane handles and free rotation.** One axis at a time.
- **Local-space handles.** Handles follow the world axes.
- **Snapping.**
- **Undo.**

## Decisions

| Decision | Chosen | Because |
| --- | --- | --- |
| Thick lines | A second gizmo pipeline that expands each segment into a screen-space quad | Exact pixel widths at any distance; the one-pixel line list stays as it is for everything else |
| Width API | `DebugGizmos::set_line_width(pixels)`, applying to every shape drawn afterwards in that system | Circles and arrows become thick for free |
| Handle hit test | Pointer distance in pixels to the handle's projection | Matches what is drawn regardless of distance |
| Colours | X red, Y green, Z blue; the hovered or dragged handle yellow | The usual convention |
| Mode keys | W translate, E rotate, in the viewport context | The usual convention; ignored while the camera is being flown, which also uses W |
| Input | Polled each frame, not UI signals | A node has one listener per signal type and picking already owns the viewport's press and click |

## Section 1 — Thick lines

`DebugGizmos` gains a line width, one pixel by default, reset for each system
that requests it. Every shape goes through the same `push_line`, which routes a
segment to the existing line list when the width is one and to a new buffer
otherwise.

A wide segment is six vertices. Each carries both endpoints, which corner of
the quad it is, the width and the colour. The vertex shader projects both
endpoints, clamps either one that is behind the camera onto the near side of
the segment, takes the perpendicular in pixels, and offsets by half the width.
It needs the target's size, so the gizmo pass binds a small per-camera uniform
taken from the camera's depth texture, which always matches its target.

Widths are in pixels of the camera's target. A caller on a scaled display
multiplies by the window's scale factor.

## Section 2 — The gizmo

A new module, `crates/editor/src/gizmo.rs`.

**When it shows.** For the selected entity, when it has a `Transform` and is
not the scene root, and the camera is not being flown.

**Where and how big.** At the entity's world position, aligned to the world
axes. Its size is a fixed fraction of the viewport height, so it does not grow
or shrink with distance.

**Translate.** Three arrows. Dragging one moves the entity along that axis: the
point on the axis nearest the pointer's ray is tracked, and the entity follows
the difference from where the drag began.

**Rotate.** Three rings. Dragging one turns the entity about that axis: the
pointer's ray is intersected with the ring's plane and the entity follows the
change in angle. A ring seen nearly edge-on cannot be dragged, because the ray
and the plane are almost parallel.

**Parents.** Handles work in world space and the result is written back to the
entity's local `Transform` through its parent's world transform.

**Picking.** A press that lands on a handle starts a drag and is not a pick.

**Dirty state.** The document is marked once when a drag that moved the entity
ends.

## Files

| File | Change |
| --- | --- |
| `crates/debug-gizmos/src/` | Line width, wide-segment buffer, second pipeline and shader |
| `crates/editor/src/gizmo.rs` | New: modes, hit test, drag, drawing |
| `crates/editor/src/picking.rs` | A press on a handle is not a pick; `world_to_viewport` |
| `crates/editor/src/asset_editor.rs` | A query-based form of `mark_entity_edited` |
| `crates/editor/src/actions.rs` | `GizmoTranslate`, `GizmoRotate` |
| `crates/editor/README.md` | Document the gizmo |

## Testing

Unit tests, headless:

- **debug-gizmos:** a one-pixel line goes to the line list and a wider one to
  the wide buffer as six vertices; the width resets per system.
- **editor, maths:** the nearest point on an axis to a ray; the angle of a ray
  on a plane, and none when the ray is edge-on; a world point projects to the
  right viewport position and to nothing behind the camera.
- **editor, hit test:** the pointer over an arrow or ring finds that axis; away
  from all of them finds none; of two overlapping handles the nearer wins.
- **editor, drag:** a translate drag moves the entity along the axis by the
  amount the pointer's ray moved; a rotate drag turns it by the angle swept;
  both come out right under a rotated, scaled parent; a drag marks the document
  once at the end and a press without movement does not; a press on a handle
  does not pick.
- **editor, modes:** W and E switch mode and are ignored while flying.

Then one pass in the running editor, driven in code: select a mesh, drag each
kind of handle, and confirm the handles are thick and coloured.
