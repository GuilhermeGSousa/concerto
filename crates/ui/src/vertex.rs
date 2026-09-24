use std::mem;

use concerto_render::assets::vertex::VertexBufferLayout;

pub(crate) const QUAD_INDICES: [u16; 6] = [0, 2, 3, 0, 1, 2];

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct UIVertex {
    pub pos_coords: [f32; 2],
    /// Normalised texture coordinates: (0,0) = top-left, (1,1) = bottom-right.
    pub uv: [f32; 2],
}

impl VertexBufferLayout for UIVertex {
    fn describe() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: mem::size_of::<UIVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x2,
                },
                wgpu::VertexAttribute {
                    offset: mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x2,
                },
            ],
        }
    }
}
