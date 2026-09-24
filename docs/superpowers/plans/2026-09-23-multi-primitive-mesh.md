# Multi-primitive Mesh Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `Mesh` a container of primitives so one glTF mesh imports as one named `.gasset`, while the render world keeps one entity per primitive.

**Architecture:** `Mesh` becomes `Vec<Primitive>`. `RenderMesh` concatenates every primitive into one vertex/index buffer pair and records per-primitive ranges. The per-primitive fan-out moves from imported scene nodes to extract time: one owner render entity spawns N primitive render entities. Materials bind per slot on `MaterialComponent<M>`, with a magenta fallback for slots nothing covers.

**Tech Stack:** Rust, wgpu, custom ECS (`crates/ecs`), bincode-serialized content assets, `gltf` and `tobj` crates.

**Spec:** `docs/superpowers/specs/2026-09-23-multi-primitive-mesh-design.md`

## Global Constraints

- Comment style: no narrative comments. Only one-line `///` doc comments on `pub` items. Explain design decisions in the PR/chat, never in source.
- Never use `.after()` for system ordering — it duplicates systems in this engine. Order by schedule group (`Extract` completes before `Render`).
- Content extension is `gasset`; content root is `content` (`crates/import/src/config.rs`).
- Every task must leave `cargo check --workspace` green and its crate's tests passing.
- `Vertex::MAX_AFFECTED_BONES == 4`.
- Jolt is native-only by design; `crates/physics/src/backend/jolt.rs` does not build for wasm. Use `cargo check --workspace` on native.

---

### Task 1: `Mesh` becomes a container of primitives

Splits geometry into a new `Primitive` type and reshapes `Mesh` to hold a `Vec` of them. Every existing construction site moves to `Mesh::single`, and the two consumers that need flat geometry (`RenderMesh` preparation, physics colliders) go through one new `Mesh::merged_geometry` helper. Nothing changes behaviourally: every asset still has exactly one primitive.

**Files:**
- Create: `crates/mesh/src/primitive.rs`
- Modify: `crates/mesh/src/mesh.rs`, `crates/mesh/src/lib.rs`
- Modify: `crates/render/src/render_asset/render_mesh.rs:27-48`
- Modify: `crates/physics/src/backend/rapier.rs:220-237`, `crates/physics/src/backend/jolt.rs:382-408`
- Modify: `crates/physics/src/simulation.rs:52-62`
- Modify: `crates/skybox/src/plugin.rs:12-15`
- Modify: `crates/world-grid/src/world_grid.rs:66-69`
- Modify: `crates/gltf-loader/src/gltf_importer.rs:204, 723-726`
- Modify: `crates/obj-loader/src/obj_importer.rs:244-247`
- Modify: `examples/render-test/src/main.rs:259`, `examples/physics-test/src/main.rs:254, 295, 329`
- Test: `crates/mesh/tests/mesh_serialization.rs`, `crates/mesh/tests/primitive_container.rs` (create)

**Interfaces:**
- Consumes: nothing (first task).
- Produces:
  - `mesh::primitive::Primitive { pub vertices: Vec<Vertex>, pub indices: Vec<u32> }`, `Serialize + Deserialize + Default`
  - `Primitive::local_aabb(&self) -> Option<Aabb>`
  - `Primitive::compute_normals(&mut self) -> &mut Self`
  - `Primitive::compute_tangents(&mut self) -> &mut Self`
  - `mesh::mesh::Mesh { pub primitives: Vec<Primitive> }`
  - `Mesh::single(vertices: Vec<Vertex>, indices: Vec<u32>) -> Mesh`
  - `Mesh::local_aabb(&self) -> Option<Aabb>` (union over primitives)
  - `Mesh::compute_normals(&mut self) -> &mut Self`, `Mesh::compute_tangents(&mut self) -> &mut Self` (forward to every primitive)
  - `Mesh::merged_geometry(&self) -> (Vec<Vertex>, Vec<u32>)` — all primitives concatenated with per-primitive index offsetting
  - `mesh::Primitive` re-exported from `crates/mesh/src/lib.rs`

- [ ] **Step 1: Write the failing tests**

Create `crates/mesh/tests/primitive_container.rs`:

```rust
//! Covers the multi-primitive `Mesh` container: construction, bounds union,
//! and the merged geometry used by colliders and GPU upload.
use glam::Vec3;
use mesh::mesh::Mesh;
use mesh::primitive::Primitive;
use mesh::vertex::Vertex;

fn vertex_at(x: f32, y: f32, z: f32) -> Vertex {
    Vertex {
        pos_coords: [x, y, z],
        ..Default::default()
    }
}

fn triangle(offset: f32) -> Primitive {
    Primitive {
        vertices: vec![
            vertex_at(offset, 0.0, 0.0),
            vertex_at(offset + 1.0, 0.0, 0.0),
            vertex_at(offset, 1.0, 0.0),
        ],
        indices: vec![0, 1, 2],
    }
}

#[test]
fn single_builds_a_one_primitive_mesh() {
    let mesh = Mesh::single(vec![vertex_at(1.0, 2.0, 3.0)], vec![0]);

    assert_eq!(mesh.primitives.len(), 1);
    assert_eq!(mesh.primitives[0].vertices.len(), 1);
    assert_eq!(mesh.primitives[0].indices, vec![0]);
}

#[test]
fn local_aabb_unions_every_primitive() {
    let mesh = Mesh {
        primitives: vec![triangle(0.0), triangle(10.0)],
    };

    let bounds = mesh.local_aabb().expect("a populated mesh has bounds");
    assert_eq!(bounds.min, Vec3::ZERO);
    assert_eq!(bounds.max, Vec3::new(11.0, 1.0, 0.0));
}

#[test]
fn local_aabb_is_none_when_no_primitive_has_vertices() {
    assert_eq!(Mesh { primitives: vec![] }.local_aabb(), None);
    assert_eq!(Mesh::single(vec![], vec![]).local_aabb(), None);
}

#[test]
fn merged_geometry_offsets_each_primitives_indices() {
    let mesh = Mesh {
        primitives: vec![triangle(0.0), triangle(10.0)],
    };

    let (vertices, indices) = mesh.merged_geometry();

    assert_eq!(vertices.len(), 6);
    assert_eq!(
        indices,
        vec![0, 1, 2, 3, 4, 5],
        "the second primitive's indices shift by the first's vertex count"
    );
    assert_eq!(vertices[3].pos_coords, [10.0, 0.0, 0.0]);
}

#[test]
fn a_multi_primitive_mesh_round_trips_through_bincode() {
    let mesh = Mesh {
        primitives: vec![triangle(0.0), triangle(5.0), triangle(9.0)],
    };

    let bytes = bincode::serialize(&mesh).expect("serializes");
    let restored: Mesh = bincode::deserialize(&bytes).expect("deserializes");

    assert_eq!(restored.primitives.len(), 3);
    assert_eq!(
        restored.primitives[2].vertices[0].pos_coords,
        [9.0, 0.0, 0.0]
    );
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p mesh --test primitive_container`
Expected: FAIL to compile — `unresolved import mesh::primitive`, `no function or associated item named single`.

- [ ] **Step 3: Create the `Primitive` type**

Create `crates/mesh/src/primitive.rs`. Move the bodies of `compute_normals` and `compute_tangents` verbatim from `crates/mesh/src/mesh.rs:68-158` — they operate on `self.vertices`/`self.indices` and need no edits beyond living on `Primitive`.

```rust
use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};

use crate::mesh::Aabb;
use crate::vertex::Vertex;

/// One drawable unit of a [`crate::mesh::Mesh`]: geometry for a single draw
/// call, paired at render time with the material in the matching slot.
#[derive(Default, Serialize, Deserialize)]
pub struct Primitive {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl Primitive {
    /// Computes this primitive's local-space bounds from its vertex positions.
    pub fn local_aabb(&self) -> Option<Aabb> {
        let first = self.vertices.first()?;
        let mut min = Vec3::from(first.pos_coords);
        let mut max = min;
        for vertex in &self.vertices[1..] {
            let point = Vec3::from(vertex.pos_coords);
            min = min.min(point);
            max = max.max(point);
        }
        Some(Aabb { min, max })
    }

}
```

