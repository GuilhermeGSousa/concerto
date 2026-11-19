use concerto_app::{
    App,
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
};
use concerto_color::Color;
use concerto_ecs::entity::Entity;
use concerto_foundation::{assets::asset_server::AssetServer, transform::Transform};
use concerto_mesh::{MeshComponent, primitives::cuboid};
use concerto_render::{
    MaterialComponent, MaterialPlugin,
    assets::material::StandardMaterial,
    components::{
        camera::Camera,
        camera_environment::{CameraEnvironment, Fog},
        light::Light,
    },
    draw_target::{DrawTarget, SharedTexture},
    gpu::Gpu,
    plugin::RenderPlugin,
    readback::read_texture,
    wgpu,
};
use concerto_skybox::{Sky, SkyboxPlugin, Sun};
use concerto_window::plugin::{Window, WindowPlugin};
use glam::{Quat, Vec3, Vec4Swizzles};

const WIDTH: u32 = 128;
const HEIGHT: u32 = 96;
const CAMERA_PITCH_DEGREES: f32 = 25.0;
const FAR_CUBE_AZIMUTH_DEGREES: f32 = -30.0;

struct Pixel {
    rgb: Vec3,
}

impl Pixel {
    fn is_blue(&self) -> bool {
        self.rgb.z > self.rgb.x && self.rgb.z > self.rgb.y
    }

    fn is_red(&self) -> bool {
        self.rgb.x > self.rgb.y && self.rgb.x > self.rgb.z
    }
}

struct SkyScene {
    app: App,
    shared: SharedTexture,
    gpu: Gpu,
    camera: Entity,
    sun: Entity,
}

fn toward(elevation_degrees: f32, azimuth_degrees: f32) -> Vec3 {
    let elevation = elevation_degrees.to_radians();
    let azimuth = azimuth_degrees.to_radians();
    Vec3::new(
        azimuth.sin() * elevation.cos(),
        elevation.sin(),
        -azimuth.cos() * elevation.cos(),
    )
}

fn sun_toward(direction: Vec3) -> Transform {
    Transform::from_rotation(Quat::from_rotation_arc(Vec3::Z, direction))
}

impl SkyScene {
    fn new(gpu: &Gpu, sun_elevation_degrees: f32) -> Self {
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
        let (mesh, material) = {
            let server = app.get_resource::<AssetServer>().unwrap();
            (
                server.add(cuboid(Vec3::ONE)),
                server.add(
                    StandardMaterial::default()
                        .with_base_color_factor(Color::rgba(1.0, 0.0, 0.0, 1.0)),
                ),
            )
        };
        let world = app.main_mut().world_mut();
        world.register_component::<Transform>();
        let camera = world.spawn((
            Camera {
                clear_color: Color::rgba(0.0, 0.0, 0.0, 1.0),
                ..Camera::perspective(70f32.to_radians(), WIDTH as f32 / HEIGHT as f32)
            },
            Sky::default().with_night_ambient(Color::rgba(0.1, 0.1, 0.1, 1.0)),
            CameraEnvironment::default()
                .with_ambient(Color::rgba(1.0, 1.0, 1.0, 1.0))
                .with_fog(Fog {
                    color: Color::rgba(1.0, 0.0, 0.0, 1.0),
                    density: 0.1,
                    start: 10.0,
                }),
            Transform::from_rotation(Quat::from_rotation_x(CAMERA_PITCH_DEGREES.to_radians())),
        ));
        let sun = world.spawn((
            Sun::default().with_disk_radius(2.0),
            Light::directional_light().with_intensity(10.0),
            sun_toward(toward(sun_elevation_degrees, 0.0)),
        ));
        world.spawn((
            MeshComponent {
                handle: mesh.clone(),
            },
            MaterialComponent::<StandardMaterial> {
                handle: material.clone(),
            },
            Transform::from_translation(Vec3::new(0.0, 0.0, -4.0)),
        ));
        world.spawn((
            MeshComponent { handle: mesh },
            MaterialComponent::<StandardMaterial> { handle: material },
            Transform::from_translation(toward(0.0, FAR_CUBE_AZIMUTH_DEGREES) * 60.0)
                .with_scale(Vec3::splat(12.0)),
        ));
        app.finish_plugin_build();
        let mut scene = Self {
            app,
            shared,
            gpu: gpu.clone(),
            camera,
            sun,
        };
        scene.settle();
        scene
    }

