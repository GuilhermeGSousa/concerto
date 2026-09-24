pub mod material;
pub mod plugin;

use derive_more::Deref;

use concerto_ecs::resource::Resource;
use concerto_foundation::assets::handle::AssetHandle;
use concerto_render::assets::{mesh::Mesh, vertex::Vertex};

pub(crate) const SKYBOX_VERTICES: [Vertex; 8] = [
    // Front
    Vertex {
        pos_coords: [-1.0, -1.0, 1.0],
        uv_coords: [0.0; 2],
        normal: [0.0; 3],
        tangent: [0.0; 3],
        bitangent: [0.0; 3],
        bone_indices: [0; Vertex::MAX_AFFECTED_BONES],
        bone_weights: [0.0; Vertex::MAX_AFFECTED_BONES],
    }, // 0
    Vertex {
        pos_coords: [1.0, -1.0, 1.0],
        uv_coords: [0.0; 2],
        normal: [0.0; 3],
        tangent: [0.0; 3],
        bitangent: [0.0; 3],
        bone_indices: [0; Vertex::MAX_AFFECTED_BONES],
        bone_weights: [0.0; Vertex::MAX_AFFECTED_BONES],
    }, // 1
    Vertex {
        pos_coords: [1.0, 1.0, 1.0],
        uv_coords: [0.0; 2],
        normal: [0.0; 3],
        tangent: [0.0; 3],
        bitangent: [0.0; 3],
        bone_indices: [0; Vertex::MAX_AFFECTED_BONES],
        bone_weights: [0.0; Vertex::MAX_AFFECTED_BONES],
    }, // 2
    Vertex {
        pos_coords: [-1.0, 1.0, 1.0],
        uv_coords: [0.0; 2],
        normal: [0.0; 3],
        tangent: [0.0; 3],
        bitangent: [0.0; 3],
        bone_indices: [0; Vertex::MAX_AFFECTED_BONES],
        bone_weights: [0.0; Vertex::MAX_AFFECTED_BONES],
    }, // 3
    // Back
    Vertex {
        pos_coords: [-1.0, -1.0, -1.0],
        uv_coords: [0.0; 2],
        normal: [0.0; 3],
        tangent: [0.0; 3],
        bitangent: [0.0; 3],
        bone_indices: [0; Vertex::MAX_AFFECTED_BONES],
        bone_weights: [0.0; Vertex::MAX_AFFECTED_BONES],
    }, // 4
    Vertex {
        pos_coords: [1.0, -1.0, -1.0],
        uv_coords: [0.0; 2],
        normal: [0.0; 3],
        tangent: [0.0; 3],
        bitangent: [0.0; 3],
        bone_indices: [0; Vertex::MAX_AFFECTED_BONES],
        bone_weights: [0.0; Vertex::MAX_AFFECTED_BONES],
    }, // 5
    Vertex {
        pos_coords: [1.0, 1.0, -1.0],
        uv_coords: [0.0; 2],
        normal: [0.0; 3],
        tangent: [0.0; 3],
        bitangent: [0.0; 3],
        bone_indices: [0; Vertex::MAX_AFFECTED_BONES],
        bone_weights: [0.0; Vertex::MAX_AFFECTED_BONES],
    }, // 6
    Vertex {
        pos_coords: [-1.0, 1.0, -1.0],
        uv_coords: [0.0; 2],
        normal: [0.0; 3],
        tangent: [0.0; 3],
        bitangent: [0.0; 3],
        bone_indices: [0; Vertex::MAX_AFFECTED_BONES],
        bone_weights: [0.0; Vertex::MAX_AFFECTED_BONES],
    }, // 7
];

pub const SKYBOX_INDICES: [u32; 36] = [
    // Front
    0, 1, 2, 2, 3, 0, // Right
    1, 5, 6, 6, 2, 1, // Back
    5, 4, 7, 7, 6, 5, // Left
    4, 0, 3, 3, 7, 4, // Top
    3, 2, 6, 6, 7, 3, // Bottom
    4, 5, 1, 1, 0, 4,
];

#[derive(Resource, Deref)]
pub struct SkyboxCube(AssetHandle<Mesh>);