Then move both compute methods across as a pure cut-and-paste: take `pub fn compute_normals` (`crates/mesh/src/mesh.rs:70-98`) and `pub fn compute_tangents` (`crates/mesh/src/mesh.rs:100-158`) out of the `impl Mesh` block and paste them verbatim into this `impl Primitive` block. Both bodies read and write only `self.vertices` and `self.indices`, so not a character inside them changes. Delete the narrative comment block above the `has_uvs` branch rather than carrying it over (see Global Constraints), and keep the `use glam::{Vec2, Vec3}` import — `compute_tangents` needs `Vec2`.

- [ ] **Step 4: Reshape `Mesh`**

In `crates/mesh/src/mesh.rs`, replace the struct and its `impl` block (keep `Aabb`, `MeshComponent`, and the `LoadableAsset` impl exactly as they are):

```rust
use crate::primitive::Primitive;

#[derive(Asset, Default, serde::Serialize, serde::Deserialize)]
pub struct Mesh {
    pub primitives: Vec<Primitive>,
}

impl Mesh {
    /// A mesh with exactly one primitive, for procedurally built geometry.
    pub fn single(vertices: Vec<Vertex>, indices: Vec<u32>) -> Self {
        Self {
            primitives: vec![Primitive { vertices, indices }],
        }
    }

    /// The union of every primitive's local-space bounds, or `None` when no
    /// primitive has vertices.
    pub fn local_aabb(&self) -> Option<Aabb> {
        self.primitives
            .iter()
            .filter_map(Primitive::local_aabb)
            .reduce(|a, b| Aabb {
                min: a.min.min(b.min),
                max: a.max.max(b.max),
            })
    }

    /// Every primitive concatenated into one vertex and index list, with each
    /// primitive's indices offset by the vertices preceding it.
    pub fn merged_geometry(&self) -> (Vec<Vertex>, Vec<u32>) {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        for primitive in &self.primitives {
            let base = vertices.len() as u32;
            vertices.extend_from_slice(&primitive.vertices);
            indices.extend(primitive.indices.iter().map(|index| index + base));
        }
        (vertices, indices)
    }

    pub fn compute_normals(&mut self) -> &mut Self {
        for primitive in &mut self.primitives {
            primitive.compute_normals();
        }
        self
    }

    pub fn compute_tangents(&mut self) -> &mut Self {
        for primitive in &mut self.primitives {
            primitive.compute_tangents();
        }
        self
    }
}
```

Remove the `use glam::{Mat4, Vec2, Vec3}` import of `Vec2` if it becomes unused; `Mat4` and `Vec3` are still needed by `Aabb`.

In `crates/mesh/src/lib.rs`:

```rust
pub mod mesh;
pub mod primitive;
pub mod skeleton;
pub mod vertex;

pub use mesh::{Mesh, MeshComponent};
pub use primitive::Primitive;
pub use skeleton::{Skeleton, SkeletonComponent};
pub use vertex::Vertex;
```

- [ ] **Step 5: Run the new tests**

Run: `cargo test -p mesh --test primitive_container`
Expected: PASS (5 tests).

- [ ] **Step 6: Update the existing mesh tests**

In `crates/mesh/tests/mesh_serialization.rs`, replace each `Mesh { vertices: …, indices: … }` literal with `Mesh::single(…, …)`. There are four, at lines 20, 26, 39 and 65.

Run: `cargo test -p mesh`
Expected: PASS.

- [ ] **Step 7: Update the flat-geometry consumers**

`crates/render/src/render_asset/render_mesh.rs` — replace the two `source_asset.vertices`/`source_asset.indices` uses. This keeps behaviour identical for the single-primitive assets that exist today; Task 2 replaces it with per-primitive ranges.

```rust
let (merged_vertices, merged_indices) = source_asset.merged_geometry();

let vertices = context
    .device
    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Vertex Buffer"),
        contents: bytemuck::cast_slice(&merged_vertices),
        usage: wgpu::BufferUsages::VERTEX,
    });

let indices = context
    .device
    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Index Buffer"),
        contents: bytemuck::cast_slice(&merged_indices),
        usage: wgpu::BufferUsages::INDEX,
    });
let index_count = merged_indices.len() as u32;
```

`crates/physics/src/backend/rapier.rs:220-236` — merging every primitive is the intended collider behaviour:

```rust
fn create_shape_from_mesh(mesh: &Mesh) -> Result<Self::ShapeHandle, MeshShapeCreationError> {
    let (merged_vertices, merged_indices) = mesh.merged_geometry();
    let vertices: Vec<rapier3d::math::Vector> = merged_vertices
        .iter()
        .map(|vertex| rapier3d::math::Vector::from_array(vertex.pos_coords))
        .collect();
    // `chunks_exact` drops a trailing partial triangle, matching the Jolt
    // shim rather than failing the whole mesh over it.
    let indices: Vec<[u32; 3]> = merged_indices
        .as_chunks::<3>()
        .0
        .iter()
        .map(|triangle| [triangle[0], triangle[1], triangle[2]])
        .collect();

    SharedShape::trimesh(vertices, indices).map_err(|_| MeshShapeCreationError)
}
```

`crates/physics/src/backend/jolt.rs:382-408` — same merge, and the safety comment must now describe the merged buffers:

```rust
fn create_shape_from_mesh(mesh: &Mesh) -> Result<Self::ShapeHandle, MeshShapeCreationError> {
    let (merged_vertices, merged_indices) = mesh.merged_geometry();
    // `Vertex` interleaves normals, UVs and skinning weights with the
    // positions, so the positions have to be packed before Jolt can read
    // them as xyz triples.
    let positions: Vec<f32> = merged_vertices
        .iter()
        .flat_map(|vertex| vertex.pos_coords)
        .collect();

    // SAFETY: `positions` holds `merged_vertices.len()` xyz triples and
    // `merged_indices` `len()` indices; both are read during the call only.
    let shape = unsafe {
        jolt_ffi::jolt_create_mesh_shape(
            positions.as_ptr(),
            merged_vertices.len() as u32,
            merged_indices.as_ptr(),
            merged_indices.len() as u32,
        )
    };

    if !shape.is_null() {
        Ok(ShapeHandle(shape))
    } else {
        Err(MeshShapeCreationError)
    }
}
```

- [ ] **Step 8: Update every remaining construction site**

Each is a mechanical `Mesh { vertices: V, indices: I }` → `Mesh::single(V, I)`:

- `crates/physics/src/simulation.rs:59` → `mesh::Mesh::single(vertices, indices)`
- `crates/skybox/src/plugin.rs:12` → `Mesh::single(SKYBOX_VERTICES.to_vec(), SKYBOX_INDICES.to_vec())`
- `crates/world-grid/src/world_grid.rs:66` → `render::assets::mesh::Mesh::single(vec![Vertex::default(); 3], vec![0, 1, 2])`
- `examples/render-test/src/main.rs:259` → `let mut mesh = Mesh::single(vertices, indices);`
- `examples/physics-test/src/main.rs:254, 295, 329` → same shape, three sites
- `crates/obj-loader/src/obj_importer.rs:244` → `let mut mesh = Mesh::single(vertices, mesh_data.indices.clone());`
- `crates/gltf-loader/src/gltf_importer.rs:723` → `let mut primitive = Mesh::single(Vec::new(), Vec::new());` is wrong here; instead change `load_primitive` to build and return a `Primitive`:

```rust
fn load_primitive(
    source_path: &Path,
    mesh_name: Option<&str>,
    buffers: &[Data],
    gltf_primitive: &Primitive,
) -> Result<mesh::Primitive, ImportError> {
```

Change line 723 to `let mut primitive = mesh::Primitive::default();` and leave the rest of the function body untouched — it only assigns `primitive.vertices` and `primitive.indices`. Note the name clash: `Primitive` is already imported from `gltf`, so refer to the engine type as `mesh::Primitive`.

Then at `crates/gltf-loader/src/gltf_importer.rs:204`, wrap the returned primitive so each still emits its own asset for now (Task 6 regroups these):

```rust
let m = load_primitive(source_path, mesh.name(), &buffers, &gltf_primitive)?;
ctx.emit(
    &format!("mesh/{mesh_counter}"),
    &Mesh {
        primitives: vec![m],
    },
)?;
```

- [ ] **Step 9: Verify the whole workspace**

