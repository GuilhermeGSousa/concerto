# Mesh Bounds and Viewport Picking — Design

**Status:** implemented
**Branch:** `functional-editor`
**Builds on:** `2026-10-06-entity-operations-design.md`

## Problem

An entity can only be selected from the hierarchy tree. In a level with a few
hundred nodes, finding the one you are looking at means reading names. The
viewport shows the scene but cannot be clicked.

Underneath that, nothing in the engine knows how big a mesh entity is.
`Mesh::local_aabb()` rescans every vertex each time it is called, and framing
with F calls it for every mesh node.

## Goals

- Left-clicking a mesh in the viewport selects its entity; clicking empty space
  clears the selection.
- The selection is visible in the viewport.
- Mesh entities carry their bounds as a component, computed once per asset.

## Non-goals

- **Frustum culling.** The bounds component is its prerequisite, but nothing
  here culls.
- **Picking entities without a mesh.** Lights, cameras and empties are selected
  from the tree.
- **Hover highlighting.** It would run the query every frame and need a
  per-mesh acceleration structure.
- **Multi-select and box select.**
- **GPU picking.** Considered and set aside; see Decisions.

## Decisions

| Decision | Chosen | Because |
| --- | --- | --- |
| Picking method | CPU ray cast: every bounded entity, sorted by entry distance, then triangles with early exit | Measured at 0.19 ms for 10,000 boxes and at most 11 ms for 262,000 triangles in release, once per click; it is what Bevy's mesh picking does; a GPU id buffer needs a new pass and a readback |
| Where bounds live | A local-space `Aabb` component, cached per asset | Avoids an asset lookup per entity; allows a per-entity override; a world-space box would be recomputed every frame because propagation rewrites `GlobalTransform` every frame |
| Detecting a changed mesh | A companion `AabbSource(AssetId)` compared each frame | `Changed<T>` is frame-exact and misses a change made after the system ran; an id comparison per entity is cheap and cannot miss |
| What a click selects | The entity that owns the hit mesh | Predictable; selecting an enclosing group instead is a later refinement |
| What counts as a click | Left press and release on the viewport within 4 px | The transform gizmo will need left drags |

## Section 1 — Bounds

`Aabb` already exists in `concerto_mesh` and already derives `Component`.
Nothing inserts it. A new system in `concerto-mesh`, registered by
`RenderPlugin` next to the `Mesh` asset:

```rust
pub fn update_mesh_bounds(/* mesh entities, AssetStore<Mesh>, a per-asset cache */)
```

- An entity with a `MeshComponent` and no `Aabb` gets one, in the mesh's local
  space, once the mesh asset has loaded. It also gets `AabbSource`, recording
  which asset the box came from.
- When an entity's mesh handle no longer matches its `AabbSource`, the box is
  recomputed for the new asset.
- An `Aabb` without an `AabbSource` was set by hand and is left alone.
- When the `MeshComponent` is removed, a computed `Aabb` goes with it.
- A mesh with no vertices gets no `Aabb`.

The box for each asset is computed once and kept in a cache local to the
system, so a thousand instances of one mesh scan its vertices once.

Two known limits. A skinned mesh gets its bind-pose box, which does not follow
animation. `WorldGrid` draws through a three-vertex placeholder mesh and so
gets a zero-size box; that is harmless here, but culling will need a way to opt
an entity out.

Framing (F and Shift-F) switches from rescanning vertices to reading `Aabb`.

## Section 2 — Ray tests

A new module in `concerto-mesh`:

```rust
pub struct Ray { pub origin: Vec3, pub direction: Vec3 }

impl Ray {
    pub fn at(&self, t: f32) -> Vec3;
    /// The same ray in another space. The direction is not renormalized, so a
    /// distance along one is the same distance along the other.
    pub fn transformed(&self, matrix: Mat4) -> Ray;
}

impl Aabb {
    /// Where the ray enters the box: zero when it starts inside, `None` on a miss.
    pub fn ray_entry(&self, ray: &Ray) -> Option<f32>;
}

impl Mesh {
    /// The nearest triangle the ray hits, from either side.
    pub fn ray_hit(&self, ray: &Ray) -> Option<f32>;
}
```

Triangles are hit from both sides so a room can be picked from inside it.

## Section 3 — Picking

A new module, `crates/editor/src/picking.rs`.

**The ray.** The pointer position relative to the viewport node's layout rect
becomes normalized device coordinates. The near and far points at those
coordinates are unprojected through the inverse of the editor camera's
view-projection matrix, and the ray runs from one to the other.

