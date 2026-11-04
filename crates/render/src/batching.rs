use std::ops::Range;

use bytemuck::Pod;
use concerto_ecs::{
    entity::EntityHashMap,
    resource::{ResMut, Resource},
};
use concerto_foundation::transform::GlobalTransformRaw;
use wgpu::{BufferDescriptor, Device, Queue};

use crate::{
    components::mesh::{RenderMeshInstance, RenderMeshInstanceFlags},
    frustum::Frustum,
};

pub(crate) const INITIAL_MESH_INSTANCE_CAPACITY: u32 = 1024;

pub(crate) struct InstanceInputUniformBuffer<T: Pod> {
    values: Vec<T>,
    free_uniform_indices: Vec<u32>,
}

impl<T: Pod> InstanceInputUniformBuffer<T> {
    pub(crate) fn new() -> Self {
        Self {
            values: Vec::new(),
            free_uniform_indices: Vec::new(),
        }
    }

    pub(crate) fn add(&mut self, value: T) -> u32 {
        if let Some(index) = self.free_uniform_indices.pop() {
            self.values[index as usize] = value;
            return index;
        }
        self.values.push(value);
        self.values.len() as u32 - 1
    }

    pub(crate) fn set(&mut self, index: u32, value: T) {
        self.values[index as usize] = value;
    }

    pub(crate) fn get(&self, index: u32) -> T {
        self.values[index as usize]
    }

    pub(crate) fn remove(&mut self, index: u32) {
        self.free_uniform_indices.push(index);
    }
}

#[derive(Resource)]
pub(crate) struct BatchedInstanceBuffers {
    pub(crate) current_input_buffer: InstanceInputUniformBuffer<GlobalTransformRaw>,
}

pub(crate) struct Batch<K> {
    pub(crate) key: K,
    pub(crate) skeleton_offset: u32,
    pub(crate) instances: Range<u32>,
}

pub(crate) struct BinCandidate<'a, K> {
    pub(crate) key: K,
    pub(crate) instance: &'a RenderMeshInstance,
    pub(crate) skeleton_offset: Option<u32>,
}

pub(crate) struct ViewBins<K> {
    pub(crate) values: Vec<GlobalTransformRaw>,
    pub(crate) batches: Vec<Batch<K>>,
}

impl<K> Default for ViewBins<K> {
    fn default() -> Self {
        Self {
            values: Vec::new(),
            batches: Vec::new(),
        }
    }
}

impl<K> ViewBins<K> {
    pub(crate) fn rebuild<'a>(
        &mut self,
        candidates: impl IntoIterator<Item = BinCandidate<'a, K>>,
        frustum: &Frustum,
        input: &InstanceInputUniformBuffer<GlobalTransformRaw>,
    ) -> ViewDrawStats
    where
        K: Ord + Copy,
    {
        self.values.clear();
        self.batches.clear();

        let mut culled = 0;
        let mut visible: Vec<BinCandidate<'a, K>> = candidates
            .into_iter()
            .filter(|candidate| {
                let visible = candidate.instance.is_visible_in(frustum);
                if !visible {
                    culled += 1;
                }
                visible
            })
            .collect();
        visible.sort_by_key(|candidate| {
            (
                candidate
                    .instance
                    .flags
                    .contains(RenderMeshInstanceFlags::SKINNED),
                candidate.key,
            )
        });

        for candidate in visible {
            let index = self.values.len() as u32;
            self.values
                .push(input.get(candidate.instance.current_uniform_index));
            let skinned = candidate
                .instance
                .flags
                .contains(RenderMeshInstanceFlags::SKINNED);
            if !skinned {
                if let Some(batch) = self
                    .batches
                    .last_mut()
                    .filter(|batch| batch.key == candidate.key)
                {
                    batch.instances.end = index + 1;
                    continue;
                }
            }
            self.batches.push(Batch {
                key: candidate.key,
                skeleton_offset: if skinned {
                    candidate.skeleton_offset.unwrap_or(0)
                } else {
                    0
                },
                instances: index..index + 1,
            });
        }

        ViewDrawStats {
            draws: self.batches.len() as u32,
            culled,
        }
    }
}

pub(crate) struct ViewInstanceBuffer {
    buffer: wgpu::Buffer,
    label: &'static str,
}

impl ViewInstanceBuffer {
    const STRIDE: u64 = size_of::<GlobalTransformRaw>() as u64;

    pub(crate) fn new(device: &Device, label: &'static str) -> Self {
        Self {
            buffer: Self::create_buffer(device, label, INITIAL_MESH_INSTANCE_CAPACITY as u64),
            label,
        }
    }

    fn create_buffer(device: &Device, label: &'static str, capacity: u64) -> wgpu::Buffer {
        device.create_buffer(&BufferDescriptor {
            label: Some(label),
            size: capacity * Self::STRIDE,
            usage: wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        })
    }