Run: `cargo check --workspace --all-targets`
Expected: no errors.

Run: `cargo test -p mesh -p physics -p gltf-loader -p obj-loader`
Expected: PASS. The glTF and OBJ importer tests assert on emitted sub-asset names and counts, which are unchanged by this task.

- [ ] **Step 10: Commit**

```bash
git add crates/mesh crates/render/src/render_asset/render_mesh.rs crates/physics crates/skybox crates/world-grid crates/gltf-loader crates/obj-loader examples
git commit -m "Make Mesh a container of primitives

Adds Primitive, reshapes Mesh to hold a Vec of them, and routes the two
flat-geometry consumers (GPU upload, mesh colliders) through
Mesh::merged_geometry. Every asset still has exactly one primitive, so
behaviour is unchanged.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 2: `RenderMesh` records per-primitive ranges

Replaces the single `index_count` with a range table so the draw loops can address one primitive at a time. Still one primitive drawn per entity (the fan-out lands in Task 5), so every entity draws primitive 0 — which for today's single-primitive assets is the whole mesh.

**Files:**
- Modify: `crates/render/src/render_asset/render_mesh.rs`
- Modify: `crates/render/src/components/mesh.rs:11-14, 52-57`
- Modify: `crates/render/src/material_plugin.rs:288-305`
- Modify: `crates/render/src/components/shadows.rs:615-622`
- Test: `crates/render/tests/render_mesh_ranges.rs` (create)

**Interfaces:**
- Consumes: `Mesh { primitives: Vec<Primitive> }`, `Mesh::merged_geometry` (Task 1).
- Produces:
  - `render::render_asset::render_mesh::PrimitiveRange { pub indices: std::ops::Range<u32>, pub base_vertex: i32 }` (`Clone`)
  - `RenderMesh { vertices, indices, primitives: Vec<PrimitiveRange> }`
  - `RenderMesh::primitive(&self, index: u32) -> Option<&PrimitiveRange>`
  - `render::render_asset::render_mesh::primitive_ranges(mesh: &Mesh) -> Vec<PrimitiveRange>` — `pub(crate)`, extracted so it is testable without a GPU device
  - `RenderMeshInstance { mesh_asset_id: AssetId, primitive: u32, transform: wgpu::Buffer }`

Because `merged_geometry` already offsets indices into the concatenated vertex buffer, `base_vertex` is always `0`. It is carried anyway so a later change to per-primitive vertex packing does not have to touch the draw loops.

- [ ] **Step 1: Write the failing test**

Create `crates/render/tests/render_mesh_ranges.rs`:

```rust
//! Covers the primitive range table `RenderMesh` builds from a `Mesh`.
//! Split out from `prepare_asset` so it runs without a GPU device.
use mesh::mesh::Mesh;
use mesh::primitive::Primitive;
use mesh::vertex::Vertex;
use render::render_asset::render_mesh::primitive_ranges;

fn primitive(vertex_count: usize, index_count: usize) -> Primitive {
    Primitive {
        vertices: vec![Vertex::default(); vertex_count],
        indices: vec![0; index_count],
    }
}

#[test]
fn each_primitive_gets_a_contiguous_index_range() {
    let mesh = Mesh {
        primitives: vec![primitive(4, 6), primitive(8, 12), primitive(3, 3)],
    };

    let ranges = primitive_ranges(&mesh);

    assert_eq!(ranges.len(), 3);
    assert_eq!(ranges[0].indices, 0..6);
    assert_eq!(ranges[1].indices, 6..18);
    assert_eq!(ranges[2].indices, 18..21);
}

#[test]
fn base_vertex_is_zero_because_indices_are_pre_offset() {
    let mesh = Mesh {
        primitives: vec![primitive(4, 6), primitive(8, 12)],
    };

    let ranges = primitive_ranges(&mesh);

    assert!(ranges.iter().all(|range| range.base_vertex == 0));
}

#[test]
fn a_mesh_with_no_primitives_has_no_ranges() {
    assert!(primitive_ranges(&Mesh { primitives: vec![] }).is_empty());
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p render --test render_mesh_ranges`
Expected: FAIL to compile — `unresolved import render::render_asset::render_mesh::primitive_ranges`.

- [ ] **Step 3: Implement the range table**

In `crates/render/src/render_asset/render_mesh.rs`:

```rust
/// Where one primitive's geometry sits inside a [`RenderMesh`]'s shared
/// vertex and index buffers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrimitiveRange {
    pub indices: std::ops::Range<u32>,
    pub base_vertex: i32,
}

/// The index range each primitive occupies in `Mesh::merged_geometry` output.
pub fn primitive_ranges(mesh: &Mesh) -> Vec<PrimitiveRange> {
    let mut start = 0u32;
    mesh.primitives
        .iter()
        .map(|primitive| {
            let end = start + primitive.indices.len() as u32;
            let range = PrimitiveRange {
                indices: start..end,
                base_vertex: 0,
            };
            start = end;
            range
        })
        .collect()
}

pub(crate) struct RenderMesh {
    pub(crate) vertices: wgpu::Buffer,
    pub(crate) indices: wgpu::Buffer,
    pub(crate) primitives: Vec<PrimitiveRange>,
}

impl RenderMesh {
    pub(crate) fn primitive(&self, index: u32) -> Option<&PrimitiveRange> {
        self.primitives.get(index as usize)
    }
}
```

In `prepare_asset`, replace the `index_count` line and the struct literal:

```rust
Ok(RenderMesh {
    vertices,
    indices,
    primitives: primitive_ranges(source_asset),
})
```

`primitive_ranges` and `PrimitiveRange` must be `pub` (not `pub(crate)`) for the integration test to reach them; `RenderMesh` itself stays `pub(crate)`. Confirm `crates/render/src/lib.rs` declares `pub mod render_asset;` and that `render_asset/mod.rs` declares `pub mod render_mesh;` — both already do.

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p render --test render_mesh_ranges`
Expected: PASS (3 tests).

- [ ] **Step 5: Add the primitive index to `RenderMeshInstance`**

In `crates/render/src/components/mesh.rs`:

```rust
#[derive(Component)]
pub(crate) struct RenderMeshInstance {
    pub(crate) mesh_asset_id: AssetId,
    pub(crate) primitive: u32,
    pub(crate) transform: wgpu::Buffer,
}
```

And in `extract_meshes`, the constructed instance gains `primitive: 0`:

```rust
let instance = RenderMeshInstance {
    mesh_asset_id: mesh.handle.id(),
    primitive: 0,
    transform: instance_buffer,
};
```

- [ ] **Step 6: Draw through the range in both passes**

`crates/render/src/material_plugin.rs`, replacing lines 302-304:

```rust
let Some(range) = mesh.primitive(mesh_instance.primitive) else {
    continue;
};

render_pass.set_vertex_buffer(0, mesh.vertices.slice(..));
render_pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
render_pass.set_vertex_buffer(1, mesh_instance.transform.slice(..));
render_pass.draw_indexed(range.indices.clone(), range.base_vertex, 0..1);
```

`crates/render/src/components/shadows.rs`, replacing lines 618-621 with the same four statements. Read the surrounding block first — it sits inside an `if let Some(mesh) = …` and the `let Some(range) = … else { continue; }` must go directly after that binding.

- [ ] **Step 7: Verify**

Run: `cargo check --workspace --all-targets && cargo test -p render`
Expected: no errors, tests pass.

Run: `cargo run -p render-test`
Expected: the scene renders exactly as before this task. (See `docs/superpowers` memory on visual verification if a screenshot is wanted.)

- [ ] **Step 8: Commit**