**The query.**

1. Collect the entities under the scene root that have a `MeshComponent`, an
   `Aabb` and a `GlobalTransform`.
2. For each, move the ray into the entity's local space and take
   `Aabb::ray_entry`. Keep the hits.
3. Sort by entry distance.
4. Walk the list, testing triangles with `Mesh::ray_hit`. Stop when the next
   entry distance is beyond the nearest triangle hit so far.

Restricting to the scene root keeps editor helpers out, including anything a
gizmo adds later.

**The click.** The viewport node listens for a left press and a left click. A
click counts when the pointer has moved less than 4 px since the press and the
camera is not being flown. A hit selects the entity and reveals its row in the
hierarchy. A miss clears the selection.

## Section 4 — Showing the selection

A system draws the selection with the debug gizmos, which already render into
the viewport's camera and always on top:

- An entity with its own `Aabb` gets that box, oriented with the entity.
- An entity without one gets a single axis-aligned box around its descendants'
  boxes, when it has any.
- The scene root gets nothing. It is selected whenever a scene loads, and a box
  around the whole level is noise.

## Debug gizmos

Found while implementing: debug gizmos had not drawn since the main and render
worlds were separated. Their render system was registered on the main world's
`Render` schedule, which never runs, so nothing was drawn and the main world's
line buffer was never cleared. The selection box and the transform gizmo both
depend on them, so they are ported here:

- The main world clears `GizmoStorage` in `First`.
- An `Extract` system copies the frame's lines into a render-world resource.
- The draw runs in the render world, in a new `RenderSet::Overlay` that follows
  `Draw`.
- Material passes join `RenderSet::Draw`, which the set's documentation already
  described but nothing was in, and the UI pass runs after `Overlay`, because it
  samples the viewport's camera target.

## Limits to know about

- A skinned mesh is picked and outlined in its bind pose.
- A click picks the mesh entity itself, which in an imported model is often a
  child of the node you think of as the object.
- Transparent texels still count as a hit; a fence drawn with alpha cut-outs
  is picked as a solid quad.
- In a debug build, a click that has to test every triangle of a Sponza-sized
  scene takes around 50 ms.

## Files

| File | Change |
| --- | --- |
| `crates/mesh/src/ray.rs` | New: `Ray`, `Aabb::ray_entry`, `Mesh::ray_hit` |
| `crates/mesh/src/bounds.rs` | New: `AabbSource`, `update_mesh_bounds` |
| `crates/render/src/plugin.rs` | Register `update_mesh_bounds` |
| `crates/debug-gizmos/src/` | Port to the split worlds: clear, extract, draw in the render world |
| `crates/render/src/sets.rs`, `plugin.rs`, `material_plugin.rs` | `RenderSet::Overlay`; material passes in `Draw` |
| `crates/ui/src/plugin.rs` | UI pass after `Overlay` |
| `crates/editor/src/picking.rs` | New: viewport ray, the query, click handling, selection drawing |
| `crates/editor/src/viewport.rs` | Attach the listeners; framing reads `Aabb` |
| `crates/editor/src/lib.rs` | Register the picking plugin |
| `crates/editor/README.md` | Document picking and its limits |

## Testing

Unit tests, headless:

- **mesh, rays:** a box is entered at the right distance from outside, at zero
  from inside, and missed when beside or behind the ray, including a ray
  parallel to a face; a triangle is hit from both sides and missed outside its
  edges; of two triangles the nearer distance is returned; a transformed ray
  keeps its distances.
- **mesh, bounds:** the box appears once the asset has loaded and not before;
  it follows a changed handle; a hand-set box is untouched; it is removed with
  the `MeshComponent`; an empty mesh gets none.
- **editor, ray:** the centre of the viewport gives the camera's forward
  direction; a corner matches the field of view and aspect ratio.
- **editor, query:** a small mesh inside a larger hollow one is picked when the
  ray reaches it first, and the larger one is picked when it does not; a
  rotated and scaled entity is hit where it is drawn; an entity outside the
  scene is ignored and does not block; a miss returns nothing.
- **editor, click:** a click selects the hit and reveals its row; a press that
  moved past the threshold selects nothing; a miss clears the selection.
- **editor, selection drawing:** an entity with a box draws twelve edges; a
  group draws twelve around its descendants; the scene root draws none.

Then one pass in the running editor, driven in code as before: click a mesh,
confirm the tree and inspector follow, and confirm the box is drawn around it.
