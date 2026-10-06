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

/// One corner of the screen-space quad a wide line segment is drawn as.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct WideGizmoVertex {
    pub start: [f32; 3],
    pub end: [f32; 3],
    pub color: [f32; 4],
    /// Which endpoint this corner sits at (0 or 1) and which side of the line (-1 or 1).
    pub corner: [f32; 2],
    pub width: f32,
}

impl WideGizmoVertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
        0 => Float32x3,
        1 => Float32x3,
        2 => Float32x4,
        3 => Float32x2,
        4 => Float32,
    ];

    pub fn describe() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<WideGizmoVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}