```bash
git add crates/render
git commit -m "Address primitives by range in RenderMesh

RenderMesh records one index range per primitive instead of a single
index count, and RenderMeshInstance names which primitive it draws. Both
draw loops go through the range. Every instance still draws primitive 0.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 3: `MaterialComponent<M>` binds slots

Reshapes `MaterialComponent<M>` from a single handle to a slot binding. Still one render entity per main entity, so only slot 0 can reach the GPU — Task 5 makes the rest reachable. This task is about the type and its serialization.

**Files:**
- Modify: `crates/render/src/components/material.rs`
- Modify: `crates/render/src/material_plugin.rs:146-163`
- Modify: `crates/obj-loader/src/obj_importer.rs:71-78`
- Modify: `crates/gltf-loader/src/gltf_importer.rs` (in `push_mesh_components`)
- Modify: `examples/render-test/src/main.rs`, `examples/physics-test/src/main.rs`, `crates/editor/src/viewport.rs` — every `MaterialComponent { handle }` literal
- Test: `crates/render/tests/material_slots.rs` (create)

**Interfaces:**
- Consumes: `RenderMeshInstance.primitive` (Task 2).
- Produces:
  - `render::components::material::SlotBinding<M: Material>` with variants `All(AssetHandle<M>)` and `PerSlot(Vec<Option<AssetHandle<M>>>)`, `Serialize + Deserialize + Clone`
  - `SlotBinding::<M>::slot(&self, index: u32) -> Option<&AssetHandle<M>>`
  - `MaterialComponent<M> { pub binding: SlotBinding<M> }`
  - `MaterialComponent::<M>::all(handle: AssetHandle<M>) -> Self`
  - `MaterialComponent::<M>::per_slot(slots: Vec<Option<AssetHandle<M>>>) -> Self`

- [ ] **Step 1: Write the failing test**

Create `crates/render/tests/material_slots.rs`:

```rust
//! Covers per-primitive material slot binding.
use essential::assets::handle::AssetHandle;
use essential::assets::AssetId;
use render::assets::material::StandardMaterial;
use render::components::material::{MaterialComponent, SlotBinding};

#[test]
fn all_covers_every_slot_index() {
    let id = AssetId::new();
    let binding = SlotBinding::<StandardMaterial>::All(AssetHandle::weak(id));

    for index in [0, 1, 7, 1000] {
        assert_eq!(
            binding.slot(index).map(AssetHandle::id),
            Some(id),
            "All must answer for slot {index}"
        );
    }
}

#[test]
fn per_slot_covers_only_its_populated_entries() {
    let wood = AssetId::new();
    let rope = AssetId::new();
    let binding = SlotBinding::<StandardMaterial>::PerSlot(vec![
        Some(AssetHandle::weak(wood)),
        None,
        Some(AssetHandle::weak(rope)),
    ]);

    assert_eq!(binding.slot(0).map(AssetHandle::id), Some(wood));
    assert!(binding.slot(1).is_none(), "an empty slot is uncovered");
    assert_eq!(binding.slot(2).map(AssetHandle::id), Some(rope));
    assert!(binding.slot(3).is_none(), "past the end is uncovered");
}

#[test]
fn a_slot_binding_round_trips_through_json() {
    let binding = SlotBinding::<StandardMaterial>::PerSlot(vec![
        Some(AssetHandle::weak(AssetId::new())),
        None,
    ]);
    let component = MaterialComponent::<StandardMaterial> { binding };

    let text = serde_json::to_string(&component).expect("serializes");
    let restored: MaterialComponent<StandardMaterial> =
        serde_json::from_str(&text).expect("deserializes");

    match restored.binding {
        SlotBinding::PerSlot(slots) => {
            assert_eq!(slots.len(), 2);
            assert!(slots[0].is_some());
            assert!(slots[1].is_none());
        }
        SlotBinding::All(_) => panic!("PerSlot must not deserialize as All"),
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p render --test material_slots`
Expected: FAIL to compile — `SlotBinding` not found.

- [ ] **Step 3: Implement `SlotBinding` and reshape `MaterialComponent`**

In `crates/render/src/components/material.rs`, replace the `MaterialComponent` definition (keep the `RenderMaterialComponent` half of the file unchanged):

```rust
/// Which material each of a mesh's primitive slots draws with.
#[derive(Serialize, Deserialize, Clone)]
#[serde(bound = "")]
pub enum SlotBinding<M: Material + Send + Sync + 'static = StandardMaterial> {
    /// One material for every slot, however many the mesh turns out to have.
    All(AssetHandle<M>),
    /// One entry per slot; `None` leaves that slot to another material type
    /// or to the fallback.
    PerSlot(Vec<Option<AssetHandle<M>>>),
}

impl<M: Material + Send + Sync + 'static> SlotBinding<M> {
    /// The material covering `index`, or `None` when this binding leaves it open.
    pub fn slot(&self, index: u32) -> Option<&AssetHandle<M>> {
        match self {
            SlotBinding::All(handle) => Some(handle),
            SlotBinding::PerSlot(slots) => slots.get(index as usize)?.as_ref(),
        }
    }
}

#[derive(Component, Serialize, Deserialize)]
#[serde(bound = "")]
pub struct MaterialComponent<M: Material + Send + Sync + 'static = StandardMaterial> {
    pub binding: SlotBinding<M>,
}

