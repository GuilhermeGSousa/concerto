use std::ops::Range;

use concerto_app::extractor::Extracted;
use concerto_ecs::{
    component::Component,
    query::{
        change_detection::{DetectChanges, Ref},
        Query,
    },
    resource::{Res, ResMut, Resource},
    CommandQueue,
};
use concerto_foundation::{
    assets::AssetId,
    transform::{GlobalTransform, GlobalTransformRaw},
};
use concerto_mesh::{mesh::MeshComponent, SkeletonComponent};
use glam::Mat4;
use wgpu::{BufferDescriptor, Device, Queue};

use crate::{components::render_entity::RenderEntity, device::RenderDevice, queue::RenderQueue};

pub(crate) const INITIAL_MESH_INSTANCE_CAPACITY: u32 = 1024;
const INSTANCE_STRIDE: u64 = size_of::<GlobalTransformRaw>() as u64;

#[derive(Resource)]
pub(crate) struct MeshInstances {
    buffer: wgpu::Buffer,
    transforms: Vec<GlobalTransformRaw>,
}

impl MeshInstances {
    pub(crate) fn new(device: &Device) -> Self {
        Self {
            buffer: Self::create_buffer(device, INITIAL_MESH_INSTANCE_CAPACITY as u64),
            transforms: Vec::new(),
        }
    }

    pub(crate) fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }

    fn create_buffer(device: &Device, capacity: u64) -> wgpu::Buffer {
        device.create_buffer(&BufferDescriptor {
            label: Some("Mesh Instances Buffer"),
            size: capacity * INSTANCE_STRIDE,
            usage: wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        })
    }

    fn alloc(&mut self) -> u32 {
        let slot = self.transforms.len() as u32;
        self.transforms.push(bytemuck::Zeroable::zeroed());
        slot
    }

    fn set(&mut self, slot: u32, transform: GlobalTransformRaw) {
        self.transforms[slot as usize] = transform;
    }

    fn upload(&mut self, device: &Device, queue: &Queue, mut span: Range<u32>) {
        let len = self.transforms.len() as u64;
        let mut capacity = self.buffer.size() / INSTANCE_STRIDE;
        if len > capacity {
            while capacity < len {
                capacity *= 2;
            }
            self.buffer = Self::create_buffer(device, capacity);
            span = 0..self.transforms.len() as u32;
        }
        if span.is_empty() {
            return;
        }
        queue.write_buffer(
            &self.buffer,
            span.start as u64 * INSTANCE_STRIDE,
            bytemuck::cast_slice(&self.transforms[span.start as usize..span.end as usize]),
        );
    }
}

#[derive(Component)]
pub(crate) struct RenderMeshInstance {
    pub(crate) mesh_asset_id: AssetId,
    pub(crate) slot: u32,
    skinned: bool,
}

