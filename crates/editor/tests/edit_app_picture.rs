//! An edit `App` built from `DefaultPlugins` draws into the picture its host shows.
use concerto_app::App;
use concerto_editor::{
    hosted::{HostRequest, HostRole, begin},
    tools::EditToolsPlugin,
};
use concerto_render::{gpu::Gpu, wgpu};
use concerto_window::events::WindowEvent;

fn game(app: &mut App) {
    app.register_plugin(concerto::DefaultPlugins);
}

fn gpu() -> Option<Gpu> {
    pollster::block_on(Gpu::request(&Gpu::instance(), None))
}

fn first_pixel(gpu: &Gpu, texture: &wgpu::Texture) -> [u8; 4] {
    let size = texture.size();
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(size.width * size.height * 4),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size.width * 4),
                rows_per_image: Some(size.height),
            },
        },
        size,
    );
    gpu.queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |result| result.unwrap());
    gpu.device.poll(wgpu::Maintain::Wait);
    let data = slice.get_mapped_range();
    [data[0], data[1], data[2], data[3]]
}

#[test]
fn an_edit_app_draws_into_its_picture_and_follows_a_resize() {
    let Some(gpu) = gpu() else {
        return;
    };
    let pending = begin(
        HostRequest {
            gpu: Some(&gpu),
            recipe: game,
            size: (64, 48),
            scale_factor: 1.0,
            role: HostRole::Edit,
            clipboard: None,
        },
        |app| {
            app.register_plugin(EditToolsPlugin);
        },
        |_| {},
    );
    let mut hosted = concerto_editor::hosted::finish_blocking(pending);

    for _ in 0..3 {
        hosted.step_for_test();
    }
    let drawn = hosted.picture().get().expect("the edit App drew");
    let size = drawn.texture().size();
    assert_eq!((size.width, size.height), (64, 48));
    assert_ne!(
        first_pixel(&gpu, drawn.texture())[3],
        0,
        "the picture is not blank"
    );

    hosted.push_for_test(WindowEvent::Resized {
        width: 128,
        height: 32,
    });
    for _ in 0..3 {
        hosted.step_for_test();
    }
    let size = hosted.picture().get().unwrap().texture().size();
    assert_eq!((size.width, size.height), (128, 32));
}
