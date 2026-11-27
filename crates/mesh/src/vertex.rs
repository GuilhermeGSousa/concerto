#[repr(C)]
#[derive(Copy, Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Vertex {
    pub pos_coords: [f32; 3],
    pub uv_coords: [f32; 2],
    pub normal: [f32; 3],
    pub tangent: [f32; 3],
    pub bitangent: [f32; 3],
    pub bone_indices: [u32; Vertex::MAX_AFFECTED_BONES],
    pub bone_weights: [f32; Vertex::MAX_AFFECTED_BONES],
}

impl Vertex {
    pub const MAX_AFFECTED_BONES: usize = 4;
}

// SAFETY: `Vertex` is `repr(C)` and holds only arrays of `f32` and `u32`, all 4-byte aligned, so it has no padding and every bit pattern is valid.
unsafe impl bytemuck::Pod for Vertex {}
// SAFETY: every field is an array of `f32` or `u32`, for which all zeros is a valid value.
unsafe impl bytemuck::Zeroable for Vertex {}
