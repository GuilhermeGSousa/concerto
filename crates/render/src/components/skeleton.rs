use std::num::NonZeroU64;

use crate::{
    assets::skeleton::Skeleton, components::render_entity::RenderEntity, device::RenderDevice,
    layouts::SkeletonLayout, queue::RenderQueue,
};
use concerto_app::extractor::Extracted;
use concerto_ecs::{
    command::CommandQueue,
    component::{Component, ComponentLifecycleCallback},
    query::{
        change_detection::{DetectChanges, Ref},
        Query,
    },
    resource::{Res, ResMut, Resource},
};
use concerto_foundation::{assets::asset_store::AssetStore, transform::GlobalTransform};
use concerto_mesh::skeleton::SkeletonComponent;
use encase::UniformBuffer;
use glam::Mat4;
use wgpu::{BindGroupDescriptor, BufferDescriptor, CommandEncoderDescriptor, Device, Queue};

const MAX_SKELETON_BONES: usize = 256;
const BONE_SIZE: usize = size_of::<Mat4>();
const SKIN_STRIDE: u32 = (MAX_SKELETON_BONES * BONE_SIZE) as u32;
const INITIAL_SKIN_CAPACITY: u32 = 8;

// Byte offset of this skin's slot in the shared [`SkinUniforms`] buffer.
pub struct RenderSkeletonComponent {
    pub(crate) offset: u32,
    palette_written: bool,
}

impl Component for RenderSkeletonComponent {
    fn on_remove() -> Option<ComponentLifecycleCallback> {
        Some(|mut world, context| {
            let offset = world
                .get_component_for_entity::<RenderSkeletonComponent>(context.entity)
                .map(|render_skeleton| render_skeleton.offset);

            if let (Some(offset), Some(skins)) = (offset, world.get_resource_mut::<SkinUniforms>())
            {
                skins.free_slot(offset / SKIN_STRIDE);
            }
        })
    }
}

// One shared bone-palette buffer for all skins, bound with a dynamic offset per
// draw. Slot 0 is reserved for an identity palette used by unskinned meshes.
#[derive(Resource)]
pub(crate) struct SkinUniforms {
    buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    free: Vec<u32>,
    next_slot: u32,
    capacity_slots: u32,
}

impl SkinUniforms {
    pub(crate) fn new(device: &Device, layout: &SkeletonLayout, queue: &Queue) -> Self {
        let (buffer, bind_group) = Self::create_buffer(device, layout, INITIAL_SKIN_CAPACITY);
        Self::write_identity_slot(queue, &buffer);

        Self {
            buffer,
            bind_group,
            free: Vec::new(),
            next_slot: 1,
            capacity_slots: INITIAL_SKIN_CAPACITY,
        }
    }

    pub(crate) fn bind_group(&self) -> &wgpu::BindGroup {
        &self.bind_group
    }

    fn create_buffer(
        device: &Device,
        layout: &SkeletonLayout,
        capacity_slots: u32,
    ) -> (wgpu::Buffer, wgpu::BindGroup) {
        let buffer = device.create_buffer(&BufferDescriptor {
            label: Some("Skin Uniforms Buffer"),
            size: capacity_slots as u64 * SKIN_STRIDE as u64,
            usage: wgpu::BufferUsages::UNIFORM
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("Skin Uniforms Bind Group"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buffer,
                    offset: 0,
                    size: Some(NonZeroU64::new(SKIN_STRIDE as u64).unwrap()),
                }),
            }],
        });

        (buffer, bind_group)
    }

    fn write_identity_slot(queue: &Queue, buffer: &wgpu::Buffer) {
        let mut identity = UniformBuffer::new(Vec::new());
        identity
            .write(&[Mat4::IDENTITY; MAX_SKELETON_BONES])
            .unwrap();
        queue.write_buffer(buffer, 0, &identity.into_inner());
    }

    fn alloc_slot(&mut self, device: &Device, layout: &SkeletonLayout, queue: &Queue) -> u32 {
        if let Some(slot) = self.free.pop() {
            return slot;
        }

        if self.next_slot == self.capacity_slots {
            self.capacity_slots *= 2;
            let (buffer, bind_group) = Self::create_buffer(device, layout, self.capacity_slots);
            let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
                label: Some("Skin Uniforms Grow Encoder"),
            });
            encoder.copy_buffer_to_buffer(&self.buffer, 0, &buffer, 0, self.buffer.size());
            queue.submit([encoder.finish()]);
            self.buffer = buffer;
            self.bind_group = bind_group;
        }

        let slot = self.next_slot;
        self.next_slot += 1;
        slot
    }

    fn free_slot(&mut self, slot: u32) {
        self.free.push(slot);
    }
}