    pub(crate) fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }

    pub(crate) fn write_buffer(
        &mut self,
        device: &Device,
        queue: &Queue,
        values: &[GlobalTransformRaw],
    ) {
        if values.is_empty() {
            return;
        }
        let len = values.len() as u64;
        let mut capacity = self.buffer.size() / Self::STRIDE;
        if len > capacity {
            while capacity < len {
                capacity *= 2;
            }
            self.buffer = Self::create_buffer(device, self.label, capacity);
        }
        queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(values));
    }
}

pub(crate) struct ViewBatches<K> {
    pub(crate) bins: ViewBins<K>,
    pub(crate) instances: ViewInstanceBuffer,
}

impl<K> ViewBatches<K> {
    pub(crate) fn new(device: &Device, label: &'static str) -> Self {
        Self {
            bins: ViewBins::default(),
            instances: ViewInstanceBuffer::new(device, label),
        }
    }

    pub(crate) fn upload(&mut self, device: &Device, queue: &Queue) {
        self.instances
            .write_buffer(device, queue, &self.bins.values);
    }
}

/// Batches queued and mesh instances culled for one view in the last frame.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct ViewDrawStats {
    pub draws: u32,
    pub culled: u32,
}

/// Batches queued and mesh instances culled for every view in the last frame.
#[derive(Resource, Default)]
pub struct RenderDrawStats {
    pub cameras: EntityHashMap<ViewDrawStats>,
    pub shadow_layers: Vec<ViewDrawStats>,
}