    fn settle(&mut self) {
        for _ in 0..100 {
            self.app.update();
            if self.shared.get().is_some() && self.pixel_toward(Vec3::NEG_Z).is_red() {
                break;
            }
        }
        for _ in 0..3 {
            self.app.update();
        }
    }

    fn turn_sun(&mut self, elevation_degrees: f32, azimuth_degrees: f32) {
        *self
            .app
            .main_mut()
            .world_mut()
            .get_component_for_entity_mut::<Transform>(self.sun)
            .unwrap() = sun_toward(toward(elevation_degrees, azimuth_degrees));
        for _ in 0..3 {
            self.app.update();
        }
    }

    fn pixel_toward(&mut self, direction: Vec3) -> Pixel {
        let world = self.app.main_mut().world_mut();
        let camera = world
            .get_component_for_entity::<Camera>(self.camera)
            .unwrap();
        let rotation = Quat::from_rotation_x(CAMERA_PITCH_DEGREES.to_radians());
        let view_proj = camera.build_projection_matrix()
            * Transform::from_rotation(rotation)
                .compute_matrix()
                .inverse();
        let clip = view_proj * direction.extend(1.0);
        let ndc = clip.xy() / clip.w;
        let x = ((ndc.x * 0.5 + 0.5) * WIDTH as f32) as u32;
        let y = ((0.5 - ndc.y * 0.5) * HEIGHT as f32) as u32;
        let published = self.shared.get().expect("a texture was published");
        let texel = read_texture(&self.gpu, published.texture())
            .get_pixel(x.min(WIDTH - 1), y.min(HEIGHT - 1))
            .0;
        Pixel {
            rgb: Vec3::new(texel[0] as f32, texel[1] as f32, texel[2] as f32),
        }
    }
}

#[test]
fn the_sky_draws_behind_geometry_and_follows_the_sun() {
    let Some(gpu) = Gpu::request_for_test() else {
        return;
    };
    let mut scene = SkyScene::new(&gpu, 50.0);

    let above_the_cube = scene.pixel_toward(toward(30.0, -25.0));
    assert!(
        above_the_cube.is_blue(),
        "the sky above the cube is blue: {:?}",
        above_the_cube.rgb
    );
    let cube = scene.pixel_toward(Vec3::NEG_Z);
    assert!(cube.is_red(), "the cube hides the sky: {:?}", cube.rgb);
    let sun = scene.pixel_toward(toward(50.0, 0.0));
    assert!(
        sun.rgb.min_element() > 230.0,
        "the sun disk is near white: {:?}",
        sun.rgb
    );

    let far_fog = scene.pixel_toward(toward(0.0, FAR_CUBE_AZIMUTH_DEGREES));
    assert!(
        far_fog.is_blue(),
        "far fog takes the sky's color, not Fog::color: {:?}",
        far_fog.rgb
    );

    scene.turn_sun(50.0, 180.0);
    let sunlit_cube = scene.pixel_toward(Vec3::NEG_Z);
    assert!(
        sunlit_cube.rgb.x > 150.0,
        "the sun lights the face toward it: {:?}",
        sunlit_cube.rgb
    );

    scene.turn_sun(1.0, 0.0);
    let horizon = scene.pixel_toward(toward(2.0, 15.0));
    assert!(
        horizon.is_red(),
        "the horizon toward a setting sun is red: {:?}",
        horizon.rgb
    );

    scene.turn_sun(-10.0, 0.0);
    let night = scene.pixel_toward(toward(30.0, -25.0));
    assert!(
        night.rgb.max_element() < 30.0,
        "the sky past twilight is dark: {:?}",
        night.rgb
    );

    scene.turn_sun(-10.0, 180.0);
    let night_cube = scene.pixel_toward(Vec3::NEG_Z);
    assert!(
        night_cube.rgb.x < 60.0 && night_cube.rgb.x < sunlit_cube.rgb.x,
        "below the horizon the sun's tint puts out its light and the sky's ambient replaces CameraEnvironment::ambient: {:?}",
        night_cube.rgb
    );
    assert!(
        night_cube.rgb.x > 10.0,
        "night_ambient keeps the face from black: {:?}",
        night_cube.rgb
    );
}
