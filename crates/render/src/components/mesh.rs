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
    render_meshes: Query<&mut RenderMeshInstance>,
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

        if let Some(mut render_mesh) = render_meshes.get_entity(render_entity) {
            if render_mesh.mesh_asset_id != mesh.handle.id() {
                render_mesh.mesh_asset_id = mesh.handle.id();
            }
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
            transform: instance_buffer,
        };

        cmd.insert(instance, render_entity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        components::render_entity::SyncWithRenderWorld, draw_target::DrawTarget, gpu::Gpu,
        plugin::RenderPlugin,
    };
    use concerto_app::{
        main_schedule::MainSchedulePlugin,
        plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
        App,
    };
    use concerto_ecs::Entity;
    use concerto_foundation::{assets::handle::AssetHandle, transform::Transform};
    use concerto_window::plugin::{Window, WindowPlugin};

    fn app(gpu: &Gpu, mesh: AssetId) -> (App, Entity) {
        let mut app = App::new();
        app.insert_resource(gpu.clone())
            .insert_resource(DrawTarget::None)
            .insert_resource(Window::new((64, 48), 1.0));
        app.register_plugin(MainSchedulePlugin)
            .register_plugin(AssetManagerPlugin)
            .register_plugin(TimePlugin)
            .register_plugin(WindowPlugin)
            .register_plugin(TransformPlugin)
            .register_plugin(RenderPlugin);
        let world = app.main_mut().world_mut();
        world.register_component::<Transform>();
        let entity = world.spawn((
            MeshComponent {
                handle: AssetHandle::weak(mesh),
            },
            Transform::IDENTITY,
            SyncWithRenderWorld,
        ));
        app.finish_plugin_build();
        (app, entity)
    }

    fn drawn(app: &mut App) -> Vec<AssetId> {
        let world = app.render_mut().world_mut();
        world
            .query::<&RenderMeshInstance, ()>()
            .iter(world)
            .map(|instance| instance.mesh_asset_id)
            .collect()
    }

    #[test]
    fn swapping_a_mesh_handle_changes_the_mesh_its_render_entity_draws() {
        let Some(gpu) = pollster::block_on(Gpu::request(&Gpu::instance(), None)) else {
            return;
        };
        let (first, second) = (AssetId::new(), AssetId::new());
        let (mut app, entity) = app(&gpu, first);
        app.update();
        assert_eq!(drawn(&mut app), [first]);

        app.main_mut()
            .world_mut()
            .get_component_for_entity_mut::<MeshComponent>(entity)
            .unwrap()
            .handle = AssetHandle::weak(second);
        app.update();

        assert_eq!(drawn(&mut app), [second]);
    }
}
