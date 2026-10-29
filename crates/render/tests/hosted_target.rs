use concerto_app::{
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
    App,
};
use concerto_color::Color;
use concerto_ecs::events::event_channel::EventChannel;
use concerto_foundation::transform::Transform;
use concerto_render::{
    components::camera::Camera,
    draw_target::{DrawTarget, SharedTexture},
    gpu::Gpu,
    plugin::RenderPlugin,
    wgpu,
};
use concerto_window::{
    events::WindowEvent,
    plugin::{Window, WindowPlugin},
};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn gpu() -> Option<Gpu> {
    pollster::block_on(Gpu::request(&Gpu::instance(), None))
}

fn hosted(gpu: &Gpu, target: DrawTarget, size: (u32, u32)) -> App {
    let mut app = App::new();
    app.insert_resource(gpu.clone())
        .insert_resource(target)
        .insert_resource(Window::new(size, 1.0));
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

fn step(app: &mut App) {
    for _ in 0..3 {
        app.update();
    }
}

fn resize(app: &mut App, width: u32, height: u32) {
    app.get_resource_mut::<EventChannel<WindowEvent>>()
        .unwrap()
        .push_event(WindowEvent::Resized { width, height });
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
fn a_hosted_app_publishes_a_texture_of_the_clear_colour() {
    let Some(gpu) = gpu() else {
        return;
    };
    let shared = SharedTexture::new(FORMAT);
    let mut app = hosted(&gpu, DrawTarget::Texture(shared.clone()), (64, 48));

    step(&mut app);

    let published = shared.get().expect("a texture was published");
    let size = published.texture().size();
    assert_eq!((size.width, size.height), (64, 48));
    assert_eq!(first_pixel(&gpu, published.texture()), [255, 0, 0, 255]);
}

#[test]
fn a_resize_event_publishes_a_texture_of_the_new_size() {
    let Some(gpu) = gpu() else {
        return;
    };
    let shared = SharedTexture::new(FORMAT);
    let mut app = hosted(&gpu, DrawTarget::Texture(shared.clone()), (64, 48));
    step(&mut app);

    resize(&mut app, 128, 32);
    step(&mut app);

    let published = shared.get().expect("a texture was published");
    let size = published.texture().size();
    assert_eq!((size.width, size.height), (128, 32));
    assert_eq!(first_pixel(&gpu, published.texture()), [255, 0, 0, 255]);
}

#[test]
fn a_zero_size_keeps_the_previous_texture() {
    let Some(gpu) = gpu() else {
        return;
    };
    let shared = SharedTexture::new(FORMAT);
    let mut app = hosted(&gpu, DrawTarget::Texture(shared.clone()), (64, 48));
    step(&mut app);

    resize(&mut app, 0, 0);
    step(&mut app);

    let size = shared.get().expect("the texture stays").texture().size();
    assert_eq!((size.width, size.height), (64, 48));
}

#[test]
fn no_texture_is_published_before_the_app_has_a_size() {
    let Some(gpu) = gpu() else {
        return;
    };
    let shared = SharedTexture::new(FORMAT);
    let mut app = hosted(&gpu, DrawTarget::Texture(shared.clone()), (0, 0));

    step(&mut app);

    assert!(shared.get().is_none());
}

#[test]
fn an_app_with_no_target_steps() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut app = hosted(&gpu, DrawTarget::None, (64, 48));

    step(&mut app);
}

#[test]
#[should_panic(expected = "RenderPlugin needs a Gpu resource")]
fn an_app_without_a_gpu_is_refused() {
    let mut app = App::new();
    app.register_plugin(RenderPlugin);
}
