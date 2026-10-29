use concerto_app::{
    App,
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
};
use concerto_color::Color;
use concerto_foundation::transform::Transform;
use concerto_render::{
    components::camera::Camera,
    draw_target::{DrawTarget, SharedTexture},
    gpu::Gpu,
    plugin::RenderPlugin,
    wgpu,
};
use concerto_ui::{
    UISharedViewport, material::UIMaterial, node::UINode, plugin::UIPlugin, transform::UIValue,
};
use concerto_window::plugin::{Window, WindowPlugin};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn gpu() -> Option<Gpu> {
    Gpu::request_for_test()
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

fn source(gpu: &Gpu, picture: SharedTexture) -> App {
    let mut app = App::new();
    app.insert_resource(gpu.clone())
        .insert_resource(DrawTarget::Texture(picture))
        .insert_resource(Window::new((64, 64), 1.0));
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(AssetManagerPlugin)
        .register_plugin(TimePlugin)
        .register_plugin(WindowPlugin)
        .register_plugin(TransformPlugin)
        .register_plugin(RenderPlugin);
    app.main_mut().world_mut().register_component::<Transform>();
    app.main_mut().world_mut().spawn((
        Camera {
            clear_color: Color::rgba(1.0, 0.0, 0.0, 1.0),
            ..Default::default()
        },
        Transform::IDENTITY,
    ));
    app.finish_plugin_build();
    app
}

fn viewer(gpu: &Gpu, target: SharedTexture, picture: SharedTexture) -> App {
    let mut app = App::new();
    app.insert_resource(gpu.clone())
        .insert_resource(DrawTarget::Texture(target))
        .insert_resource(Window::new((64, 64), 1.0));
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(AssetManagerPlugin)
        .register_plugin(TimePlugin)
        .register_plugin(WindowPlugin)
        .register_plugin(TransformPlugin)
        .register_plugin(RenderPlugin)
        .register_plugin(UIPlugin);
    let mut viewport = UISharedViewport::new();
    viewport.picture = Some(picture);
    app.main_mut().world_mut().spawn((
        UINode::default().with_size(UIValue::Percent(100.0), UIValue::Percent(100.0)),
        UIMaterial::flat(Color::WHITE),
        viewport,
    ));
    app.finish_plugin_build();
    app
}

#[test]
fn a_shared_viewport_shows_the_other_apps_picture() {
    let Some(gpu) = gpu() else {
        return;
    };
    let picture = SharedTexture::new(FORMAT);
    let screen = SharedTexture::new(FORMAT);
    let mut source = source(&gpu, picture.clone());
    let mut viewer = viewer(&gpu, screen.clone(), picture);

    for _ in 0..3 {
        source.update();
        viewer.update();
    }

    let shown = screen.get().expect("the viewer drew");
    let [red, green, _, _] = first_pixel(&gpu, shown.texture());
    assert!(
        red > 200 && green < 60,
        "expected the source's red, got {red} {green}"
    );
}
