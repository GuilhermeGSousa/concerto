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

The two needs are reconciled by making the *asset* a container and keeping one
render entity per mesh, which draws every primitive from its `RenderMesh`.
Material dispatch moves from the entity to the primitive slot.

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
    transform: wgpu::Buffer,
    coverage: SlotCoverage,
}
```

`prepare_asset` concatenates every primitive into one vertex buffer and one
index buffer, recording each primitive's index range and base vertex. This is
one allocation pair per mesh asset rather than per primitive.

The draw loop binds the vertex and index buffers once per mesh instance and
issues `draw_indexed(range, base_vertex, 0..1)` per primitive whose slot the
pass's material type covers. The shadow pass ignores materials and draws the
whole index buffer in one call.

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

`extract_materials::<M>` mirrors the binding as asset ids in
`RenderMaterialComponent<M>` on the mesh's render entity, and
`material_renderpass::<M>` looks up each primitive's material by slot index.

### Fallback material

A magenta `StandardMaterial` is registered through `AssetServer::add` at plugin
startup and held in a `FallbackMaterial` resource.

Which slots have a material is tracked per instance, not in a global set.
`extract_meshes` clears the instance's `SlotCoverage` each frame, and every
`extract_materials::<M>` (ordered after it) marks the slots its binding covers.
The `StandardMaterial` pass then draws, with the fallback, every primitive whose
slot nothing covered. A new instance starts fully covered, so it skips the
fallback until its materials have been extracted.

Two material types covering the same slot draw it twice. That is an authoring
error; `SlotCoverage` detects it while marking and warns once per instance.

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
`SkeletonComponent` and one `AnimationPlayer`, and every primitive draws with that
skeleton's palette.

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
- `SlotBinding::All` covers every slot; `PerSlot` covers only its populated
  slots.
- A primitive whose slot no binding covers receives the fallback material; a
  covered slot does not.
- A mesh collider built from a multi-primitive mesh contains every primitive's
  triangles.
