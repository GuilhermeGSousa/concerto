use concerto_app::{
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
    App,
};
use concerto_color::Color;
use concerto_ecs::Entity;
use concerto_foundation::{
    assets::{asset_server::AssetServer, asset_store::AssetStore, handle::AssetHandle, AssetId},
    transform::Transform,
};
use concerto_mesh::{
    primitives::{cuboid, plane},
    MeshComponent,
};
use concerto_render::{
    assets::{material::StandardMaterial, texture::Texture},
    components::{
        camera::{Camera, RenderCamera, RenderTarget},
        camera_environment::CameraEnvironment,
        material::MaterialComponent,
        render_entity::RenderEntity,
    },
    draw_target::DrawTarget,
    gpu::Gpu,
    material_plugin::MaterialPlugin,
    plugin::RenderPlugin,
    readback::read_texture,
    AmbientOcclusionPlugin, ScreenSpaceAmbientOcclusion, Tonemapping,
};
use concerto_window::plugin::{Window, WindowPlugin};
use glam::{Quat, UVec2, Vec2, Vec3};

const SIZE: u32 = 128;
const EYE: Vec3 = Vec3::new(0.0, 6.0, 6.0);
const CREASE: Vec3 = Vec3::new(0.0, 0.0, 1.1);
const OPEN_FLOOR: Vec3 = Vec3::new(0.0, 0.0, 3.0);
const NOISE: u8 = 8;

struct Scene {
    app: App,
    gpu: Gpu,
    camera: Entity,
}

struct Shown {
    crease: u8,
    open_floor: u8,
}

fn rotation() -> Quat {
    glam::camera::rh::view::look_at_quat(EYE, Vec3::ZERO, Vec3::Y).inverse()
}

fn pixel(point: Vec3) -> UVec2 {
    let view = glam::Mat4::from_rotation_translation(rotation(), EYE).inverse();
    let ndc = (Camera::default().build_projection_matrix() * view).project_point3(point);
    let uv = Vec2::new(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    (uv * SIZE as f32).as_uvec2()
}

impl Scene {
    fn new(gpu: &Gpu) -> Self {
        let mut app = App::new();
        app.insert_resource(gpu.clone())
            .insert_resource(DrawTarget::None)
            .insert_resource(Window::new((64, 48), 1.0));
        app.register_plugin(MainSchedulePlugin)
            .register_plugin(AssetManagerPlugin)
            .register_plugin(TimePlugin)
            .register_plugin(WindowPlugin)
            .register_plugin(TransformPlugin)
            .register_plugin(RenderPlugin)
            .register_plugin(MaterialPlugin::<StandardMaterial>::default())
            .register_plugin(AmbientOcclusionPlugin);
        app.main_mut().world_mut().register_component::<Transform>();

        let server = app.get_resource::<AssetServer>().unwrap().clone();
        let white = server.add(
            StandardMaterial::default().with_base_color_factor(Color::rgba(0.8, 0.8, 0.8, 1.0)),
        );
        app.main_mut().world_mut().spawn((
            MeshComponent {
                handle: server.add(plane(Vec2::splat(12.0))),
            },
            MaterialComponent::<StandardMaterial> {
                handle: white.clone(),
            },
            Transform::IDENTITY,
        ));
        app.main_mut().world_mut().spawn((
            MeshComponent {
                handle: server.add(cuboid(Vec3::splat(2.0))),
            },
            MaterialComponent::<StandardMaterial> { handle: white },
            Transform::from_translation(Vec3::new(0.0, 1.0, 0.0)),
        ));

        let target = AssetId::from_path("textures/ambient_occlusion");
        app.main_mut()
            .world_mut()
            .get_resource_mut::<AssetStore<Texture>>()
            .unwrap()
            .insert(target, Texture::render_target(SIZE, SIZE));
        let camera = app.main_mut().world_mut().spawn((
            Camera {
                render_target: RenderTarget::Texture(AssetHandle::weak(target)),
                ..Camera::default()
            },
            CameraEnvironment::default().with_ambient(Color::rgba(1.0, 1.0, 1.0, 1.0)),
            Tonemapping::None,
            Transform::from_translation_rotation(EYE, rotation()),
        ));
        app.finish_plugin_build();
        Self {
            app,
            gpu: gpu.clone(),
            camera,
        }
    }

    fn with(&mut self, occlusion: ScreenSpaceAmbientOcclusion) -> Shown {
        let camera = self.camera;
        self.app.main_mut().world_mut().insert(occlusion, camera);
        self.shown()
    }

    fn shown(&mut self) -> Shown {
        for _ in 0..10 {
            self.app.update();
        }
        let render_entity = **self
            .app
            .main()
            .world()
            .get_component_for_entity::<RenderEntity>(self.camera)
            .unwrap();
        let render_camera = self
            .app
            .render()
            .world()
            .get_component_for_entity::<RenderCamera>(render_entity)
            .unwrap();
        let image = read_texture(
            &self.gpu,
            render_camera.render_target.as_ref().unwrap().texture(),
        );
        let red = |point: Vec3| {
            let pixel = pixel(point);
            image.get_pixel(pixel.x, pixel.y).0[0]
        };
        Shown {
            crease: red(CREASE),
            open_floor: red(OPEN_FLOOR),
        }
    }
}

#[test]
fn ambient_occlusion_darkens_a_crease_and_leaves_open_floor_and_an_intensity_of_zero_alone() {
    let Some(gpu) = Gpu::request_for_test() else {
        return;
    };
    let mut scene = Scene::new(&gpu);

    let plain = scene.shown();
    assert!(
        plain.open_floor > 150,
        "the floor is lit by the ambient light"
    );
    assert_eq!(
        plain.crease, plain.open_floor,
        "without occlusion the ambient light is flat"
    );

    let occluded = scene.with(ScreenSpaceAmbientOcclusion::default());
    assert!(
        occluded.open_floor.abs_diff(plain.open_floor) <= NOISE,
        "open floor keeps its ambient light: {} against {}",
        occluded.open_floor,
        plain.open_floor
    );
    assert!(
        occluded.crease + 20 < occluded.open_floor,
        "the floor at the foot of the box is darker: {} against {}",
        occluded.crease,
        occluded.open_floor
    );

    let short = scene.with(ScreenSpaceAmbientOcclusion::default().with_radius(0.05));
    assert!(
        short.crease.abs_diff(plain.crease) <= NOISE,
        "a radius that stops short of the box finds nothing: {} against {}",
        short.crease,
        plain.crease
    );

    let off = scene.with(ScreenSpaceAmbientOcclusion::default().with_intensity(0.0));
    assert_eq!(
        off.crease, plain.crease,
        "an intensity of zero is no occlusion"
    );
    assert_eq!(off.open_floor, plain.open_floor);
}