impl<M: Material + Send + Sync + 'static> MaterialComponent<M> {
    /// One material on every primitive — the common single-material case.
    pub fn all(handle: AssetHandle<M>) -> Self {
        Self {
            binding: SlotBinding::All(handle),
        }
    }

    /// One material per primitive slot, `None` leaving a slot uncovered.
    pub fn per_slot(slots: Vec<Option<AssetHandle<M>>>) -> Self {
        Self {
            binding: SlotBinding::PerSlot(slots),
        }
    }
}
```

The `SceneComponent` impl below it resolves `self.handle` through the `AssetServer`. It must now walk the binding:

```rust
impl<M: Material + LoadableAsset> SceneComponent for MaterialComponent<M> {
    fn apply(mut self, entity: Entity, ctx: &mut SceneSpawnContext<'_>) {
        if let Some(server) = ctx.get_resource::<AssetServer>() {
            match &mut self.binding {
                SlotBinding::All(handle) => *handle = server.load(handle.id()),
                SlotBinding::PerSlot(slots) => {
                    for handle in slots.iter_mut().flatten() {
                        *handle = server.load(handle.id());
                    }
                }
            }
        }
        ctx.insert(self, entity);
    }
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p render --test material_slots`
Expected: PASS (3 tests).

Add `serde_json` to `crates/render/Cargo.toml` under `[dev-dependencies]` if it is not already there.

- [ ] **Step 5: Update `extract_materials`**

In `crates/render/src/material_plugin.rs:146-163`, read the covering material for the instance's own primitive:

```rust
pub(crate) fn extract_materials<M: Material>(
    materials: Extracted<Query<(&MaterialComponent<M>, &RenderEntity)>>,
    render_meshes: Query<&RenderMeshInstance>,
    render_materials: Query<&RenderMaterialComponent<M>>,
    mut cmd: CommandQueue,
) {
    for (material, render_entity) in materials.iter() {
        let render_entity = **render_entity;

        if render_materials.get_entity(render_entity).is_some() {
            continue;
        }

        let Some(instance) = render_meshes.get_entity(render_entity) else {
            continue;
        };
        let Some(handle) = material.binding.slot(instance.primitive) else {
            continue;
        };

        cmd.insert(
            RenderMaterialComponent::<M>::new(handle.id()),
            render_entity,
        );
    }
}
```

- [ ] **Step 6: Update every construction site**

Replace each `MaterialComponent { handle }` / `MaterialComponent::<M> { handle }` literal with `MaterialComponent::all(handle)`. Find them with:

Run: `rg -n 'MaterialComponent\s*(::<[^>]+>)?\s*\{' --type rust`

Known sites: `crates/obj-loader/src/obj_importer.rs:73`, `crates/gltf-loader/src/gltf_importer.rs` (inside `push_mesh_components`), `crates/editor/src/viewport.rs`, `examples/render-test/src/main.rs`, `examples/physics-test/src/main.rs`.

- [ ] **Step 7: Verify**

Run: `cargo check --workspace --all-targets && cargo test -p render -p gltf-loader -p obj-loader -p scene`
Expected: PASS.

Run: `cargo run -p render-test`
Expected: unchanged rendering.

- [ ] **Step 8: Commit**

```bash
git add crates/render crates/gltf-loader crates/obj-loader crates/editor examples
git commit -m "Bind materials per primitive slot

MaterialComponent<M> carries a SlotBinding instead of a single handle:
All for the common one-material case, PerSlot for per-primitive
assignment. extract_materials resolves the slot the instance draws.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 4: Magenta fallback for uncovered slots

A primitive no `MaterialComponent<M>` covers currently does not draw at all, which reads as missing geometry. It should draw magenta so the mistake is visible. This lands before the fan-out so the mechanism is in place when multi-primitive meshes start producing uncovered slots.

**Files:**
- Create: `crates/render/src/components/fallback_material.rs`
- Modify: `crates/render/src/components/mod.rs`
- Modify: `crates/render/src/components/mesh.rs` (tag new instances)
- Modify: `crates/render/src/material_plugin.rs` (clear the tag)
- Modify: `crates/render/src/plugin.rs:160-175`
- Test: `crates/render/tests/fallback_material.rs` (create)

**Interfaces:**
- Consumes: `SlotBinding`, `MaterialComponent::all` (Task 3); `RenderMeshInstance.primitive` (Task 2).
- Produces:
  - `render::components::fallback_material::FallbackMaterial(pub AssetHandle<StandardMaterial>)` — a `Resource` in the **main** world
  - `render::components::fallback_material::RenderFallbackMaterial(pub AssetId)` — a `Resource` in the **render** world
  - `render::components::fallback_material::NeedsFallbackMaterial` — a marker `Component` on render-world primitive entities
  - `render::components::fallback_material::insert_fallback_material` — a system registered in the `Render` schedule
  - `render::components::fallback_material::fallback_material_asset() -> StandardMaterial` — the magenta material, textureless
  - `render::components::fallback_material::ClaimedSlots(pub std::collections::HashSet<Entity>)` — a render-world `Resource` recording which primitive entities a material claimed this frame

The fallback runs in `Render`, not behind an ordering edge, because `.after()` duplicates systems here and `Extract` fully completes before `Render` starts.

- [ ] **Step 1: Write the failing test**

Create `crates/render/tests/fallback_material.rs`:

```rust
//! Covers the magenta stand-in applied to primitive slots no material covers.
use color::Color;
use render::components::fallback_material::fallback_material_asset;

#[test]
fn the_fallback_material_is_opaque_magenta() {
    let material = fallback_material_asset();

    assert_eq!(material.base_color_factor(), Color::rgba(1.0, 0.0, 1.0, 1.0));
}

#[test]
fn the_fallback_material_samples_no_textures() {
    let material = fallback_material_asset();

    assert!(material.base_color_texture().is_none());
    assert!(material.normal_texture().is_none());
}
```

Check `crates/render/src/assets/material.rs` for the accessor names before writing this — `StandardMaterial` is built through setters (`set_base_color_factor`, `set_metallic_factor`, …), and the getters may be named differently or be absent. If a getter is missing, add the minimal `pub fn` needed rather than making the field public, and match the existing accessor naming in that file.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p render --test fallback_material`
Expected: FAIL to compile — module `fallback_material` not found.

- [ ] **Step 3: Implement the fallback module**

Create `crates/render/src/components/fallback_material.rs`:

```rust
use ecs::{component::Component, resource::Res, CommandQueue, Query, Resource};
use essential::assets::{handle::AssetHandle, AssetId};

use crate::assets::material::StandardMaterial;
use crate::components::material::RenderMaterialComponent;

/// The magenta stand-in drawn where no material covers a primitive slot.
pub fn fallback_material_asset() -> StandardMaterial {
    let mut material = StandardMaterial::new(None, None);
    material.set_base_color_factor(color::Color::rgba(1.0, 0.0, 1.0, 1.0));
    material.set_metallic_factor(0.0);
    material.set_roughness_factor(1.0);
    material
}

/// Main-world handle keeping the fallback material loaded.
#[derive(Resource)]
pub struct FallbackMaterial(pub AssetHandle<StandardMaterial>);

/// Render-world id of the fallback material.
#[derive(Resource)]
pub struct RenderFallbackMaterial(pub AssetId);

/// On a render-world primitive entity: no material has claimed its slot yet.
#[derive(Component)]
pub struct NeedsFallbackMaterial;

/// Primitive entities some `MaterialComponent<M>` claimed this frame. Two
/// material types claiming one slot is an authoring error no query can catch,
/// because `RenderMaterialComponent<M>` is a distinct type per `M`.
#[derive(Resource, Default)]
pub struct ClaimedSlots(pub std::collections::HashSet<ecs::Entity>);

impl ClaimedSlots {
    /// Records a claim, returning `true` when another material type already
    /// claimed this entity this frame.
    pub fn claim(&mut self, entity: ecs::Entity) -> bool {
        !self.0.insert(entity)
    }
}

/// Gives every still-unclaimed primitive entity the fallback material.
pub(crate) fn insert_fallback_material(
    unclaimed: Query<(ecs::Entity, &NeedsFallbackMaterial)>,
    fallback: Res<RenderFallbackMaterial>,
    mut claimed: ResMut<ClaimedSlots>,
    mut cmd: CommandQueue,
) {
    for (entity, _) in unclaimed.iter() {
        cmd.insert(
            RenderMaterialComponent::<StandardMaterial>::new(fallback.0),
            entity,
        );
        cmd.remove::<NeedsFallbackMaterial>(entity);
    }

    claimed.0.clear();
}
```

Clearing `ClaimedSlots` here needs no ordering edge: `insert_fallback_material` runs in `Render`, which starts only once all of `Extract` has finished.

`RenderMaterialComponent::new` is `pub(crate)` in `crates/render/src/components/material.rs`; it stays that way since this module is inside the same crate.

Register the module in `crates/render/src/components/mod.rs`:

```rust
pub mod fallback_material;
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p render --test fallback_material`
Expected: PASS (2 tests).

- [ ] **Step 5: Create the asset and register the system**

In `crates/render/src/plugin.rs`, near the existing `app.register_asset::<Mesh>()` block, add a `Startup` system in the main world that creates the material and publishes both resources:

```rust
fn create_fallback_material(asset_server: Res<AssetServer>, mut cmd: CommandQueue) {
    let handle = asset_server.add(fallback_material_asset());
    cmd.insert_resource(RenderFallbackMaterial(handle.id()));
    cmd.insert_resource(FallbackMaterial(handle));
}
```

`RenderFallbackMaterial` must reach the **render** world. Follow whatever pattern `RenderPlugin` already uses to seed render-world resources — check how `SkinUniforms` or `SkeletonLayout` get there and mirror it. If render-world resources are inserted directly at plugin build time, insert `RenderFallbackMaterial` there and keep only `FallbackMaterial` in the `Startup` system.

Register the system:

```rust
.add_render_system(Render, insert_fallback_material)
```

`insert_fallback_material` must be registered by `RenderPlugin`, not `MaterialPlugin<M>` — `MaterialPlugin` is instantiated once per material type and would register it repeatedly. Insert `ClaimedSlots::default()` into the render world alongside `RenderFallbackMaterial`.

- [ ] **Step 6: Tag new instances and clear the tag on claim**

In `crates/render/src/components/mesh.rs`, the newly created instance is inserted alongside the marker:

```rust
cmd.insert(instance, render_entity);
cmd.insert(NeedsFallbackMaterial, render_entity);
```

In `crates/render/src/material_plugin.rs`, `extract_materials::<M>` removes it when it claims the slot, directly after the `cmd.insert(RenderMaterialComponent…)` call:

```rust
cmd.insert(
    RenderMaterialComponent::<M>::new(handle.id()),
    render_entity,
);
cmd.remove::<NeedsFallbackMaterial>(render_entity);
```

- [ ] **Step 7: Verify**

Run: `cargo check --workspace --all-targets && cargo test -p render`
Expected: PASS.

Run: `cargo run -p render-test`
Expected: unchanged rendering — every entity in that example has a material, so nothing should turn magenta. If something does, a `MaterialComponent` site was missed in Task 3.

To confirm the fallback actually fires, temporarily delete the `MaterialComponent` from one spawned entity in `examples/render-test/src/main.rs`, run, see magenta, and revert.

- [ ] **Step 8: Commit**

```bash
git add crates/render
git commit -m "Draw uncovered primitive slots in magenta

A primitive no MaterialComponent covers previously did not draw, which
read as missing geometry. It now gets a magenta StandardMaterial,
applied in the Render schedule after every Extract-stage material claim.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 5: Fan out to one render entity per primitive

The switch. One main entity's `MeshComponent` now produces N render entities, one per primitive of the loaded mesh. This is what makes a multi-primitive `Mesh` actually draw.

**Files:**
- Modify: `crates/render/src/components/mesh.rs`
- Modify: `crates/render/src/material_plugin.rs` (`extract_materials` walks the fan-out)
- Modify: `crates/render/src/components/skeleton.rs:142-163`
- Test: `crates/render/tests/mesh_fanout.rs` (create)

**Interfaces:**
- Consumes: `RenderMeshInstance.primitive` (Task 2), `SlotBinding::slot` (Task 3), `NeedsFallbackMaterial` and `ClaimedSlots::claim` (Task 4).
- Produces:
  - `render::components::mesh::RenderMeshFanout { pub mesh_asset_id: AssetId, pub primitives: Vec<Entity> }` — a `Component` on the owner render entity
  - `render::components::mesh::fanout_is_stale(fanout: Option<&RenderMeshFanout>, mesh_asset_id: AssetId, primitive_count: usize) -> bool` — `pub`, extracted so the rebuild rule is testable without a GPU

Primitive entities carry `MainEntity(main)` so `despawn_stale_render_entities` reclaims them with their main entity; they deliberately do **not** get a `RenderEntity` back-link, which stays 1:1 with the owner mirror.

- [ ] **Step 1: Write the failing test**

Create `crates/render/tests/mesh_fanout.rs`:

```rust
//! Covers when the per-primitive render-entity fan-out must be rebuilt.
use ecs::Entity;
use essential::assets::AssetId;
use render::components::mesh::{fanout_is_stale, RenderMeshFanout};

#[test]
fn a_missing_fanout_is_stale() {
    assert!(fanout_is_stale(None, AssetId::new(), 3));
}

#[test]
fn a_fanout_for_a_different_mesh_is_stale() {
    let fanout = RenderMeshFanout {
        mesh_asset_id: AssetId::new(),
        primitives: vec![Entity::from_raw(1), Entity::from_raw(2)],
    };

    assert!(fanout_is_stale(Some(&fanout), AssetId::new(), 2));
}

#[test]
fn a_fanout_with_the_wrong_primitive_count_is_stale() {
    let id = AssetId::new();
    let fanout = RenderMeshFanout {
        mesh_asset_id: id,
        primitives: vec![Entity::from_raw(1)],
    };

    assert!(fanout_is_stale(Some(&fanout), id, 3));
}

#[test]
fn a_matching_fanout_is_current() {
    let id = AssetId::new();
    let fanout = RenderMeshFanout {
        mesh_asset_id: id,
        primitives: vec![Entity::from_raw(1), Entity::from_raw(2)],
    };

    assert!(!fanout_is_stale(Some(&fanout), id, 2));
}
```

`Entity::from_raw` is a guess at the constructor. Check `crates/ecs/src/entity.rs` for how a test builds a bare `Entity` and use that; if none is public, make the test build entities through a `World::spawn(())`.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p render --test mesh_fanout`
Expected: FAIL to compile — `RenderMeshFanout` not found.

- [ ] **Step 3: Implement the staleness rule**

In `crates/render/src/components/mesh.rs`:

```rust
/// On an owner render entity: the per-primitive entities standing in for its
/// mesh, and which mesh they were built from.
#[derive(Component)]
pub struct RenderMeshFanout {
    pub mesh_asset_id: AssetId,
    pub primitives: Vec<Entity>,
}

/// Whether the fan-out must be rebuilt for this mesh and primitive count.
pub fn fanout_is_stale(
    fanout: Option<&RenderMeshFanout>,
    mesh_asset_id: AssetId,
    primitive_count: usize,
) -> bool {
    match fanout {
        None => true,
        Some(fanout) => {
            fanout.mesh_asset_id != mesh_asset_id || fanout.primitives.len() != primitive_count
        }
    }
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p render --test mesh_fanout`
Expected: PASS (4 tests).

- [ ] **Step 5: Rewrite `extract_meshes`**

Replace the body of `extract_meshes` in `crates/render/src/components/mesh.rs`. It now needs the mesh asset store to learn the primitive count, and skips entities whose mesh has not loaded yet.

```rust
pub(crate) fn extract_meshes(
    meshes: Extracted<
        Query<(
            &MeshComponent,
            &GlobalTransform,
            Option<&SkeletonComponent>,
            &RenderEntity,
        )>,
    >,
    mesh_assets: Extracted<Res<AssetStore<Mesh>>>,
    fanouts: Query<&RenderMeshFanout>,
    render_meshes: Query<&RenderMeshInstance>,
    main_entities: Query<&MainEntity>,
    mut cmd: CommandQueue,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    for (mesh, transform, skeleton, render_entity) in meshes.iter() {
        let owner = **render_entity;

        let Some(mesh_asset) = mesh_assets.get(mesh.handle.id()) else {
            continue;
        };
        let primitive_count = mesh_asset.primitives.len();

        let raw_transform = match skeleton {
            Some(_) => GlobalTransform::new(Mat4::IDENTITY).to_raw(),
            None => transform.to_raw(),
        };

        let fanout = fanouts.get_entity(owner);
        if fanout_is_stale(fanout, mesh.handle.id(), primitive_count) {
            if let Some(stale) = fanout {
                for entity in &stale.primitives {
                    cmd.despawn(*entity);
                }
            }

            let main = **main_entities
                .get_entity(owner)
                .expect("an owner render entity always mirrors a main entity");

            let mut primitives = Vec::with_capacity(primitive_count);
            for primitive in 0..primitive_count as u32 {
                let instance_buffer =
                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("Instance Buffer"),
                        contents: bytemuck::cast_slice(&[raw_transform]),
                        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    });

                let entity = cmd
                    .spawn((
                        MainEntity::new(main),
                        RenderMeshInstance {
                            mesh_asset_id: mesh.handle.id(),
                            primitive,
                            transform: instance_buffer,
                        },
                        NeedsFallbackMaterial,
                    ))
                    .entity();
                primitives.push(entity);
            }

            cmd.insert(
                RenderMeshFanout {
                    mesh_asset_id: mesh.handle.id(),
                    primitives,
                },
                owner,
            );
            continue;
        }

        for entity in &fanout.expect("a current fan-out is present").primitives {
            if let Some(instance) = render_meshes.get_entity(*entity) {
                queue.write_buffer(
                    &instance.transform,
                    0,
                    bytemuck::cast_slice(&[raw_transform]),
                );
            }
        }
    }
}
```

Two things to verify against the real APIs before running:

- `CommandQueue::spawn` returns an `EntityCommandQueue` with an `entity()` accessor (`crates/ecs/src/command.rs:79, 17`). Commands are deferred, so the returned `Entity` must be valid immediately — check `crates/ecs/src/command.rs` and, if `spawn` reserves the id up front, this works as written. If it does not, spawn the primitive entities through direct world access instead of the queue, or store the fan-out in a render-world `Resource` keyed by owner entity.
- The freshly spawned entities are not visible to `render_meshes`/`fanouts` until commands flush, which is why the rebuild branch `continue`s rather than falling through to the transform write.

Add the needed imports: `AssetStore`, `Mesh`, `MainEntity`, `NeedsFallbackMaterial`, `Entity`.

- [ ] **Step 6: Point `extract_materials` at the fan-out**

In `crates/render/src/material_plugin.rs`, `extract_materials::<M>` no longer writes to the owner. It walks the owner's primitive list and claims each slot its binding covers:

```rust
pub(crate) fn extract_materials<M: Material>(
    materials: Extracted<Query<(&MaterialComponent<M>, &RenderEntity)>>,
    fanouts: Query<&RenderMeshFanout>,
    render_meshes: Query<&RenderMeshInstance>,
    render_materials: Query<&RenderMaterialComponent<M>>,
    mut claimed: ResMut<ClaimedSlots>,
    mut cmd: CommandQueue,
) {
    for (material, render_entity) in materials.iter() {
        let Some(fanout) = fanouts.get_entity(**render_entity) else {
            continue;
        };

        for entity in &fanout.primitives {
            if render_materials.get_entity(*entity).is_some() {
                continue;
            }
            let Some(instance) = render_meshes.get_entity(*entity) else {
                continue;
            };
            let Some(handle) = material.binding.slot(instance.primitive) else {
                continue;
            };

            if claimed.claim(*entity) {
                log::warn!(
                    "primitive slot {} is claimed by more than one material type; it will draw once per type",
                    instance.primitive
                );
            }

            cmd.insert(RenderMaterialComponent::<M>::new(handle.id()), *entity);
            cmd.remove::<NeedsFallbackMaterial>(*entity);
        }
    }
}
```

`claimed.claim` writes a resource rather than queueing a command, so it takes effect immediately and the second material type to run sees the first type's claim whatever order the per-type systems happen to execute in.

- [ ] **Step 7: Propagate skeletons to the primitive entities**

In `crates/render/src/components/skeleton.rs`, `extract_skeletons` allocates one skin slot per main entity and writes `RenderSkeletonComponent { offset }` to the owner. Every primitive entity of that owner draws from the same skin, so it needs the same component with the same offset.

Add `fanouts: Query<&RenderMeshFanout>` to the system's parameters, and directly after the `let offset = match render_skeletons.get_entity(render_entity) { … };` block insert:

```rust
if let Some(fanout) = fanouts.get_entity(render_entity) {
    for entity in &fanout.primitives {
        if render_skeletons.get_entity(*entity).is_none() {
            cmd.insert(RenderSkeletonComponent { offset }, *entity);
        }
    }
}
```

The owner keeps its own `RenderSkeletonComponent` — it is what makes the offset allocation idempotent across frames — even though the owner no longer draws.

- [ ] **Step 8: Verify**

The unit tests above cover only the staleness rule, because `extract_meshes` needs a `RenderDevice` and two live worlds. The rest of this task's behaviour — deferring until the asset loads, rebuilding on a handle change, despawning with the main entity — is verified by running the examples. Do not claim those behaviours are tested.

Run: `cargo check --workspace --all-targets && cargo test -p render`
Expected: PASS.

Run: `cargo run -p render-test`
Expected: unchanged rendering. Every asset is still single-primitive at this point, so the fan-out produces exactly one primitive entity per mesh. Watch the first few frames: meshes should appear once their assets load, not stay invisible, which is what a broken deferral would look like.

Run: `cargo run -p physics-test`
Expected: unchanged. This exercises the skinned and collider paths.

- [ ] **Step 9: Commit**

```bash
git add crates/render
git commit -m "Fan out one render entity per mesh primitive

