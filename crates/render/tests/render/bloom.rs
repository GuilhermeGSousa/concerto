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
    Bloom, BloomPlugin, Tonemapping,
};
use concerto_window::plugin::{Window, WindowPlugin};
use glam::{Quat, Vec2, Vec3};

const SIZE: u32 = 64;
const CENTRE: u32 = SIZE / 2;
const BESIDE: u32 = CENTRE + 12;

struct Scene {
    app: App,
    gpu: Gpu,
    camera: Entity,
}

struct Shown {
    lamp: u8,
    beside: u8,
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
            .register_plugin(BloomPlugin);
        app.main_mut().world_mut().register_component::<Transform>();

        let server = app.get_resource::<AssetServer>().unwrap().clone();
        let mut material =
            StandardMaterial::default().with_base_color_factor(Color::rgba(0.0, 0.0, 0.0, 1.0));
        material.set_emissive_factor(Vec3::splat(8.0));
        material.set_metallic_factor(1.0);
        app.main_mut().world_mut().spawn((
            MeshComponent {
                handle: server.add(plane(Vec2::splat(1.0))),
            },
            MaterialComponent::<StandardMaterial> {
                handle: server.add(material),
            },
            Transform::IDENTITY,
        ));

        let target = AssetId::from_path("textures/bloom");
        app.main_mut()
            .world_mut()
            .get_resource_mut::<AssetStore<Texture>>()
            .unwrap()
            .insert(target, Texture::render_target(SIZE, SIZE));
        let camera = app.main_mut().world_mut().spawn((
            Camera {
                render_target: RenderTarget::Texture(AssetHandle::weak(target)),
                clear_color: Color::rgba(0.0, 0.0, 0.0, 1.0),
                ..Camera::default()
            },
            Tonemapping::None,
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

    fn with(&mut self, bloom: Bloom) -> Shown {
        let camera = self.camera;
        self.app.main_mut().world_mut().insert(bloom, camera);
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
        Shown {
            lamp: image.get_pixel(CENTRE, CENTRE).0[0],
            beside: image.get_pixel(BESIDE, CENTRE).0[0],
        }
    }
}

#[test]
fn bloom_spreads_bright_light_around_it_and_an_intensity_of_zero_leaves_the_frame_alone() {
    let Some(gpu) = Gpu::request_for_test() else {
        return;
    };
    let mut scene = Scene::new(&gpu);

    let plain = scene.shown();
    assert_eq!(plain.lamp, 255);
    assert_eq!(
        plain.beside, 0,
        "without bloom the dark beside the lamp is black"
    );

    let mixed = scene.with(Bloom::default());
    assert_eq!(mixed.lamp, 255, "the lamp stays brighter than white");
    assert!(
        (4..200).contains(&mixed.beside),
        "the lamp glows into the dark beside it: {}",
        mixed.beside
    );

    let stronger = scene.with(Bloom::default().with_intensity(0.5));
    assert!(
        stronger.beside > mixed.beside,
        "more intensity, more glow: {} against {}",
        stronger.beside,
        mixed.beside
    );

    let added = scene.with(Bloom::default().with_threshold(1.0));
    assert!(
        added.beside > 0,
        "light above the threshold glows: {}",
        added.beside
    );
    let dim = scene.with(Bloom::default().with_threshold(20.0));
    assert_eq!(dim.beside, 0, "light below the threshold does not");

    let off = scene.with(Bloom::default().with_intensity(0.0));
    assert_eq!(off.lamp, 255);
    assert_eq!(off.beside, 0, "an intensity of zero is no bloom");
}
