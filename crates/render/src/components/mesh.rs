use concerto_app::extractor::Extracted;
use concerto_ecs::{
    component::Component,
    query::{
        change_detection::{DetectChanges, Ref},
        Query,
    },
    resource::Res,
    CommandQueue,
};
use concerto_foundation::{assets::AssetId, transform::GlobalTransform};
use concerto_mesh::{mesh::MeshComponent, SkeletonComponent};
use glam::Mat4;
use wgpu::util::DeviceExt;

use crate::{components::render_entity::RenderEntity, device::RenderDevice, queue::RenderQueue};

#[derive(Component)]
pub(crate) struct RenderMeshInstance {
    pub(crate) mesh_asset_id: AssetId,
    pub(crate) transform: wgpu::Buffer,
    skinned: bool,
}

pub(crate) fn extract_meshes(
    meshes: Extracted<
        Query<(
            Ref<MeshComponent>,
            Ref<GlobalTransform>,
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

        let skinned = skeleton.is_some();
        let raw_transform = if skinned {
            GlobalTransform::new(Mat4::IDENTITY).to_raw()
        } else {
            transform.to_raw()
        };

        if let Some(mut render_mesh) = render_meshes.get_entity(render_entity) {
            if render_mesh.mesh_asset_id != mesh.handle.id() {
                render_mesh.mesh_asset_id = mesh.handle.id();
            }
            if mesh.is_added()
                || render_mesh.skinned != skinned
                || (!skinned && transform.is_changed())
            {
                render_mesh.skinned = skinned;
                queue.write_buffer(
                    &render_mesh.transform,
                    0,
                    bytemuck::cast_slice(&[raw_transform]),
                );
            }
            continue;
        }

        let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Instance Buffer"),
            contents: bytemuck::cast_slice(&[raw_transform]),
            usage: wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
        });

        let instance = RenderMeshInstance {
            mesh_asset_id: mesh.handle.id(),
            transform: instance_buffer,
            skinned,
        };

        cmd.insert(instance, render_entity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{draw_target::DrawTarget, gpu::Gpu, plugin::RenderPlugin};
    use concerto_app::{
        main_schedule::MainSchedulePlugin,
        plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
        App,
    };
    use concerto_ecs::Entity;
    use concerto_foundation::{assets::handle::AssetHandle, transform::Transform};
    use concerto_window::plugin::{Window, WindowPlugin};
    use glam::Vec3;

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
        let Some(gpu) = Gpu::request_for_test() else {
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

    fn instance_buffer(app: &mut App, entity: Entity) -> wgpu::Buffer {
        let render_entity = **app
            .main_mut()
            .world_mut()
            .get_component_for_entity::<RenderEntity>(entity)
            .unwrap();
        app.render_mut()
            .world_mut()
            .get_component_for_entity::<RenderMeshInstance>(render_entity)
            .unwrap()
            .transform
            .clone()
    }

    fn uploaded(gpu: &Gpu, buffer: &wgpu::Buffer) -> Mat4 {
        let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Instance Readback Buffer"),
            size: buffer.size(),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encoder.copy_buffer_to_buffer(buffer, 0, &readback, 0, buffer.size());
        gpu.queue.submit([encoder.finish()]);

        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        gpu.device.poll(wgpu::Maintain::Wait);
        rx.recv().unwrap().unwrap();
        let matrix = Mat4::from_cols_slice(bytemuck::cast_slice(&slice.get_mapped_range()));
        readback.unmap();
        matrix
    }

    fn overwrite(gpu: &Gpu, buffer: &wgpu::Buffer, matrix: Mat4) {
        gpu.queue
            .write_buffer(buffer, 0, bytemuck::cast_slice(&matrix.to_cols_array()));
    }

    #[test]
    fn a_moved_mesh_uploads_its_new_transform() {
        let Some(gpu) = Gpu::request_for_test() else {
            return;
        };
        let (mut app, entity) = app(&gpu, AssetId::new());
        app.update();
        app.update();
        let buffer = instance_buffer(&mut app, entity);

        app.main_mut()
            .world_mut()
            .get_component_for_entity_mut::<Transform>(entity)
            .unwrap()
            .translation = Vec3::new(1.0, 2.0, 3.0);
        app.update();

        assert_eq!(
            uploaded(&gpu, &buffer),
            Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0))
        );
    }

    #[test]
    fn an_unmoved_mesh_in_a_hierarchy_is_not_uploaded_again() {
        let Some(gpu) = Gpu::request_for_test() else {
            return;
        };
        let (mut app, entity) = app(&gpu, AssetId::new());
        let world = app.main_mut().world_mut();
        let parent = world.spawn(Transform::from_translation(Vec3::X));
        world.entity_mut(parent).add_child(entity);
        app.update();
        app.update();
        let buffer = instance_buffer(&mut app, entity);
        let sentinel = Mat4::from_scale(Vec3::splat(7.0));

        overwrite(&gpu, &buffer, sentinel);
        app.update();
        app.update();

        assert_eq!(uploaded(&gpu, &buffer), sentinel);
    }

    #[test]
    fn a_mesh_that_gains_a_skeleton_uploads_the_identity() {
        let Some(gpu) = Gpu::request_for_test() else {
            return;
        };
        let (mut app, entity) = app(&gpu, AssetId::new());
        app.main_mut()
            .world_mut()
            .get_component_for_entity_mut::<Transform>(entity)
            .unwrap()
            .translation = Vec3::X;
        app.update();
        app.update();
        let buffer = instance_buffer(&mut app, entity);
        assert_eq!(uploaded(&gpu, &buffer), Mat4::from_translation(Vec3::X));

        app.main_mut()
            .world_mut()
            .insert(SkeletonComponent::default(), entity);
        app.update();

        assert_eq!(uploaded(&gpu, &buffer), Mat4::IDENTITY);
    }

    #[test]
    fn a_mesh_that_loses_its_skeleton_uploads_its_world_transform() {
        let Some(gpu) = Gpu::request_for_test() else {
            return;
        };
        let (mut app, entity) = app(&gpu, AssetId::new());
        let world = app.main_mut().world_mut();
        world.insert(SkeletonComponent::default(), entity);
        world
            .get_component_for_entity_mut::<Transform>(entity)
            .unwrap()
            .translation = Vec3::X;
        app.update();
        app.update();
        let buffer = instance_buffer(&mut app, entity);
        assert_eq!(uploaded(&gpu, &buffer), Mat4::IDENTITY);

        app.main_mut()
            .world_mut()
            .remove_component::<SkeletonComponent>(entity);
        app.update();

        assert_eq!(uploaded(&gpu, &buffer), Mat4::from_translation(Vec3::X));
    }

    #[test]
    fn a_mesh_readded_after_moving_without_one_uploads_where_it_now_is() {
        let Some(gpu) = Gpu::request_for_test() else {
            return;
        };
        let mesh = AssetId::new();
        let (mut app, entity) = app(&gpu, mesh);
        app.update();
        app.update();
        let buffer = instance_buffer(&mut app, entity);

        let world = app.main_mut().world_mut();
        world.remove_component::<MeshComponent>(entity);
        world
            .get_component_for_entity_mut::<Transform>(entity)
            .unwrap()
            .translation = Vec3::Y;
        app.update();
        app.main_mut().world_mut().insert(
            MeshComponent {
                handle: AssetHandle::weak(mesh),
            },
            entity,
        );
        app.update();

        assert_eq!(uploaded(&gpu, &buffer), Mat4::from_translation(Vec3::Y));
    }
}