extract_meshes spawns a render entity per primitive of the loaded mesh
and rebuilds them when the handle or primitive count changes. Material
and skeleton extraction follow the fan-out list. Primitive entities carry
MainEntity, so existing stale-mirror cleanup reclaims them.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 6: glTF imports one named asset per mesh

The user-facing payoff: a glTF mesh becomes one `.gasset` named after the source, and the synthesized per-primitive scene nodes disappear.

**Files:**
- Modify: `crates/gltf-loader/src/gltf_importer.rs:196-226, 476-546`
- Test: `crates/gltf-loader/tests/gltf_importer.rs`

**Interfaces:**
- Consumes: `Mesh { primitives }` (Task 1), `MaterialComponent::per_slot` (Task 3), extract-time fan-out (Task 5).
- Produces: sub-asset names of the form `mesh/{name}` or `mesh/{name}.{index}` or `mesh/{index}`, one per glTF mesh.

- [ ] **Step 1: Write the failing tests**

Add to `crates/gltf-loader/tests/gltf_importer.rs`. Read the file's existing helpers first — it already has a way to run the importer over a fixture and inspect `ImportOutputs`; reuse it rather than writing a new harness. Use a fixture with a multi-primitive mesh; if none exists, add one to the crate's test assets. The same goes for `skinned_multi_primitive.gltf`, `unnamed_mesh.gltf` and `duplicate_names.gltf` — check what the crate already ships before adding anything, and match the exact `type_name` string `SerializedComponent` records for `SkeletonComponent` rather than assuming the `contains` check above matches.

