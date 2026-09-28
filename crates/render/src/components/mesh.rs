use concerto_app::extractor::Extracted;
use concerto_ecs::{component::Component, query::Query, resource::Res, CommandQueue};
use concerto_foundation::{assets::AssetId, transform::GlobalTransform};
use concerto_mesh::{mesh::MeshComponent, SkeletonComponent};
use glam::Mat4;
use wgpu::util::DeviceExt;

use crate::{components::render_entity::RenderEntity, device::RenderDevice, queue::RenderQueue};

#[derive(Component)]
pub(crate) struct RenderMeshInstance {
    pub(crate) mesh_asset_id: AssetId,
    pub(crate) primitive: u32,
    pub(crate) transform: wgpu::Buffer,
}

pub(crate) fn extract_meshes(
    meshes: Extracted<
        Query<(
            &MeshComponent,
            &GlobalTransform,
            Option<&SkeletonComponent>,
            &RenderEntity,
        )>,
    >,
    render_meshes: Query<&RenderMeshInstance>,
    mut cmd: CommandQueue,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    for (mesh, transform, skeleton, render_entity) in meshes.iter() {
        let render_entity = **render_entity;

        let raw_transform = match skeleton {
            Some(_) => GlobalTransform::new(Mat4::IDENTITY).to_raw(),
            None => transform.to_raw(),
        };

        if let Some(render_mesh) = render_meshes.get_entity(render_entity) {
            queue.write_buffer(
                &render_mesh.transform,
                0,
                bytemuck::cast_slice(&[raw_transform]),
            );
            continue;
        }

        let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Instance Buffer"),
            contents: bytemuck::cast_slice(&[raw_transform]),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });

        let instance = RenderMeshInstance {
            mesh_asset_id: mesh.handle.id(),
            primitive: 0,
            transform: instance_buffer,
        };

        cmd.insert(instance, render_entity);
    }
}
