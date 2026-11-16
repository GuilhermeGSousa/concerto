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
        let view = glam::camera::rh::view::look_at_mat4(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y);
        let proj = glam::camera::rh::proj::directx::perspective(
            std::f32::consts::FRAC_PI_2,
            1.0,
            0.1,
            100.0,
        );
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
            layers: Default::default(),
        }
    }

    fn matrix(value: GlobalTransformRaw) -> Mat4 {
        Mat4::from_cols_slice(bytemuck::cast_slice(&[value]))
    }

    #[derive(Debug, PartialEq)]
    struct Batch {
        key: u32,
        instances: Range<u32>,
        skeleton_offset: u32,
    }

    fn batch(key: u32, instances: Range<u32>) -> Batch {
        Batch {
            key,
            instances,
            skeleton_offset: 0,
        }
    }

    struct Candidate {
        key: u32,
        at: Vec3,
        skeleton_offset: Option<u32>,
    }

    fn at(key: u32, x: f32, y: f32, z: f32) -> Candidate {
        Candidate {
            key,
            at: Vec3::new(x, y, z),
            skeleton_offset: None,
        }
    }

    struct Binning {
        case: &'static str,
        candidates: Vec<Candidate>,
        stats: ViewDrawStats,
        batches: Vec<Batch>,
        drawn: Vec<Vec3>,
    }

    fn stats(draws: u32, culled: u32) -> ViewDrawStats {
        ViewDrawStats { draws, culled }
    }

    #[test]
    fn visible_instances_bin_into_one_batch_per_key_in_key_order_with_skinned_ones_last() {
        let cases = [
            Binning {
                case: "several keys",
                candidates: vec![
                    at(3, 0.0, 0.0, -10.0),
                    at(1, 1.0, 0.0, -10.0),
                    at(3, 2.0, 0.0, -10.0),
                    at(2, 3.0, 0.0, -10.0),
                ],
                stats: stats(3, 0),
                batches: vec![batch(1, 0..1), batch(2, 1..2), batch(3, 2..4)],
                drawn: vec![
                    Vec3::new(1.0, 0.0, -10.0),
                    Vec3::new(3.0, 0.0, -10.0),
                    Vec3::new(0.0, 0.0, -10.0),
                    Vec3::new(2.0, 0.0, -10.0),
                ],
            },
            Binning {
                case: "skinned instances",
                candidates: vec![
                    Candidate {
                        skeleton_offset: Some(256),
                        ..at(5, 0.0, 0.0, 0.0)
                    },
                    at(9, 0.0, 0.0, -10.0),
                    Candidate {
                        skeleton_offset: Some(512),
                        ..at(5, 0.0, 0.0, 0.0)
                    },
                ],
                stats: stats(3, 0),
                batches: vec![
                    batch(9, 0..1),
                    Batch {
                        skeleton_offset: 256,
                        ..batch(5, 1..2)
                    },
                    Batch {
                        skeleton_offset: 512,
                        ..batch(5, 2..3)
                    },
                ],
                drawn: vec![Vec3::new(0.0, 0.0, -10.0), Vec3::ZERO, Vec3::ZERO],
            },
            Binning {
                case: "culled instances",
                candidates: vec![
                    at(1, 0.0, 0.0, -10.0),
                    at(1, 0.0, 0.0, 10.0),
                    at(1, 0.0, 0.0, -500.0),
                ],
                stats: stats(1, 2),
                batches: vec![batch(1, 0..1)],
                drawn: vec![Vec3::new(0.0, 0.0, -10.0)],
            },
        ];
        for case in &cases {
            let mut input = InstanceInputUniformBuffer::new();
            let instances: Vec<RenderMeshInstance> = case
                .candidates
                .iter()
                .map(|candidate| {
                    let flags = if candidate.skeleton_offset.is_some() {
                        RenderMeshInstanceFlags::SKINNED
                    } else {
                        RenderMeshInstanceFlags::empty()
                    };
                    instance(&mut input, candidate.at, flags)
                })
                .collect();
            let mut bins = ViewBins::default();

            let stats = bins.rebuild(
                case.candidates
                    .iter()
                    .zip(&instances)
                    .map(|(candidate, instance)| BinCandidate {
                        key: candidate.key,
                        instance,
                        skeleton_offset: candidate.skeleton_offset,
                    }),
                &frustum(),
                &input,
            );

            assert_eq!(stats, case.stats, "{}", case.case);
            let batches: Vec<Batch> = bins
                .batches
                .iter()
                .map(|batch| Batch {
                    key: batch.key,
                    instances: batch.instances.clone(),
                    skeleton_offset: batch.skeleton_offset,
                })
                .collect();
            assert_eq!(batches, case.batches, "{}", case.case);
            assert_eq!(
                bins.values.iter().copied().map(matrix).collect::<Vec<_>>(),
                case.drawn
                    .iter()
                    .copied()
                    .map(Mat4::from_translation)
                    .collect::<Vec<_>>(),
                "{}",
                case.case
            );
        }
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
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("the device polls");
        rx.recv().unwrap().unwrap();
        let matrices = bytemuck::cast_slice::<u8, f32>(
            &slice
                .get_mapped_range()
                .expect("the mapped range is readable"),
        )
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