```rust
#[test]
fn a_multi_primitive_gltf_mesh_emits_one_named_sub_asset() {
    let outputs = import_fixture("multi_primitive.gltf");

    let meshes: Vec<&str> = outputs
        .sub_assets
        .iter()
        .filter(|sub| sub.type_name == "Mesh")
        .map(|sub| sub.name.as_str())
        .collect();

    assert_eq!(
        meshes,
        vec!["mesh/Barrel"],
        "one sub-asset per glTF mesh, named from the source"
    );
}

#[test]
fn a_mesh_sub_asset_holds_every_primitive_of_its_gltf_mesh() {
    let outputs = import_fixture("multi_primitive.gltf");

    let emitted = outputs
        .sub_assets
        .iter()
        .find(|sub| sub.name == "mesh/Barrel")
        .expect("the named mesh was emitted");
    let mesh: Mesh = bincode::deserialize(&emitted.bytes).expect("deserializes");

    assert_eq!(mesh.primitives.len(), 2);
}

#[test]
fn the_scene_has_no_synthesized_primitive_nodes() {
    let outputs = import_fixture("multi_primitive.gltf");
    let scene = deserialize_scene(&outputs);

    assert!(
        !scene.nodes.iter().any(|node| node.name.contains(".primitive")),
        "primitives are fanned out at extract time, not baked into the scene"
    );
}

#[test]
fn a_skinned_multi_primitive_node_carries_one_skeleton_component() {
    let outputs = import_fixture("skinned_multi_primitive.gltf");
    let scene = deserialize_scene(&outputs);

    let skeleton_components: usize = scene
        .nodes
        .iter()
        .map(|node| {
            node.components
                .iter()
                .filter(|component| component.type_name.contains("SkeletonComponent"))
                .count()
        })
        .sum();

    assert_eq!(
        skeleton_components, 1,
        "the skinned node owns the only SkeletonComponent; primitives no longer get rootless copies"
    );
}

#[test]
fn an_unnamed_gltf_mesh_falls_back_to_its_index() {
    let outputs = import_fixture("unnamed_mesh.gltf");

    assert!(
        outputs
            .sub_assets
            .iter()
            .any(|sub| sub.name == "mesh/0"),
        "an unnamed mesh is addressed by glTF mesh index"
    );
}

#[test]
fn a_duplicate_mesh_name_is_disambiguated_by_index() {
    let outputs = import_fixture("duplicate_names.gltf");

    let meshes: Vec<&str> = outputs
        .sub_assets
        .iter()
        .filter(|sub| sub.type_name == "Mesh")
        .map(|sub| sub.name.as_str())
        .collect();

    assert_eq!(meshes, vec!["mesh/Crate", "mesh/Crate.1"]);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p gltf-loader --test gltf_importer`
Expected: FAIL — one sub-asset per primitive with `mesh/0`, `mesh/1` names.

- [ ] **Step 3: Group primitives and name the asset**

Replace the mesh loop at `crates/gltf-loader/src/gltf_importer.rs:199-220`:

```rust
let mut mesh_names: Vec<String> = Vec::new();
let mut mesh_prims: Vec<Vec<usize>> = Vec::new();
let mut taken_names: HashSet<String> = HashSet::new();
for (mesh_index, gltf_mesh) in document.meshes().enumerate() {
    let base = gltf_mesh
        .name()
        .map(str::to_string)
        .unwrap_or_else(|| mesh_index.to_string());
    let name = if taken_names.insert(base.clone()) {
        base
    } else {
        format!("{base}.{mesh_index}")
    };

    let mut primitives = Vec::new();
    let mut material_slots = Vec::new();
    for gltf_primitive in gltf_mesh.primitives() {
        primitives.push(load_primitive(
            source_path,
            gltf_mesh.name(),
            &buffers,
            &gltf_primitive,
        )?);
        material_slots.push(match gltf_primitive.material().index() {
            Some(material_index) => material_index,
            None => {
                default_material_used = true;
                default_material_index
            }
        });
    }

    ctx.emit(&format!("mesh/{name}"), &Mesh { primitives })?;
    mesh_names.push(name);
    mesh_prims.push(material_slots);
}
```

`mesh_counter` and the `PrimRef` struct are no longer used — delete both. `mesh_prims[mesh_index]` is now the per-slot material index list, in primitive order.

- [ ] **Step 4: Collapse the node walk**

Replace the whole second node-walk loop at `crates/gltf-loader/src/gltf_importer.rs:476-546` — the `match prims.len()` with its `0 / 1 / _` arms and the child-node construction — with a single form that never synthesizes children:

