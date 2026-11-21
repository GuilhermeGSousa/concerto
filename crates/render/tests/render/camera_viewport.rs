use concerto_app::{
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
    App,
};
use concerto_color::Color;
use concerto_ecs::{events::event_channel::EventChannel, Entity};
use concerto_foundation::{assets::asset_server::AssetServer, transform::Transform};
use concerto_mesh::{primitives::cuboid, MeshComponent};
use concerto_render::{
    assets::material::StandardMaterial,
    components::{
        camera::{Camera, Viewport},
        camera_environment::CameraEnvironment,
        visibility::RenderLayers,
    },
    draw_target::{DrawTarget, SharedTexture},
    gpu::Gpu,
    plugin::RenderPlugin,
    readback::read_texture,
    wgpu, MaterialComponent, MaterialPlugin,
};
use concerto_window::{
    events::WindowEvent,
    plugin::{Window, WindowPlugin},
};
use glam::{Vec2, Vec3};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const BLACK: [u8; 4] = [0, 0, 0, 255];

struct Hosted {
    app: App,
    shared: SharedTexture,
    gpu: Gpu,
}

impl Hosted {
    fn new(gpu: &Gpu) -> Self {
        let shared = SharedTexture::new(FORMAT);
        let mut app = App::new();
        app.insert_resource(gpu.clone())
            .insert_resource(DrawTarget::Texture(shared.clone()))
            .insert_resource(Window::new((64, 32), 1.0));
        app.register_plugin(MainSchedulePlugin)
            .register_plugin(AssetManagerPlugin)
            .register_plugin(TimePlugin)
            .register_plugin(WindowPlugin)
            .register_plugin(TransformPlugin)
            .register_plugin(RenderPlugin)
            .register_plugin(MaterialPlugin::<StandardMaterial>::new());
        app.main_mut().world_mut().register_component::<Transform>();
        Self {
            app,
            shared,
            gpu: gpu.clone(),
        }
    }

    fn camera(&mut self, clear_color: Color, viewport: Viewport) -> Entity {
        self.app.main_mut().world_mut().spawn((
            Camera {
                clear_color,
                ..Camera::default()
            }
            .with_viewport(viewport),
            Transform::IDENTITY,
        ))
    }

    fn step(&mut self) {
        for _ in 0..3 {
            self.app.update();
        }
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.app
            .get_resource_mut::<EventChannel<WindowEvent>>()
            .unwrap()
            .push_event(WindowEvent::Resized { width, height });
    }

    fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let published = self.shared.get().expect("a texture was published");
        read_texture(&self.gpu, published.texture())
            .get_pixel(x, y)
            .0
    }
}

fn left_half() -> Viewport {
    Viewport::new(Vec2::ZERO, Vec2::new(0.5, 1.0))
}

fn right_half() -> Viewport {
    Viewport::new(Vec2::new(0.5, 0.0), Vec2::ONE)
}

fn red() -> Color {
    Color::rgba(1.0, 0.0, 0.0, 1.0)
}

fn green() -> Color {
    Color::rgba(0.0, 1.0, 0.0, 1.0)
}

fn blue() -> Color {
    Color::rgba(0.0, 0.0, 1.0, 1.0)
}

#[test]
fn two_halves_keep_their_own_clear_colours() {
    let Some(gpu) = Gpu::request_for_test() else {
        return;
    };
    let mut hosted = Hosted::new(&gpu);
    hosted.camera(red(), left_half());
    hosted.camera(blue(), right_half());
    hosted.app.finish_plugin_build();

    hosted.step();

    assert_eq!(hosted.pixel(8, 16), RED);
    assert_eq!(hosted.pixel(56, 16), BLUE);
}

#[test]
fn the_window_outside_every_viewport_is_black() {
    let Some(gpu) = Gpu::request_for_test() else {
        return;
    };
    let mut hosted = Hosted::new(&gpu);
    hosted.camera(red(), left_half());
    hosted.app.finish_plugin_build();

    hosted.step();

    assert_eq!(hosted.pixel(8, 16), RED);
    assert_eq!(hosted.pixel(56, 16), BLACK);
}

