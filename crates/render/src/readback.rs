use image::RgbaImage;

use crate::gpu::Gpu;

/// Bytes per row of `width` texels of `format`, rounded up to `COPY_BYTES_PER_ROW_ALIGNMENT`.
pub fn padded_bytes_per_row(width: u32, format: wgpu::TextureFormat) -> u32 {
    let block_copy_size = format
        .block_copy_size(Some(wgpu::TextureAspect::All))
        .unwrap();

    (width * block_copy_size).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
}

/// Copies an RGBA8 texture back from the GPU, blocking until it arrives.
pub fn read_texture(gpu: &Gpu, texture: &wgpu::Texture) -> RgbaImage {
    let size = texture.size();
    let (width, height) = (size.width, size.height);
    let padded_bpr = padded_bytes_per_row(width, texture.format());
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Texture Readback Buffer"),
        size: u64::from(padded_bpr) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Texture Readback"),
        });
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bpr),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit([encoder.finish()]);

    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    gpu.device.poll(wgpu::Maintain::Wait);
    rx.recv()
        .expect("the readback buffer was mapped")
        .expect("the readback buffer maps");

    let row_bytes = (width * 4) as usize;
    let mut pixels = Vec::with_capacity(row_bytes * height as usize);
    {
        let data = slice.get_mapped_range();
        for row in data.chunks_exact(padded_bpr as usize) {
            pixels.extend_from_slice(&row[..row_bytes]);
        }
    }
    buffer.unmap();

    RgbaImage::from_raw(width, height, pixels).expect("the pixels fill the image")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padded_rows_are_aligned_and_hold_the_row() {
        for width in [1u32, 4, 16, 64, 80, 100, 160, 256, 320] {
            let pbr = padded_bytes_per_row(width, wgpu::TextureFormat::Rgba8Unorm);
            assert_eq!(
                pbr % wgpu::COPY_BYTES_PER_ROW_ALIGNMENT,
                0,
                "width={width} pbr={pbr}"
            );
            assert!(
                pbr >= width * 4,
                "padded must be >= unpadded for width={width}"
            );
        }
    }
}