pub(crate) fn clear_draw_stats(mut stats: ResMut<RenderDrawStats>) {
    stats.cameras.clear();
    stats.shadow_layers.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::Gpu;
    use concerto_foundation::{assets::AssetId, transform::GlobalTransform};
    use concerto_mesh::Aabb;
    use glam::{Mat4, Vec3};

    fn frustum() -> Frustum {
        let view = Mat4::look_at_rh(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y);
        let proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_2, 1.0, 0.1, 100.0);
        Frustum::from_clip_from_world(proj * view)
    }

    fn instance(
        input: &mut InstanceInputUniformBuffer<GlobalTransformRaw>,
        translation: Vec3,
        flags: RenderMeshInstanceFlags,
    ) -> RenderMeshInstance {
        let transform = Mat4::from_translation(translation);
        RenderMeshInstance {
            mesh_asset_id: AssetId::new(),
            flags,
            current_uniform_index: input.add(GlobalTransform::new(transform).to_raw()),
            world_aabb: Some(
                Aabb {
                    min: Vec3::splat(-0.5),
                    max: Vec3::splat(0.5),
                }
                .transformed(transform),
            ),
        }
    }

    fn candidate(key: u32, instance: &RenderMeshInstance) -> BinCandidate<'_, u32> {
        BinCandidate {
            key,
            instance,
            skeleton_offset: None,
        }
    }

    fn matrix(value: GlobalTransformRaw) -> Mat4 {
        Mat4::from_cols_slice(bytemuck::cast_slice(&[value]))
    }

    fn batch_keys(bins: &ViewBins<u32>) -> Vec<(u32, Range<u32>)> {
        bins.batches
            .iter()
            .map(|batch| (batch.key, batch.instances.clone()))
            .collect()
    }

    #[test]
    fn instances_sharing_a_key_form_one_batch() {
        let mut input = InstanceInputUniformBuffer::new();
        let instances: Vec<RenderMeshInstance> = (0..3)
            .map(|i| {
                instance(
                    &mut input,
                    Vec3::new(i as f32, 0.0, -10.0),
                    RenderMeshInstanceFlags::empty(),
                )
            })
            .collect();
        let mut bins = ViewBins::default();

        let stats = bins.rebuild(
            instances.iter().map(|instance| candidate(7, instance)),
            &frustum(),
            &input,
        );

        assert_eq!(
            stats,
            ViewDrawStats {
                draws: 1,
                culled: 0
            }
        );
        assert_eq!(batch_keys(&bins), [(7, 0..3)]);
    }

    #[test]
    fn instances_with_different_keys_form_one_batch_each_in_key_order() {
        let mut input = InstanceInputUniformBuffer::new();
        let instances: Vec<RenderMeshInstance> = (0..4)
            .map(|i| {
                instance(
                    &mut input,
                    Vec3::new(i as f32, 0.0, -10.0),
                    RenderMeshInstanceFlags::empty(),
                )
            })
            .collect();
        let keys = [3, 1, 3, 2];
        let mut bins = ViewBins::default();

        let stats = bins.rebuild(
            instances
                .iter()
                .zip(keys)
                .map(|(instance, key)| candidate(key, instance)),
            &frustum(),
            &input,
        );

        assert_eq!(
            stats,
            ViewDrawStats {
                draws: 3,
                culled: 0
            }
        );
        assert_eq!(batch_keys(&bins), [(1, 0..1), (2, 1..2), (3, 2..4)]);
    }

    #[test]
    fn a_skinned_instance_forms_its_own_batch_after_the_unskinned_ones() {
        let mut input = InstanceInputUniformBuffer::new();
        let skinned = [
            instance(&mut input, Vec3::ZERO, RenderMeshInstanceFlags::SKINNED),
            instance(&mut input, Vec3::ZERO, RenderMeshInstanceFlags::SKINNED),
        ];
        let unskinned = instance(
            &mut input,
            Vec3::new(0.0, 0.0, -10.0),
            RenderMeshInstanceFlags::empty(),
        );
        let mut bins = ViewBins::default();

        let stats = bins.rebuild(
            [
                BinCandidate {
                    key: 5,
                    instance: &skinned[0],
                    skeleton_offset: Some(256),
                },
                candidate(9, &unskinned),
                BinCandidate {
                    key: 5,
                    instance: &skinned[1],
                    skeleton_offset: Some(512),
                },
            ],
            &frustum(),
            &input,
        );

        assert_eq!(
            stats,
            ViewDrawStats {
                draws: 3,
                culled: 0
            }
        );
        assert_eq!(batch_keys(&bins), [(9, 0..1), (5, 1..2), (5, 2..3)]);
        assert_eq!(
            bins.batches
                .iter()
                .map(|batch| batch.skeleton_offset)
                .collect::<Vec<_>>(),
            [0, 256, 512]
        );
    }

    #[test]
    fn culled_instances_are_counted_and_not_drawn() {
        let mut input = InstanceInputUniformBuffer::new();
        let instances = [
            instance(
                &mut input,
                Vec3::new(0.0, 0.0, -10.0),
                RenderMeshInstanceFlags::empty(),
            ),
            instance(
                &mut input,
                Vec3::new(0.0, 0.0, 10.0),
                RenderMeshInstanceFlags::empty(),
            ),
            instance(
                &mut input,
                Vec3::new(0.0, 0.0, -500.0),
                RenderMeshInstanceFlags::empty(),
            ),
        ];
        let mut bins = ViewBins::default();

        let stats = bins.rebuild(
            instances.iter().map(|instance| candidate(1, instance)),
            &frustum(),
            &input,
        );

        assert_eq!(
            stats,
            ViewDrawStats {
                draws: 1,
                culled: 2
            }
        );
        assert_eq!(batch_keys(&bins), [(1, 0..1)]);
        assert_eq!(
            matrix(bins.values[0]),
            Mat4::from_translation(Vec3::new(0.0, 0.0, -10.0))
        );
    }

    #[test]
    fn a_batch_holds_its_instances_transforms_in_order() {
        let mut input = InstanceInputUniformBuffer::new();
        let translations = [
            Vec3::new(-1.0, 0.0, -10.0),
            Vec3::new(0.0, 1.0, -10.0),
            Vec3::new(1.0, 0.0, -10.0),
        ];
        let instances: Vec<RenderMeshInstance> = translations
            .iter()
            .map(|&translation| instance(&mut input, translation, RenderMeshInstanceFlags::empty()))
            .collect();
        let mut bins = ViewBins::default();

        bins.rebuild(
            instances.iter().map(|instance| candidate(2, instance)),
            &frustum(),
            &input,
        );

        assert_eq!(
            bins.values.iter().copied().map(matrix).collect::<Vec<_>>(),
            translations.map(Mat4::from_translation)
        );
    }

    fn read_back(gpu: &Gpu, buffer: &wgpu::Buffer, len: usize) -> Vec<Mat4> {
        let size = (len * size_of::<GlobalTransformRaw>()) as u64;
        let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("View Instance Readback Buffer"),
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encoder.copy_buffer_to_buffer(buffer, 0, &readback, 0, size);
        gpu.queue.submit([encoder.finish()]);

        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        gpu.device.poll(wgpu::Maintain::Wait);
        rx.recv().unwrap().unwrap();
        let matrices = bytemuck::cast_slice::<u8, f32>(&slice.get_mapped_range())
            .as_chunks::<16>()
            .0
            .iter()
            .map(Mat4::from_cols_array)
            .collect();
        readback.unmap();
        matrices
    }

    #[test]
    fn a_view_buffer_grows_past_capacity_and_keeps_every_transform() {
        let Some(gpu) = Gpu::request_for_test() else {
            return;
        };
        let mut buffer = ViewInstanceBuffer::new(&gpu.device, "Test View Instances");
        let matrices: Vec<Mat4> = (0..=INITIAL_MESH_INSTANCE_CAPACITY)
            .map(|i| Mat4::from_translation(Vec3::new(i as f32 + 1.0, 0.0, 0.0)))
            .collect();
        let values: Vec<GlobalTransformRaw> = matrices
            .iter()
            .map(|&matrix| GlobalTransform::new(matrix).to_raw())
            .collect();

        buffer.write_buffer(&gpu.device, &gpu.queue, &values);

        assert_eq!(read_back(&gpu, buffer.buffer(), values.len()), matrices);
    }
}
