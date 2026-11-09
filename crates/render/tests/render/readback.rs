use concerto_render::{gpu::Gpu, readback::read_texture, wgpu};

fn gpu() -> Option<Gpu> {
    Gpu::request_for_test()
}

fn pattern(width: u32, height: u32) -> Vec<u8> {
    (0..height)
        .flat_map(|y| {
            (0..width).flat_map(move |x| {
                [
                    x as u8,
                    y as u8,
                    (x * 7 + y * 13) as u8,
                    (255 - x - y) as u8,
                ]
            })
        })
        .collect()
}

#[test]
fn a_texture_whose_width_is_not_a_multiple_of_64_reads_back_exactly() {
    let Some(gpu) = gpu() else {
        return;
    };
    let (width, height) = (100, 37);
    let size = wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let expected = pattern(width, height);
    gpu.queue.write_texture(
        texture.as_image_copy(),
        &expected,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        size,
    );

    let image = read_texture(&gpu, &texture);

    assert_eq!(image.dimensions(), (width, height));
    assert_eq!(image.into_raw(), expected);
}
