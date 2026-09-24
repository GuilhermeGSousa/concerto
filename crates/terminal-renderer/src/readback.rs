use concerto_ecs::{
    resource::{Res, Resource},
    Component, Query, ResMut,
};
use concerto_render::{components::camera::RenderCamera, device::RenderDevice, queue::RenderQueue};

use crate::{
    ascii::{padded_bytes_per_row, pixels_to_ascii_into},
    frame::TerminalFrame,
};

#[derive(Resource)]
pub struct TerminalRenderState {
    pub(crate) staging_buffer: wgpu::Buffer,
    pub(crate) padded_bpr: u32,
    pub width: u32,
    pub height: u32,
}

impl TerminalRenderState {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let padded_bpr = padded_bytes_per_row(width, wgpu::TextureFormat::Rgba8UnormSrgb);
        let staging_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Terminal Readback Staging Buffer"),
            size: (padded_bpr * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        Self {
            staging_buffer,
            padded_bpr,
            width,
            height,
        }
    }
}

#[derive(Component)]
pub struct TerminalOutput;

pub fn readback_terminal_frame(
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    render_cameras: Query<&RenderCamera>,
    state: Res<TerminalRenderState>,
    mut frame: ResMut<TerminalFrame>,
) {
    let render_camera = match render_cameras.iter().next() {
        Some(e) => e,
        None => return,
    };

    let rt = render_camera
                .render_target
                .as_ref()
                .expect("Render target not found on camera — make sure the camera is configured for offscreen rendering");

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("Terminal Readback"),
    });

    encoder.copy_texture_to_buffer(
        rt.texture().as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &state.staging_buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(state.padded_bpr),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d {
            width: state.width,
            height: state.height,
            depth_or_array_layers: 1,
        },
    );

    queue.submit([encoder.finish()]);
    device.poll(wgpu::Maintain::Wait);

    let buffer_slice = state.staging_buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
        tx.send(result).unwrap();
    });
    device.poll(wgpu::Maintain::Wait);
    rx.recv().unwrap().unwrap();

    {
        let data = buffer_slice.get_mapped_range();
        frame.scoped_buffer(|buffer| {
            pixels_to_ascii_into(&data, state.width, state.height, state.padded_bpr, buffer);
        });
    }

    state.staging_buffer.unmap();
}

#[cfg(test)]
mod tests {
    use crate::ascii::{padded_bytes_per_row, pixels_to_ascii_into};

    #[test]
    fn test_headless_gpu_render_produces_output() {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            compatible_surface: None,
            force_fallback_adapter: false,
        }));
        let adapter = match adapter {
            Some(a) => a,
            None => return, // No GPU available, skip
        };

        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))
                .unwrap();

        let width = 16u32;
        let height = 8u32;

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Test RTT"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.8,
                            g: 0.4,
                            b: 0.2,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });
        }

        let pbr = padded_bytes_per_row(width, wgpu::TextureFormat::Rgba8Unorm);
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Test Staging"),
            size: (pbr * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(pbr),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        queue.submit([encoder.finish()]);
        device.poll(wgpu::Maintain::Wait);

        let buffer_slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |r| {
            tx.send(r).unwrap();
        });
        device.poll(wgpu::Maintain::Wait);
        rx.recv().unwrap().unwrap();

        let data = buffer_slice.get_mapped_range();
        let mut ascii = String::new();
        pixels_to_ascii_into(&data, width, height, pbr, &mut ascii);

        assert!(!ascii.is_empty());
        assert!(
            ascii.chars().filter(|&c| c != '\n').any(|c| c != ' '),
            "Expected non-space ASCII output for bright clear color, got: {:?}",
            ascii
        );
    }
}
