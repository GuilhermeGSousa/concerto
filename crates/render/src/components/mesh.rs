use concerto_app::extractor::Extracted;
use concerto_ecs::{component::Component, query::Query, resource::Res, CommandQueue, Entity};
use concerto_foundation::{
    assets::{asset_store::AssetStore, AssetId},
    transform::GlobalTransform,
};
use concerto_mesh::{
    mesh::{Mesh, MeshComponent},
    SkeletonComponent,
};
use glam::Mat4;
use wgpu::util::DeviceExt;

use crate::{
    components::render_entity::{MainEntity, RenderEntity},
    device::RenderDevice,
    queue::RenderQueue,
};

#[derive(Component)]
pub(crate) struct RenderMeshInstance {
    pub(crate) mesh_asset_id: AssetId,
    pub(crate) primitive: u32,
    pub(crate) transform: wgpu::Buffer,
}

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

// Mirrors every `MeshComponent` as one render entity per primitive of its
// loaded mesh, listed in a `RenderMeshFanout` on the owner mirror. Waits for
// the mesh asset to load, and rebuilds the primitive entities when the handle
// or primitive count changes. Primitive entities carry `MainEntity`, so they
// are despawned with their main entity like any other mirror.
pub(crate) fn extract_meshes(
    meshes: Extracted<
        Query<(
            Entity,
            &MeshComponent,
            &GlobalTransform,
            Option<&SkeletonComponent>,
            &RenderEntity,
        )>,
    >,
    mesh_assets: Extracted<Res<AssetStore<Mesh>>>,
    fanouts: Query<&RenderMeshFanout>,
    render_meshes: Query<&RenderMeshInstance>,
    mut cmd: CommandQueue,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    for (main_entity, mesh, transform, skeleton, render_entity) in meshes.iter() {
        let owner = **render_entity;
        let mesh_asset_id = mesh.handle.id();

        let Some(mesh_asset) = mesh_assets.get(&mesh.handle) else {
            continue;
        };
        let primitive_count = mesh_asset.primitives.len();

        let raw_transform = match skeleton {
            Some(_) => GlobalTransform::new(Mat4::IDENTITY).to_raw(),
            None => transform.to_raw(),
        };

        let fanout = fanouts.get_entity(owner);
        if !fanout_is_stale(fanout, mesh_asset_id, primitive_count) {
            for entity in fanout.into_iter().flat_map(|fanout| &fanout.primitives) {
                if let Some(instance) = render_meshes.get_entity(*entity) {
                    queue.write_buffer(
                        &instance.transform,
                        0,
                        bytemuck::cast_slice(&[raw_transform]),
                    );
                }
            }
            continue;
        }

        for entity in fanout.into_iter().flat_map(|fanout| &fanout.primitives) {
            cmd.despawn(*entity);
        }

        let primitives = (0..primitive_count as u32)
            .map(|primitive| {
                let transform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Instance Buffer"),
                    contents: bytemuck::cast_slice(&[raw_transform]),
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                });

                cmd.spawn((
                    MainEntity::new(main_entity),
                    RenderMeshInstance {
                        mesh_asset_id,
                        primitive,
                        transform,
                    },
                ))
                .entity()
            })
            .collect();

        cmd.insert(
            RenderMeshFanout {
                mesh_asset_id,
                primitives,
            },
            owner,
        );
    }
}
