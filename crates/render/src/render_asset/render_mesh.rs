use concerto_ecs::resource::Res;
use wgpu::util::DeviceExt;

use crate::{
    assets::mesh::Mesh,
    device::RenderDevice,
    render_asset::{AssetPreparationError, RenderAsset},
};

pub(crate) struct RenderMesh {
    pub(crate) vertices: wgpu::Buffer,
    pub(crate) indices: wgpu::Buffer,
    pub(crate) index_count: u32,
}

impl RenderAsset for RenderMesh {
    type SourceAsset = Mesh;

    type PreparationParams = (Res<'static, RenderDevice>,);

    fn prepare_asset(
        source_asset: &Self::SourceAsset,
        params: &mut concerto_ecs::system::input::SystemInputData<Self::PreparationParams>,
    ) -> Result<Self, AssetPreparationError> {
        let (context,) = params;

        let vertices = context
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Vertex Buffer"),
                contents: bytemuck::cast_slice(&source_asset.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });

        let indices = context
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Index Buffer"),
                contents: bytemuck::cast_slice(&source_asset.indices),
                usage: wgpu::BufferUsages::INDEX,
            });
        let index_count = source_asset.indices.len() as u32;

        Ok(RenderMesh {
            vertices,
            indices,
            index_count,
        })
    }
}