/// Extracts every `SkeletonComponent` into its slot of the shared bone-palette buffer, writing the skeleton's bones only when the palette is new, its asset has just arrived, its `SkeletonComponent` changed, or a bone's `GlobalTransform` changed.
pub(crate) fn extract_skeletons(
    skeletons: Extracted<Query<(Ref<SkeletonComponent>, &RenderEntity)>>,
    bone_transforms: Extracted<Query<Ref<GlobalTransform>>>,
    render_skeletons: Query<&mut RenderSkeletonComponent>,
    skeleton_assets: Extracted<Res<AssetStore<Skeleton>>>,
    skeleton_layout: Res<SkeletonLayout>,
    mut skins: ResMut<SkinUniforms>,
    mut cmd: CommandQueue,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    for (skeleton, render_entity) in skeletons.iter() {
        let render_entity = **render_entity;
        let skeleton_asset = skeleton_assets.get(skeleton.skeleton());

        let offset = match render_skeletons.get_entity(render_entity) {
            Some(mut render_skeleton) => {
                let bone_moved = skeleton.bones().iter().any(|bone| {
                    bone.entity()
                        .and_then(|entity| bone_transforms.get_entity(entity))
                        .is_some_and(|transform| transform.is_changed())
                });
                let stale = !render_skeleton.palette_written || skeleton.is_changed() || bone_moved;
                if !stale || skeleton_asset.is_none() {
                    continue;
                }
                render_skeleton.palette_written = true;
                render_skeleton.offset
            }
            None => {
                let offset = skins.alloc_slot(&device, &skeleton_layout, &queue) * SKIN_STRIDE;
                cmd.insert(
                    RenderSkeletonComponent {
                        offset,
                        palette_written: skeleton_asset.is_some(),
                    },
                    render_entity,
                );
                offset
            }
        };

        let Some(skeleton_asset) = skeleton_asset else {
            continue;
        };

        let bone_matrices: Vec<[f32; 16]> = skeleton_asset
            .inverse_bindposes
            .iter()
            .zip(skeleton.bones())
            .take(MAX_SKELETON_BONES)
            .map(|(inverse_bindpose, bone)| {
                bone.entity()
                    .and_then(|entity| bone_transforms.get_entity(entity))
                    .map_or(Mat4::IDENTITY, |bone_transform| {
                        bone_transform.matrix() * *inverse_bindpose
                    })
                    .to_cols_array()
            })
            .collect();

        if bone_matrices.is_empty() {
            continue;
        }
        queue.write_buffer(
            &skins.buffer,
            offset as u64,
            bytemuck::cast_slice(&bone_matrices),
        );
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
    use concerto_ecs::{component::scene::SceneEntityRef, Entity};
    use concerto_foundation::{
        assets::{handle::AssetHandle, AssetId},
        transform::Transform,
    };
    use concerto_mesh::skeleton::JointPose;
    use concerto_window::plugin::{Window, WindowPlugin};
    use glam::Vec3;

    fn app(gpu: &Gpu) -> App {
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
        app.finish_plugin_build();
        app
    }

    fn insert_skeleton_asset(app: &mut App, id: AssetId, bone_count: usize) {
        let skeleton = Skeleton::new(
            vec![Mat4::IDENTITY; bone_count],
            vec![None; bone_count],
            (0..bone_count).map(|bone| format!("bone{bone}")).collect(),
            vec![JointPose::IDENTITY; bone_count],
            vec![Default::default(); bone_count],
        )
        .unwrap();
        app.main_mut()
            .world_mut()
            .get_resource_mut::<AssetStore<Skeleton>>()
            .unwrap()
            .insert(id, skeleton);
    }

    struct SpawnedSkeleton {
        entity: Entity,
        bones: Vec<Entity>,
    }

    fn spawn_skeleton(app: &mut App, asset: AssetId, bones: &[Vec3]) -> SpawnedSkeleton {
        let world = app.main_mut().world_mut();
        let bones: Vec<Entity> = bones
            .iter()
            .map(|&translation| world.spawn(Transform::from_translation(translation)))
            .collect();
        let entity = world.spawn((
            SkeletonComponent::new(
                AssetHandle::weak(asset),
                bones
                    .iter()
                    .map(|&bone| SceneEntityRef::Entity(bone))
                    .collect(),
                None,
            ),
            Transform::default(),
        ));
        SpawnedSkeleton { entity, bones }
    }

    fn move_bone(app: &mut App, bone: Entity, translation: Vec3) {
        app.main_mut()
            .world_mut()
            .get_component_for_entity_mut::<Transform>(bone)
            .unwrap()
            .translation = translation;
    }

    fn offset(app: &App, skeleton: Entity) -> u64 {
        let render_entity = **app
            .main()
            .world()
            .get_component_for_entity::<RenderEntity>(skeleton)
            .unwrap();
        app.render()
            .world()
            .get_component_for_entity::<RenderSkeletonComponent>(render_entity)
            .unwrap()
            .offset as u64
    }

    fn skin_buffer(app: &App) -> wgpu::Buffer {
        app.render()
            .get_resource::<SkinUniforms>()
            .unwrap()
            .buffer
            .clone()
    }

    fn palette(gpu: &Gpu, app: &App, offset: u64, bone_count: usize) -> Vec<Mat4> {
        let size = (bone_count * BONE_SIZE) as u64;
        let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Palette Readback Buffer"),
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encoder.copy_buffer_to_buffer(&skin_buffer(app), offset, &readback, 0, size);
        gpu.queue.submit([encoder.finish()]);

        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        gpu.device.poll(wgpu::Maintain::Wait);
        rx.recv().unwrap().unwrap();
        let matrices = bytemuck::cast_slice::<u8, [f32; 16]>(&slice.get_mapped_range())
            .iter()
            .map(Mat4::from_cols_array)
            .collect();
        readback.unmap();
        matrices
    }

    fn overwrite(gpu: &Gpu, app: &App, offset: u64, matrices: &[Mat4]) {
        let floats: Vec<[f32; 16]> = matrices.iter().map(Mat4::to_cols_array).collect();
        gpu.queue
            .write_buffer(&skin_buffer(app), offset, bytemuck::cast_slice(&floats));
    }

    #[test]
    fn a_moved_bone_reaches_the_palette() {
        let Some(gpu) = Gpu::request_for_test() else {
            return;
        };
        let mut app = app(&gpu);
        let asset = AssetId::new();
        insert_skeleton_asset(&mut app, asset, 2);
        let skeleton = spawn_skeleton(&mut app, asset, &[Vec3::X, Vec3::Y]);
        app.update();
        app.update();
        let offset = offset(&app, skeleton.entity);

        move_bone(&mut app, skeleton.bones[1], Vec3::Z);
        app.update();

        assert_eq!(
            palette(&gpu, &app, offset, 2),
            [
                Mat4::from_translation(Vec3::X),
                Mat4::from_translation(Vec3::Z)
            ]
        );
    }

    #[test]
    fn a_skeleton_whose_bones_do_not_move_is_not_written_again() {
        let Some(gpu) = Gpu::request_for_test() else {
            return;
        };
        let mut app = app(&gpu);
        let asset = AssetId::new();
        insert_skeleton_asset(&mut app, asset, 2);
        let skeleton = spawn_skeleton(&mut app, asset, &[Vec3::X, Vec3::Y]);
        app.update();
        app.update();
        let offset = offset(&app, skeleton.entity);
        let sentinel = [Mat4::from_scale(Vec3::splat(7.0)); 2];

        overwrite(&gpu, &app, offset, &sentinel);
        app.update();
        app.update();

        assert_eq!(palette(&gpu, &app, offset, 2), sentinel);
    }

    #[test]
    fn a_skeleton_whose_asset_arrives_later_writes_its_palette() {
        let Some(gpu) = Gpu::request_for_test() else {
            return;
        };
        let mut app = app(&gpu);
        let asset = AssetId::new();
        let skeleton = spawn_skeleton(&mut app, asset, &[Vec3::X]);
        app.update();
        app.update();
        let offset = offset(&app, skeleton.entity);

        insert_skeleton_asset(&mut app, asset, 1);
        app.update();

        assert_eq!(
            palette(&gpu, &app, offset, 1),
            [Mat4::from_translation(Vec3::X)]
        );
    }

    #[test]
    fn a_palette_write_covers_only_the_skeletons_bones() {
        let Some(gpu) = Gpu::request_for_test() else {
            return;
        };
        let mut app = app(&gpu);
        let asset = AssetId::new();
        insert_skeleton_asset(&mut app, asset, 1);
        let skeleton = spawn_skeleton(&mut app, asset, &[Vec3::X]);
        app.update();
        app.update();
        let offset = offset(&app, skeleton.entity);
        let sentinel = Mat4::from_scale(Vec3::splat(7.0));
        overwrite(&gpu, &app, offset, &[sentinel; 2]);

        move_bone(&mut app, skeleton.bones[0], Vec3::Y);
        app.update();

        assert_eq!(
            palette(&gpu, &app, offset, 2),
            [Mat4::from_translation(Vec3::Y), sentinel]
        );
    }

    #[test]
    fn growing_past_capacity_keeps_earlier_palettes() {
        let Some(gpu) = Gpu::request_for_test() else {
            return;
        };
        let mut app = app(&gpu);
        let asset = AssetId::new();
        insert_skeleton_asset(&mut app, asset, 1);
        let earlier: Vec<(Entity, Vec3)> = (1..INITIAL_SKIN_CAPACITY)
            .map(|slot| {
                let translation = Vec3::new(slot as f32, 0.0, 0.0);
                (
                    spawn_skeleton(&mut app, asset, &[translation]).entity,
                    translation,
                )
            })
            .collect();
        app.update();
        app.update();

        let extra_translation = Vec3::new(0.0, -5.0, 0.0);
        let extra = spawn_skeleton(&mut app, asset, &[extra_translation]).entity;
        app.update();

        assert_eq!(palette(&gpu, &app, 0, 1), [Mat4::IDENTITY]);
        for (entity, translation) in earlier {
            assert_eq!(
                palette(&gpu, &app, offset(&app, entity), 1),
                [Mat4::from_translation(translation)]
            );
        }
        assert_eq!(
            palette(&gpu, &app, offset(&app, extra), 1),
            [Mat4::from_translation(extra_translation)]
        );
    }
}
