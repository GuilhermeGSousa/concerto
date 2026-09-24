use bytemuck::{Pod, Zeroable};
use concerto_color::Color;
use glam::Vec3;

/// A single vertex of a gizmo line list.
///
/// Gizmos are always drawn as a `line_list`, so vertices come in pairs (one
/// segment per two vertices).  Each vertex carries its own colour, which lets a
/// single draw call render lines of arbitrary colours and even per-segment
/// gradients.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GizmoVertex {
    pub position: [f32; 3],
    pub color: [f32; 4],
}

impl GizmoVertex {
    #[inline]
    pub fn new(position: Vec3, color: Color) -> Self {
        Self {
            position: position.to_array(),
            color: color.to_array(),
        }
    }

    /// The wgpu vertex-buffer layout matching the `gizmo.wgsl` vertex inputs.
    pub fn describe() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GizmoVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute {
                    offset: std::mem::size_of::<[f32; 3]>() as wgpu::BufferAddress,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x4,
                },
            ],
        }
    }
}
