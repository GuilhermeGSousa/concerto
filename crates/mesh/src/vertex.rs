#[repr(C)]
#[derive(Copy, Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
/// One mesh vertex, laid out as the renderer's vertex buffer reads it.
pub struct Vertex {
    /// The position in mesh-local space.
    pub pos_coords: [f32; 3],
    /// The texture coordinates, with `[0, 0]` at the top left of the texture.
    pub uv_coords: [f32; 2],
    /// The unit surface normal in mesh-local space.
    pub normal: [f32; 3],
    /// The direction of increasing U in mesh-local space, for normal mapping.
    pub tangent: [f32; 3],
    /// The direction of increasing V in mesh-local space, for normal mapping.
    pub bitangent: [f32; 3],
    /// The indices into the [`Skeleton`](crate::Skeleton)'s bones of the bones that move this
    /// vertex.
    pub bone_indices: [u32; Vertex::MAX_AFFECTED_BONES],
    /// The weight of each bone in `bone_indices`; an unused slot has weight 0.
    pub bone_weights: [f32; Vertex::MAX_AFFECTED_BONES],
}

impl Vertex {
    /// The most bones that can move one vertex.
    pub const MAX_AFFECTED_BONES: usize = 4;
}

// SAFETY: `Vertex` is `repr(C)` and holds only arrays of `f32` and `u32`, all 4-byte aligned, so it has no padding and every bit pattern is valid.
unsafe impl bytemuck::Pod for Vertex {}
// SAFETY: every field is an array of `f32` or `u32`, for which all zeros is a valid value.
unsafe impl bytemuck::Zeroable for Vertex {}
