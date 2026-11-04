use std::ops::Range;

use bytemuck::Pod;
use concerto_ecs::resource::Resource;
use concerto_foundation::transform::GlobalTransformRaw;
use wgpu::{BufferDescriptor, Device, Queue};

pub(crate) const INITIAL_MESH_INSTANCE_CAPACITY: u32 = 1024;

pub(crate) struct InstanceInputUniformBuffer<T: Pod> {
    values: Vec<T>,
    free_uniform_indices: Vec<u32>,
    buffer: wgpu::Buffer,
    label: &'static str,
}

impl<T: Pod> InstanceInputUniformBuffer<T> {
    const STRIDE: u64 = size_of::<T>() as u64;

    pub(crate) fn new(device: &Device, label: &'static str, initial_capacity: u32) -> Self {
        Self {
            values: Vec::new(),
            free_uniform_indices: Vec::new(),
            buffer: Self::create_buffer(device, label, initial_capacity as u64),
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

    pub(crate) fn remove(&mut self, index: u32) {
        self.free_uniform_indices.push(index);
    }

    pub(crate) fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }

    pub(crate) fn write_buffer(&mut self, device: &Device, queue: &Queue, mut span: Range<u32>) {
        let len = self.values.len() as u64;
        let mut capacity = self.buffer.size() / Self::STRIDE;
        if len > capacity {
            while capacity < len {
                capacity *= 2;
            }
            self.buffer = Self::create_buffer(device, self.label, capacity);
            span = 0..self.values.len() as u32;
        }
        if span.is_empty() {
            return;
        }
        queue.write_buffer(
            &self.buffer,
            span.start as u64 * Self::STRIDE,
            bytemuck::cast_slice(&self.values[span.start as usize..span.end as usize]),
        );
    }
}

#[derive(Resource)]
pub(crate) struct BatchedInstanceBuffers {
    pub(crate) current_input_buffer: InstanceInputUniformBuffer<GlobalTransformRaw>,
}
