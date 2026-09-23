# Multi-primitive Mesh assets

## Problem

A `Mesh` asset is one glTF primitive. Importing a glTF emits one `.gasset` per
primitive, named `mesh/0` through `mesh/N`, which produces hundreds of
unnamed files for a single source and floods the editor's content panel.

The single-primitive shape exists because the render world's unit is one
entity drawing one primitive with one material. That invariant is load-bearing:
`material_renderpass::<M>` queries `(RenderMeshInstance,
RenderMaterialComponent<M>)`, and an entity cannot hold two
`MaterialComponent<M>` of the same type.

The two needs are reconciled by keeping one render entity per primitive while
making the *asset* a container, and moving the fan-out from import time to
extract time.

## Asset shape

```rust
pub struct Mesh {
    pub primitives: Vec<Primitive>,
}

pub struct Primitive {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}
```

`Mesh::single(vertices, indices)` covers the hand-built call sites. Primitives
carry no name: glTF primitives are unnamed, and the slot index is their
identity.

`compute_normals` and `compute_tangents` move to `Primitive`; `Mesh` forwards
to every primitive. `Mesh::local_aabb` unions the primitives, and
`Primitive::local_aabb` is added for later per-primitive culling.

## Import

One `.gasset` per glTF *mesh*, not per source file. A file-level asset would
force loading every mesh together and collapse the instancing batch key; a mesh
must stay independently addressable.

The sub-asset name comes from `mesh.name()`, falling back to `mesh/{index}`
when the source has no name. Two glTF meshes may share a name, so a name
already taken by an earlier mesh index gets `mesh/{name}.{index}` — keyed on
the glTF mesh index, which is stable across re-imports of an unchanged source.
A glTF mesh named `Barrel` lands at `content/barrel/mesh_Barrel.gasset`.

Existing content must be re-imported. `.import.toml` keys outputs by sub-asset
name, so renaming reallocates UUIDs and orphans scene references. The
primitive-to-mesh regrouping renumbers every output regardless, so the break is
unavoidable and the readable names come with it.

OBJ sources emit one `Mesh` per file, with tobj models as primitives. OBJ's
per-material groups map onto primitives exactly.

## Render

```rust
struct RenderMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    primitives: Vec<PrimitiveRange>,
}

struct PrimitiveRange {
    indices: Range<u32>,
    base_vertex: i32,
}

struct RenderMeshInstance {
    mesh_asset_id: AssetId,
    primitive: u32,
    transform: wgpu::Buffer,
}
```

`prepare_asset` concatenates every primitive into one vertex buffer and one
index buffer, recording each primitive's index range and base vertex. This is
one allocation pair per mesh asset rather than per primitive.

The draw loop binds the vertex and index buffers once per mesh and issues
`draw_indexed(range, base_vertex, 0..1)` per primitive.

### Extract-time fan-out

`extract_meshes` gains `Extracted<Res<AssetStore<Mesh>>>` to read the primitive
count. The mesh may not be loaded on the first frames, so fan-out is deferred
until the asset resolves and redone when the handle changes.

The owner render entity carries `RenderMeshFanout { mesh_id, entities:
Vec<Entity> }`. Each primitive entity carries `MainEntity`, so
`despawn_stale_render_entities` reclaims them without new cleanup code, while
`spawn_new_render_entities` still creates only the single owner mirror per main
entity.

## Materials

```rust
pub struct MaterialComponent<M: Material = StandardMaterial> {
    pub binding: SlotBinding<M>,
}

pub enum SlotBinding<M: Material> {
    All(AssetHandle<M>),
    PerSlot(Vec<Option<AssetHandle<M>>>),
}
```

`All` exists because the common single-material case cannot know the primitive
count at authoring time, and it keeps today's call sites to
`MaterialComponent::all(handle)`.

Slots live on `MaterialComponent<M>` rather than on `MeshComponent` so material
dispatch stays typed. Two material types coexist on one entity as two
components, each covering a subset of slots.

`extract_materials::<M>` walks the owner's `RenderMeshFanout` list and writes
`RenderMaterialComponent<M>` onto the render entity for each slot its binding
covers.

### Fallback material

A magenta `StandardMaterial` is registered through `AssetServer::add` at plugin
startup and held in a `FallbackMaterial` resource.

`extract_meshes` tags each new primitive entity `NeedsFallbackMaterial`. Every
`extract_materials::<M>` clears the tag on the slots it claims. A single system
at the start of the `Render` stage gives anything still tagged a
`RenderMaterialComponent<StandardMaterial>` pointing at the fallback.

The fallback runs in a later schedule group rather than behind an ordering
edge: `.after()` duplicates systems in this engine, and `Extract` completes
before `Render` begins, so no edge is needed.

Two material types claiming the same slot produce two components of different
types on one render entity, so both passes draw it. This is an authoring error
rather than a state the engine can resolve, and nothing can detect it by
querying, since `RenderMaterialComponent<M1>` and `<M2>` are distinct types with
no cross-type query.

It is detected by bookkeeping instead. A `ClaimedSlots` render-world resource
holds the primitive entities claimed this frame. Every `extract_materials::<M>`
records its claims there and warns when an entity is already present; resource
writes are immediate rather than deferred, so the second claimant sees the
first regardless of which order the per-type systems run in. The system that
applies the fallback clears the set, which is correct without an ordering edge
because it runs in `Render`, after all of `Extract`.

## Instancing

The batch key becomes `(mesh_asset_id, primitive, material_asset_id)` — the
same shape as today with one field added. Because a mesh's primitives share
buffers, a batch collapses to one bind plus one `draw_indexed` with instance
count N, and the per-entity single-element instance buffers can later be
replaced by a pooled per-batch buffer.

No instancing is built here. The point is that the current
one-primitive-per-asset layout forces one tiny wgpu buffer pair per primitive
forever, which blocks batching; the container removes that obstacle.

## Knock-on changes

**Skeletons.** The synthesized `<node>.primitiveN` child nodes carrying rootless
`SkeletonComponent` are deleted from the importer. A skinned node keeps one
`SkeletonComponent` and one `AnimationPlayer`, and the render fan-out copies the
binding onto each primitive entity.

**Hierarchy panel.** Scene trees no longer contain per-primitive child nodes.

**Physics.** `shape.rs` merges every primitive into one collider, offsetting
indices per primitive. A static collider wants the whole mesh, not one slot.

**Editor.** The content panel's mesh flood resolves without panel changes. The
`KINDS` filter is unchanged.

**Hand-built meshes.** `skybox`, `world-grid`, `physics::simulation`, the
`render-test` and `physics-test` examples construct `Mesh { vertices, indices }`
directly and move to `Mesh::single`.

## Testing

- `Mesh` round-trips through bincode with zero, one, and several primitives.
- `Mesh::local_aabb` unions primitive bounds; an empty mesh returns `None`.
- The glTF importer emits one sub-asset per glTF mesh, named from the source,
  with duplicate and missing names resolved deterministically.
- The importer emits no per-primitive scene nodes, and a skinned multi-primitive
  node carries exactly one `SkeletonComponent`.
- `RenderMesh` preparation records index ranges and base vertices that address
  each primitive's geometry in the concatenated buffers.
- Fan-out creates one render entity per primitive, defers until the asset
  loads, rebuilds when the mesh handle changes, and despawns with the main
  entity.
- `SlotBinding::All` covers every slot; `PerSlot` covers only its populated
  slots.
- A primitive whose slot no binding covers receives the fallback material; a
  covered slot does not.
- A mesh collider built from a multi-primitive mesh contains every primitive's
  triangles.
