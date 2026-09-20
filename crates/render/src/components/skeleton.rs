use std::num::NonZeroU64;

use crate::{
    assets::skeleton::Skeleton, components::render_entity::RenderEntity, device::RenderDevice,
    layouts::SkeletonLayout, queue::RenderQueue,
};
use app::extractor::Extracted;
use ecs::{
    command::CommandQueue,
    component::{Component, ComponentLifecycleCallback},
    query::Query,
    resource::{Res, ResMut, Resource},
};
use encase::UniformBuffer;
use essential::{assets::asset_store::AssetStore, transform::GlobalTransform};
use glam::Mat4;
use mesh::skeleton::SkeletonComponent;
use wgpu::{BindGroupDescriptor, BufferDescriptor, Device, Queue};

const MAX_SKELETON_BONES: usize = 256;
const BONE_SIZE: usize = size_of::<Mat4>();
const SKIN_STRIDE: u32 = (MAX_SKELETON_BONES * BONE_SIZE) as u32;
const INITIAL_SKIN_CAPACITY: u32 = 8;

// Byte offset of this skin's slot in the shared [`SkinUniforms`] buffer.
pub struct RenderSkeletonComponent {
    pub(crate) offset: u32,
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
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
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
            // Existing byte offsets stay valid; skin palettes are rewritten every
            // frame by update_skeletons, so only the identity slot needs restoring.
            self.capacity_slots *= 2;
            let (buffer, bind_group) = Self::create_buffer(device, layout, self.capacity_slots);
            Self::write_identity_slot(queue, &buffer);
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

/// Extracts every `SkeletonComponent` into its GPU-side bone palette,
/// upserting like the other extract systems: allocates a `SkinUniforms` slot
/// only the first time a render mirror is seen (recorded via
/// `RenderSkeletonComponent`, inserted through `CommandQueue`), then writes
/// the current bone matrices to that slot every frame unconditionally —
/// bones are separate main-world entities, so their `GlobalTransform` needs
/// the same unconditional read as the other extract systems' transforms.
pub(crate) fn extract_skeletons(
    skeletons: Extracted<Query<(&SkeletonComponent, &RenderEntity)>>,
    bone_transforms: Extracted<Query<&GlobalTransform>>,
    render_skeletons: Query<&RenderSkeletonComponent>,
    skeleton_assets: Extracted<Res<AssetStore<Skeleton>>>,
    skeleton_layout: Res<SkeletonLayout>,
    mut skins: ResMut<SkinUniforms>,
    mut cmd: CommandQueue,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    for (skeleton, render_entity) in skeletons.iter() {
        let render_entity = **render_entity;

        let offset = match render_skeletons.get_entity(render_entity) {
            Some(render_skeleton) => render_skeleton.offset,
            None => {
                let offset = skins.alloc_slot(&device, &skeleton_layout, &queue) * SKIN_STRIDE;
                cmd.insert(RenderSkeletonComponent { offset }, render_entity);
                offset
            }
        };

        let Some(skeleton_asset) = skeleton_assets.get(skeleton.skeleton()) else {
            continue;
        };

        let mut bone_matrices = [Mat4::IDENTITY; MAX_SKELETON_BONES];

        for (bone_index, (inverse_bindpose, bone)) in skeleton_asset
            .inverse_bindposes
            .iter()
            .zip(skeleton.bones())
            .enumerate()
        {
            let transform = match bone
                .entity()
                .and_then(|entity| bone_transforms.get_entity(entity))
            {
                Some(bone_transform) => bone_transform.matrix() * *inverse_bindpose,
                None => Mat4::IDENTITY,
            };

            bone_matrices[bone_index] = transform;
        }

        let mut buffer = UniformBuffer::new(Vec::new());
        buffer.write(&bone_matrices).unwrap();
        queue.write_buffer(&skins.buffer, offset as u64, &buffer.into_inner());
    }
}
