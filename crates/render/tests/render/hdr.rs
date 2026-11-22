use std::f32::consts::FRAC_PI_2;

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
use concerto_mesh::{primitives::plane, MeshComponent};
use concerto_render::{
    assets::{material::StandardMaterial, texture::Texture},
    components::{
        camera::{Camera, RenderCamera, RenderTarget},
        material::MaterialComponent,
        render_entity::RenderEntity,
    },
    draw_target::DrawTarget,
    gpu::Gpu,
    material_plugin::MaterialPlugin,
    plugin::RenderPlugin,
    readback::read_texture,
    AutoExposure, AutoExposurePlugin, Exposure, Tonemapping,
};
use concerto_window::plugin::{Window, WindowPlugin};
use glam::{Quat, Vec2, Vec3};

const SIZE: u32 = 16;
const TOLERANCE: u8 = 2;
const LIGHT: Vec3 = Vec3::new(4.0, 0.5, 0.0);

struct Scene {
    app: App,
    gpu: Gpu,
    camera: Entity,
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
            .register_plugin(AutoExposurePlugin);
        app.main_mut().world_mut().register_component::<Transform>();

        let server = app.get_resource::<AssetServer>().unwrap().clone();
        let mut material =
            StandardMaterial::default().with_base_color_factor(Color::rgba(0.0, 0.0, 0.0, 1.0));
        material.set_emissive_factor(LIGHT);
        material.set_metallic_factor(1.0);
        app.main_mut().world_mut().spawn((
            MeshComponent {
                handle: server.add(plane(Vec2::splat(8.0))),
            },
            MaterialComponent::<StandardMaterial> {
                handle: server.add(material),
            },
            Transform::IDENTITY,
        ));

        let target = AssetId::from_path("textures/hdr");
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
            Transform::from_translation_rotation(
                Vec3::new(0.0, 10.0, 0.0),
                Quat::from_rotation_x(-FRAC_PI_2),
            ),
        ));
        app.finish_plugin_build();
        Self {
            app,
            gpu: gpu.clone(),
            camera,
        }
    }

    fn shown(&mut self) -> [u8; 3] {
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
        let [r, g, b, _] = image.get_pixel(SIZE / 2, SIZE / 2).0;
        [r, g, b]
    }
}

fn assert_shown(actual: [u8; 3], expected: Vec3, what: &str) {
    let expected = (expected * 255.0)
        .round()
        .to_array()
        .map(|channel| channel as u8);
    assert!(
        actual
            .iter()
            .zip(expected)
            .all(|(&actual, expected)| actual.abs_diff(expected) <= TOLERANCE),
        "{what}: read {actual:?}, expected {expected:?}"
    );
}

#[test]
fn scene_light_above_one_is_kept_until_the_camera_exposes_and_tonemaps_it() {
    let Some(gpu) = Gpu::request_for_test() else {
        return;
    };
    let mut scene = Scene::new(&gpu);

    assert_shown(
        scene.shown(),
        Tonemapping::Aces.apply(LIGHT),
        "a camera with no settings tonemaps with ACES at an exposure of one",
    );

    let camera = scene.camera;
    scene
        .app
        .main_mut()
        .world_mut()
        .insert(Tonemapping::None, camera);
    assert_shown(
        scene.shown(),
        Vec3::new(1.0, 0.5, 0.0),
        "without a curve the light is clamped",
    );

    scene
        .app
        .main_mut()
        .world_mut()
        .insert(Exposure::new(-2.0), camera);
    assert_shown(
        scene.shown(),
        Vec3::new(1.0, 0.125, 0.0),
        "two stops down, light of four still reaches one: the target held it unclamped",
    );

    scene
        .app
        .main_mut()
        .world_mut()
        .insert((Tonemapping::Aces, Exposure::new(1.0)), camera);
    assert_shown(
        scene.shown(),
        Tonemapping::Aces.apply(LIGHT * 2.0),
        "exposure applies before the curve",
    );
}

#[test]
fn auto_exposure_brings_a_bright_scene_and_a_dark_one_toward_middle_grey_within_its_range() {
    let Some(gpu) = Gpu::request_for_test() else {
        return;
    };
    let mut scene = Scene::new(&gpu);
    let camera = scene.camera;
    let world = scene.app.main_mut().world_mut();
    world.insert(Tonemapping::None, camera);
    world.insert(Exposure::new(-6.0), camera);
    let dim = scene.shown();
    assert!(
        dim[0] < 20 && dim[1] < 5,
        "six stops down, light of four is dark: {dim:?}"
    );

    scene
        .app
        .main_mut()
        .world_mut()
        .insert(AutoExposure::default().with_speed(1000.0), camera);
    let adapted = scene.shown();
    scene
        .app
        .main_mut()
        .world_mut()
        .insert(Exposure::new(0.0), camera);
    let bright_adapted = scene.shown();
    assert!(
        adapted[0] < 20,
        "exposure still applies on top, as a compensation: {adapted:?}"
    );
    assert!(
        (60..=200).contains(&bright_adapted[0]) && (5..=40).contains(&bright_adapted[1]),
        "light of four and a half is brought down toward middle grey: {bright_adapted:?}"
    );

    scene.app.main_mut().world_mut().insert(
        AutoExposure::default()
            .with_range(0.0, 0.0)
            .with_speed(1000.0),
        camera,
    );
    assert_shown(
        scene.shown(),
        Vec3::new(1.0, 0.5, 0.0),
        "a range of nothing leaves the exposure alone",
    );
}