#[test]
fn draws_stay_inside_their_camera_rect() {
    let Some(gpu) = Gpu::request_for_test() else {
        return;
    };
    let mut hosted = Hosted::new(&gpu);
    let camera = hosted.camera(blue(), left_half());
    hosted.app.main_mut().world_mut().insert(
        CameraEnvironment {
            fog: None,
            ambient: Color::rgba(1.0, 1.0, 1.0, 1.0),
        },
        camera,
    );
    let (mesh, material) = {
        let server = hosted.app.get_resource::<AssetServer>().unwrap();
        (
            server.add(cuboid(Vec3::new(10.0, 10.0, 1.0))),
            server.add(StandardMaterial::default()),
        )
    };
    hosted.app.main_mut().world_mut().spawn((
        MeshComponent { handle: mesh },
        MaterialComponent::<StandardMaterial> { handle: material },
        Transform::from_translation(Vec3::new(0.0, 0.0, -3.0)),
    ));
    hosted.app.finish_plugin_build();

    for _ in 0..100 {
        hosted.app.update();
        if hosted.shared.get().is_some() && hosted.pixel(8, 16) != BLUE {
            break;
        }
    }

    assert_ne!(hosted.pixel(8, 16), BLUE);
    assert_eq!(hosted.pixel(56, 16), BLACK);
}

#[test]
fn a_camera_draws_only_the_layers_it_has() {
    let Some(gpu) = Gpu::request_for_test() else {
        return;
    };
    let mut hosted = Hosted::new(&gpu);
    let sees = hosted.camera(blue(), left_half());
    let skips = hosted.camera(green(), right_half());
    let bright = CameraEnvironment {
        fog: None,
        ambient: Color::rgba(1.0, 1.0, 1.0, 1.0),
    };
    let world = hosted.app.main_mut().world_mut();
    world.insert((bright, RenderLayers::DEFAULT.with(1)), sees);
    world.insert(bright, skips);
    let (mesh, material) = {
        let server = hosted.app.get_resource::<AssetServer>().unwrap();
        (
            server.add(cuboid(Vec3::new(10.0, 10.0, 1.0))),
            server.add(StandardMaterial::default()),
        )
    };
    hosted.app.main_mut().world_mut().spawn((
        MeshComponent { handle: mesh },
        MaterialComponent::<StandardMaterial> { handle: material },
        Transform::from_translation(Vec3::new(0.0, 0.0, -3.0)),
        RenderLayers::layer(1),
    ));
    hosted.app.finish_plugin_build();

    for _ in 0..100 {
        hosted.app.update();
        if hosted.shared.get().is_some() && hosted.pixel(8, 16) != BLUE {
            break;
        }
    }

    assert_ne!(
        hosted.pixel(8, 16),
        BLUE,
        "the camera with layer 1 draws the box"
    );
    assert_eq!(
        hosted.pixel(56, 16),
        GREEN,
        "the camera on layer 0 alone sees only its clear colour"
    );
}

#[test]
fn the_split_follows_a_resize() {
    let Some(gpu) = Gpu::request_for_test() else {
        return;
    };
    let mut hosted = Hosted::new(&gpu);
    hosted.camera(red(), left_half());
    hosted.camera(blue(), right_half());
    hosted.app.finish_plugin_build();
    hosted.step();

    hosted.resize(100, 40);
    hosted.step();

    assert_eq!(hosted.pixel(45, 20), RED);
    assert_eq!(hosted.pixel(55, 20), BLUE);
}

#[test]
fn an_empty_viewport_draws_nothing() {
    let Some(gpu) = Gpu::request_for_test() else {
        return;
    };
    let mut hosted = Hosted::new(&gpu);
    hosted.camera(green(), left_half());
    hosted.camera(
        red(),
        Viewport::new(Vec2::new(0.5, 0.0), Vec2::new(0.5, 1.0)),
    );
    hosted.app.finish_plugin_build();

    hosted.step();

    assert_eq!(hosted.pixel(8, 16), GREEN);
    assert_eq!(hosted.pixel(56, 16), BLACK);
}