```rust
for gltf_node in document.nodes() {
    let Some(gltf_mesh) = gltf_node.mesh() else {
        continue;
    };
    let node_index = gltf_node.index();
    let mesh_index = gltf_mesh.index();
    let material_slots = &mesh_prims[mesh_index];
    if material_slots.is_empty() {
        continue;
    }

    let mesh_id = ctx.sub_asset_id(&format!("mesh/{}", mesh_names[mesh_index]));
    push_node_component(
        &mut nodes[node_index],
        &MeshComponent {
            handle: AssetHandle::weak(mesh_id),
        },
    )?;
    referenced_assets.push(mesh_id);

    let mut slots = Vec::with_capacity(material_slots.len());
    for material_index in material_slots {
        let material_id = ctx.sub_asset_id(&format!("material/{material_index}"));
        slots.push(Some(AssetHandle::weak(material_id)));
        referenced_assets.push(material_id);
    }
    push_node_component(
        &mut nodes[node_index],
        &MaterialComponent::<StandardMaterial>::per_slot(slots),
    )?;
    push_node_component(&mut nodes[node_index], &SyncWithRenderWorld)?;
}
```

`push_mesh_components` is now unused — delete it, or reduce it to what this loop needs, whichever keeps the file tidier.

The rootless-`SkeletonComponent`-on-children branch goes away entirely with this rewrite. The rooted `SkeletonComponent` on the owning node in the *first* node-walk loop (`gltf_importer.rs:388-408`) stays exactly as it is.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p gltf-loader`
Expected: PASS. Existing tests in this file assert on `mesh/0`-style names and on primitive child nodes; update them to the new grouping rather than working around it. Any test asserting a specific sub-asset count will need its expected number revised.

- [ ] **Step 6: Verify end to end**

Run: `cargo test -p import`
Expected: PASS. `crates/import/tests/import_gltf.rs` asserts on emitted addresses and will need the same updates.

Re-import a real multi-mesh glTF and check the content tree:

```bash
cargo run -p import -- <path/to/a/real.gltf>
ls content/
```

Expected: one `mesh_<Name>.gasset` per glTF mesh, with readable names, instead of a `mesh_N.gasset` per primitive.

Then run an example that loads imported content and confirm it renders with every primitive drawn and correctly materialed.

- [ ] **Step 7: Commit**

```bash
git add crates/gltf-loader crates/import
git commit -m "Import one named asset per glTF mesh

A glTF mesh's primitives group into a single Mesh sub-asset named from
the source, and the scene carries one node with a per-slot
MaterialComponent instead of synthesized primitive children. Skinned
multi-primitive nodes keep a single SkeletonComponent.

Existing content must be re-imported: sub-asset names are the sidecar's
identity keys, so the regrouping reallocates output UUIDs.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 7: OBJ imports one asset per file

An OBJ has a single material library and a flat list of models, which maps onto one `Mesh` with one primitive per model.

**Files:**
- Modify: `crates/obj-loader/src/obj_importer.rs:45-90, 203-250`
- Test: `crates/obj-loader/tests/obj_importer.rs`

**Interfaces:**
- Consumes: `Mesh { primitives }`, `Primitive` (Task 1); `MaterialComponent::all` (Task 3).
- Produces: a single `mesh/0` sub-asset per OBJ source, holding one primitive per tobj model.

- [ ] **Step 1: Write the failing test**

Add to `crates/obj-loader/tests/obj_importer.rs`, reusing the file's existing fixture helper:

```rust
#[test]
fn an_obj_emits_one_mesh_holding_every_model_as_a_primitive() {
    let outputs = import_fixture("multi_model.obj");

    let meshes: Vec<&EmittedSubAsset> = outputs
        .sub_assets
        .iter()
        .filter(|sub| sub.type_name == "Mesh")
        .collect();

    assert_eq!(meshes.len(), 1, "one Mesh asset per OBJ file");
    assert_eq!(meshes[0].name, "mesh/0");

    let mesh: Mesh = bincode::deserialize(&meshes[0].bytes).expect("deserializes");
    assert_eq!(mesh.primitives.len(), 2, "one primitive per tobj model");
}

#[test]
fn an_obj_emits_a_single_scene_node() {
    let outputs = import_fixture("multi_model.obj");
    let scene = deserialize_scene(&outputs);

    assert_eq!(scene.nodes.len(), 1);
}
```

If `multi_model.obj` does not exist in the crate's fixtures, add a minimal one — two `o`-separated cubes sharing one `mtllib`.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p obj-loader`
Expected: FAIL — one `Mesh` per model, one node per model.

- [ ] **Step 3: Change `build_mesh` to build a primitive**

In `crates/obj-loader/src/obj_importer.rs`, rename and reshape:

```rust
fn build_primitive(mesh_data: &tobj::Mesh) -> Primitive {
```

Replace the tail of the function (lines 244-252):

```rust
let mut primitive = Primitive {
    vertices,
    indices: mesh_data.indices.clone(),
};

if requires_normal_computation {
    primitive.compute_normals();
}
primitive.compute_tangents();

primitive
```

The vertex-assembly body above it is unchanged. Import `mesh::primitive::Primitive`.

- [ ] **Step 4: Emit one mesh and one node**

Replace the emit loop and the node loop (`crates/obj-loader/src/obj_importer.rs:45-88`):

```rust
let mtl_stem = import_material(source_path, ctx)?;

let mesh = Mesh {
    primitives: models.iter().map(|model| build_primitive(&model.mesh)).collect(),
};
ctx.emit("mesh/0", &mesh)?;

let mut referenced_assets: Vec<AssetId> = Vec::new();
let mut node = SceneNode {
    name: source_path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "mesh".to_string()),
    children: vec![],
    components: Vec::new(),
};
push_node_component(&mut node, &Transform::default())?;

let mesh_id = ctx.sub_asset_id("mesh/0");
push_node_component(
    &mut node,
    &MeshComponent {
        handle: AssetHandle::weak(mesh_id),
    },
)?;
referenced_assets.push(mesh_id);

if let Some(stem) = mtl_stem.as_ref() {
    let material_id = ctx.sub_asset_id(&format!("material/{stem}"));
    push_node_component(
        &mut node,
        &MaterialComponent::<StandardMaterial>::all(AssetHandle::weak(material_id)),
    )?;
    referenced_assets.push(material_id);
}

push_node_component(&mut node, &SyncWithRenderWorld)?;

ctx.emit(
    "scene",
    &Scene {
        nodes: vec![node],
        referenced_assets,
    },
)?;
```

`SlotBinding::All` is right here: an OBJ has one material for the whole file, so it covers however many primitives the models produce.

Update the module doc comment at the top of the file — it still describes `mesh/*` as multiple sub-assets.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p obj-loader`
Expected: PASS. Existing tests asserting `mesh/0`…`mesh/N` and one node per model need updating to the new grouping.

- [ ] **Step 6: Verify the whole workspace**

Run: `cargo test --workspace`
Expected: PASS.

Run: `cargo run -p render-test && cargo run -p physics-test`
Expected: both render correctly.

- [ ] **Step 7: Commit**

```bash
git add crates/obj-loader
git commit -m "Import an OBJ as one mesh of per-model primitives

An OBJ has a single material library and a flat model list, so it maps
onto one Mesh asset with one primitive per model and one scene node
carrying SlotBinding::All.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Notes for the executor

**Re-importing content.** Task 6 changes glTF sub-asset names, which are the `.import.toml` sidecar's identity keys. Every existing glTF-derived asset gets a new UUID, and scenes referencing the old ones break. This is intended and stated in the spec. After Task 6, re-import every glTF source in the repo's example content and commit the regenerated content tree along with any scene files that referenced the old ids.

**Where the two invariants live.** One render entity draws one primitive with one material — that is what `material_renderpass::<M>` requires, and no task changes it. The main world's invariant is different: one entity holds one `MeshComponent` and any number of differently-typed `MaterialComponent<M>`. Task 5 is the bridge between them.

**If `CommandQueue::spawn` cannot return a usable `Entity`.** Task 5, Step 5 depends on it. Check `crates/ecs/src/command.rs` early — before writing Task 5's implementation — and if deferred spawns do not reserve ids, raise it rather than working around it silently. The fallback is a render-world `Resource` mapping owner entity to primitive entities, populated by a system with direct `&mut World` access.