fn widen(span: Option<Range<u32>>, slot: u32) -> Range<u32> {
    match span {
        Some(span) => span.start.min(slot)..span.end.max(slot + 1),
        None => slot..slot + 1,
    }
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
    mut instances: ResMut<MeshInstances>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    let mut span = None;

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
                instances.set(render_mesh.slot, raw_transform);
                span = Some(widen(span, render_mesh.slot));
            }
            continue;
        }

        let slot = instances.alloc();
        instances.set(slot, raw_transform);
        span = Some(widen(span, slot));

        let instance = RenderMeshInstance {
            mesh_asset_id: mesh.handle.id(),
            slot,
            skinned,
        };

        cmd.insert(instance, render_entity);
    }

    instances.upload(&device, &queue, span.unwrap_or(0..0));
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

    fn app(gpu: &Gpu, mesh: AssetId, translations: &[Vec3]) -> (App, Vec<Entity>) {
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
        app.main_mut().world_mut().register_component::<Transform>();
        let entities = translations
            .iter()
            .map(|&translation| spawn_mesh(&mut app, mesh, translation))
            .collect();
        app.finish_plugin_build();
        (app, entities)
    }

    fn spawn_mesh(app: &mut App, mesh: AssetId, translation: Vec3) -> Entity {
        app.main_mut().world_mut().spawn((
            MeshComponent {
                handle: AssetHandle::weak(mesh),
            },
            Transform::from_translation(translation),
        ))
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
        let (mut app, entities) = app(&gpu, first, &[Vec3::ZERO]);
        app.update();
        assert_eq!(drawn(&mut app), [first]);

        app.main_mut()
            .world_mut()
            .get_component_for_entity_mut::<MeshComponent>(entities[0])
            .unwrap()
            .handle = AssetHandle::weak(second);
        app.update();

        assert_eq!(drawn(&mut app), [second]);
    }

    fn slot(app: &mut App, entity: Entity) -> u32 {
        let render_entity = **app
            .main_mut()
            .world_mut()
            .get_component_for_entity::<RenderEntity>(entity)
            .unwrap();
        app.render_mut()
            .world_mut()
            .get_component_for_entity::<RenderMeshInstance>(render_entity)
            .unwrap()
            .slot
    }

    fn instance_buffer(app: &App) -> wgpu::Buffer {
        app.render()
            .get_resource::<MeshInstances>()
            .unwrap()
            .buffer()
            .clone()
    }

    fn uploaded(gpu: &Gpu, app: &App, slot: u32) -> Mat4 {
        let buffer = instance_buffer(app);
        let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Instance Readback Buffer"),
            size: INSTANCE_STRIDE,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encoder.copy_buffer_to_buffer(
            &buffer,
            slot as u64 * INSTANCE_STRIDE,
            &readback,
            0,
            INSTANCE_STRIDE,
        );
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

    fn overwrite(gpu: &Gpu, app: &App, slot: u32, matrix: Mat4) {
        gpu.queue.write_buffer(
            &instance_buffer(app),
            slot as u64 * INSTANCE_STRIDE,
            bytemuck::cast_slice(&matrix.to_cols_array()),
        );
    }

    #[test]
    fn a_moved_mesh_uploads_its_new_transform() {
        let Some(gpu) = Gpu::request_for_test() else {
            return;
        };
        let (mut app, entities) = app(&gpu, AssetId::new(), &[Vec3::ZERO]);
        let entity = entities[0];
        app.update();
        app.update();
        let slot = slot(&mut app, entity);

        app.main_mut()
            .world_mut()
            .get_component_for_entity_mut::<Transform>(entity)
            .unwrap()
            .translation = Vec3::new(1.0, 2.0, 3.0);
        app.update();

        assert_eq!(
            uploaded(&gpu, &app, slot),
            Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0))
        );
    }

    #[test]
    fn an_unmoved_mesh_in_a_hierarchy_is_not_uploaded_again() {
        let Some(gpu) = Gpu::request_for_test() else {
            return;
        };
        let (mut app, entities) = app(&gpu, AssetId::new(), &[Vec3::ZERO]);
        let entity = entities[0];
        let world = app.main_mut().world_mut();
        let parent = world.spawn(Transform::from_translation(Vec3::X));
        world.entity_mut(parent).add_child(entity);
        app.update();
        app.update();
        let slot = slot(&mut app, entity);
        let sentinel = Mat4::from_scale(Vec3::splat(7.0));

        overwrite(&gpu, &app, slot, sentinel);
        app.update();
        app.update();

        assert_eq!(uploaded(&gpu, &app, slot), sentinel);
    }

    #[test]
    fn a_mesh_that_gains_a_skeleton_uploads_the_identity() {
        let Some(gpu) = Gpu::request_for_test() else {
            return;
        };
        let (mut app, entities) = app(&gpu, AssetId::new(), &[Vec3::X]);
        let entity = entities[0];
        app.update();
        app.update();
        let slot = slot(&mut app, entity);
        assert_eq!(uploaded(&gpu, &app, slot), Mat4::from_translation(Vec3::X));

        app.main_mut()
            .world_mut()
            .insert(SkeletonComponent::default(), entity);
        app.update();

        assert_eq!(uploaded(&gpu, &app, slot), Mat4::IDENTITY);
    }

    #[test]
    fn a_mesh_that_loses_its_skeleton_uploads_its_world_transform() {
        let Some(gpu) = Gpu::request_for_test() else {
            return;
        };
        let (mut app, entities) = app(&gpu, AssetId::new(), &[Vec3::X]);
        let entity = entities[0];
        app.main_mut()
            .world_mut()
            .insert(SkeletonComponent::default(), entity);
        app.update();
        app.update();
        let slot = slot(&mut app, entity);
        assert_eq!(uploaded(&gpu, &app, slot), Mat4::IDENTITY);

        app.main_mut()
            .world_mut()
            .remove_component::<SkeletonComponent>(entity);
        app.update();

        assert_eq!(uploaded(&gpu, &app, slot), Mat4::from_translation(Vec3::X));
    }

    #[test]
    fn a_mesh_readded_after_moving_without_one_uploads_where_it_now_is() {
        let Some(gpu) = Gpu::request_for_test() else {
            return;
        };
        let mesh = AssetId::new();
        let (mut app, entities) = app(&gpu, mesh, &[Vec3::ZERO]);
        let entity = entities[0];
        app.update();
        app.update();
        let slot = slot(&mut app, entity);

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

        assert_eq!(uploaded(&gpu, &app, slot), Mat4::from_translation(Vec3::Y));
    }

    #[test]
    fn growing_past_capacity_keeps_earlier_transforms() {
        let Some(gpu) = Gpu::request_for_test() else {
            return;
        };
        let mesh = AssetId::new();
        let translations: Vec<Vec3> = (0..INITIAL_MESH_INSTANCE_CAPACITY)
            .map(|i| Vec3::new(i as f32 + 1.0, 0.0, 0.0))
            .collect();
        let (mut app, entities) = app(&gpu, mesh, &translations);
        app.update();
        app.update();
        let (first, first_translation) = entities
            .iter()
            .zip(&translations)
            .find(|(entity, _)| slot(&mut app, **entity) == 0)
            .map(|(&entity, &translation)| (entity, translation))
            .unwrap();

        let extra_translation = Vec3::new(0.0, -5.0, 0.0);
        let extra = spawn_mesh(&mut app, mesh, extra_translation);
        app.update();

        assert_eq!(slot(&mut app, first), 0);
        assert_eq!(
            uploaded(&gpu, &app, 0),
            Mat4::from_translation(first_translation)
        );
        let extra_slot = slot(&mut app, extra);
        assert_eq!(
            uploaded(&gpu, &app, extra_slot),
            Mat4::from_translation(extra_translation)
        );
    }

    #[test]
    fn a_frame_uploads_only_the_span_of_moved_meshes() {
        let Some(gpu) = Gpu::request_for_test() else {
            return;
        };
        let translations = [Vec3::X, Vec3::Y, Vec3::Z, Vec3::ONE];
        let (mut app, entities) = app(&gpu, AssetId::new(), &translations);
        app.update();
        app.update();
        let mut by_slot: Vec<(u32, Entity, Vec3)> = entities
            .iter()
            .zip(translations)
            .map(|(&entity, translation)| (slot(&mut app, entity), entity, translation))
            .collect();
        by_slot.sort_by_key(|&(slot, _, _)| slot);
        let sentinel = Mat4::from_scale(Vec3::splat(7.0));
        for &(slot, _, _) in &by_slot {
            overwrite(&gpu, &app, slot, sentinel);
        }

        let moved = [Vec3::new(4.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 6.0)];
        for (&(_, entity, _), &translation) in [by_slot[0], by_slot[2]].iter().zip(&moved) {
            app.main_mut()
                .world_mut()
                .get_component_for_entity_mut::<Transform>(entity)
                .unwrap()
                .translation = translation;
        }
        app.update();

        assert_eq!(
            uploaded(&gpu, &app, by_slot[0].0),
            Mat4::from_translation(moved[0])
        );
        assert_eq!(
            uploaded(&gpu, &app, by_slot[2].0),
            Mat4::from_translation(moved[1])
        );
        assert_eq!(
            uploaded(&gpu, &app, by_slot[1].0),
            Mat4::from_translation(by_slot[1].2)
        );
        assert_eq!(uploaded(&gpu, &app, by_slot[3].0), sentinel);
    }
}
