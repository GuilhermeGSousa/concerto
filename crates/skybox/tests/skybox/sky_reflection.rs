use concerto_app::{
    App,
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
};
use concerto_color::Color;
use concerto_ecs::{component::scene::SceneEntityRef, entity::Entity};
use concerto_foundation::{
    assets::{asset_server::AssetServer, handle::AssetHandle},
    transform::Transform,
};
use concerto_mesh::{Mesh, MeshComponent};
use concerto_render::{
    MaterialComponent, MaterialPlugin,
    assets::material::StandardMaterial,
    components::{camera::Camera, light::Light},
    draw_target::{DrawTarget, SharedTexture},
    gpu::Gpu,
    plugin::RenderPlugin,
    wgpu,
};
use concerto_skybox::{Sky, SkyboxPlugin, Sun};
use concerto_window::plugin::{Window, WindowPlugin};
use glam::{Quat, Vec2, Vec3};

const WIDTH: u32 = 96;
const HEIGHT: u32 = 96;
const FOV_DEGREES: f32 = 70.0;
const ELEVATION_DEGREES: f32 = 20.0;
const TOLERANCE: u8 = 16;

struct Scene {
    app: App,
    shared: SharedTexture,
    gpu: Gpu,
    floor: Entity,
}

struct Shown {
    sky: [u8; 3],
    floor: [u8; 3],
}

fn row(elevation_degrees: f32) -> u32 {
    let offset = elevation_degrees.to_radians().tan() / (FOV_DEGREES / 2.0).to_radians().tan();
    (HEIGHT as f32 / 2.0 * (1.0 - offset)) as u32
}

fn metal(app: &App, roughness: f32) -> AssetHandle<StandardMaterial> {
    let mut material = StandardMaterial::default();
    material.set_metallic_factor(1.0);
    material.set_roughness_factor(roughness);
    app.get_resource::<AssetServer>().unwrap().add(material)
}

impl Scene {
    fn new(gpu: &Gpu) -> Self {
        let shared = SharedTexture::new(wgpu::TextureFormat::Rgba8Unorm);
        let mut app = App::new();
        app.insert_resource(gpu.clone())
            .insert_resource(DrawTarget::Texture(shared.clone()))
            .insert_resource(Window::new((WIDTH, HEIGHT), 1.0));
        app.register_plugin(MainSchedulePlugin)
            .register_plugin(AssetManagerPlugin)
            .register_plugin(TimePlugin)
            .register_plugin(WindowPlugin)
            .register_plugin(TransformPlugin)
            .register_plugin(RenderPlugin)
            .register_plugin(SkyboxPlugin)
            .register_plugin(MaterialPlugin::<StandardMaterial>::new());
        let mesh = app
            .get_resource::<AssetServer>()
            .unwrap()
            .add(Mesh::plane(Vec2::splat(200.0)));
        let mirror = metal(&app, 0.0);
        let world = app.main_mut().world_mut();
        world.register_component::<Transform>();
        let sun = world.spawn((
            Sun::default(),
            Light::directional_light().with_intensity(10.0),
            Transform::from_rotation(Quat::from_rotation_arc(Vec3::Z, Vec3::new(0.0, 0.6, 0.8))),
        ));
        world.spawn((
            Camera {
                clear_color: Color::rgba(0.0, 0.0, 0.0, 1.0),
                ..Camera::perspective(FOV_DEGREES.to_radians(), WIDTH as f32 / HEIGHT as f32)
            },
            Sky::default().with_sun(SceneEntityRef::Entity(sun)),
            Transform::IDENTITY,
        ));
        let floor = world.spawn((
            MeshComponent { handle: mesh },
            MaterialComponent::<StandardMaterial> { handle: mirror },
            Transform::from_translation(Vec3::new(0.0, -1.0, 0.0)),
        ));
        app.finish_plugin_build();
        Self {
            app,
            shared,
            gpu: gpu.clone(),
            floor,
        }
    }

    fn shown(&mut self) -> Shown {
        for _ in 0..30 {
            self.app.update();
        }
        let image = self.gpu.read_texture(self.shared.get().unwrap().texture());
        let rgb = |y: u32| {
            let [r, g, b, _] = image.get_pixel(WIDTH / 2, y).0;
            [r, g, b]
        };
        Shown {
            sky: rgb(row(ELEVATION_DEGREES)),
            floor: rgb(row(-ELEVATION_DEGREES)),
        }
    }
}

fn furthest(a: [u8; 3], b: [u8; 3]) -> u8 {
    a.iter().zip(b).map(|(&a, b)| a.abs_diff(b)).max().unwrap()
}

#[test]
fn a_metal_floor_mirrors_the_sky_it_faces_and_a_rough_one_scatters_it() {
    let Some(gpu) = Gpu::request_for_test() else {
        return;
    };
    let mut scene = Scene::new(&gpu);

    let mirrored = scene.shown();
    assert!(
        mirrored.sky.iter().any(|&channel| channel > 40),
        "the sky is lit: {:?}",
        mirrored.sky
    );
    assert!(
        furthest(mirrored.floor, mirrored.sky) <= TOLERANCE,
        "a mirror shows the sky at the angle it is seen from: {:?} against {:?}",
        mirrored.floor,
        mirrored.sky
    );

    let rough = metal(&scene.app, 1.0);
    let floor = scene.floor;
    scene.app.main_mut().world_mut().insert(
        MaterialComponent::<StandardMaterial> { handle: rough },
        floor,
    );
    let scattered = scene.shown();
    assert!(
        scattered.floor.iter().any(|&channel| channel > 20),
        "a rough metal is still lit by its sky: {:?}",
        scattered.floor
    );
    assert!(
        furthest(scattered.floor, mirrored.floor) > TOLERANCE,
        "and no longer shows one direction of it: {:?} against {:?}",
        scattered.floor,
        mirrored.floor
    );
}
