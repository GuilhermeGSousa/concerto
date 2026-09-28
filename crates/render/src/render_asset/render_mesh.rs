use concerto_ecs::resource::Res;
use wgpu::util::DeviceExt;

use crate::{
    assets::mesh::Mesh,
    device::RenderDevice,
    render_asset::{AssetPreparationError, RenderAsset},
};

/// Where one primitive sits in a [`RenderMesh`]'s shared buffers.
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

impl RenderAsset for RenderMesh {
    type SourceAsset = Mesh;

    type PreparationParams = (Res<'static, RenderDevice>,);

    fn prepare_asset(
        source_asset: &Self::SourceAsset,
        params: &mut concerto_ecs::system::input::SystemInputData<Self::PreparationParams>,
    ) -> Result<Self, AssetPreparationError> {
        let (context,) = params;

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

        Ok(RenderMesh {
            vertices,
            indices,
            primitives: primitive_ranges(source_asset),
        })
    }
}
