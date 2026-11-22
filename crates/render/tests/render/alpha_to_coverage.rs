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
    assets::{
        material::{AlphaMode, StandardMaterial},
        texture::Texture,
    },
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
    Msaa, Tonemapping,
};
use concerto_window::plugin::{Window, WindowPlugin};
use glam::{Quat, Vec2, Vec3};

const SIZE: u32 = 64;
const TEXELS: u32 = 8;

fn disc() -> Texture {
    let mut data = Vec::with_capacity((TEXELS * TEXELS * 4) as usize);
    let centre = (TEXELS as f32 - 1.0) * 0.5;
    for y in 0..TEXELS {
        for x in 0..TEXELS {
            let inside = Vec2::new(x as f32 - centre, y as f32 - centre).length() < 2.6;
            data.extend([255, 255, 255, if inside { 255 } else { 0 }]);
        }
    }
    Texture::new(TEXELS, TEXELS, wgpu::TextureFormat::Rgba8UnormSrgb, data)
}

struct Scene {
    app: App,
    gpu: Gpu,
    camera: Entity,
}

struct Shown {
    lit: usize,
    dark: usize,
    between: usize,
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
            .register_plugin(MaterialPlugin::<StandardMaterial>::default());
        app.main_mut().world_mut().register_component::<Transform>();

        let server = app.get_resource::<AssetServer>().unwrap().clone();
        let mut material = StandardMaterial::default()
            .with_base_color_factor(Color::rgba(0.0, 0.0, 0.0, 1.0))
            .with_alpha_mode(AlphaMode::Mask { cutoff: 0.5 });
        material.set_base_color_texture(server.add(disc()));
        material.set_emissive_factor(Vec3::ONE);
        material.set_metallic_factor(1.0);
        app.main_mut().world_mut().spawn((
            MeshComponent {
                handle: server.add(plane(Vec2::splat(40.0))),
            },
            MaterialComponent::<StandardMaterial> {
                handle: server.add(material),
            },
            Transform::IDENTITY,
        ));

        let target = AssetId::from_path("textures/alpha_to_coverage");
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
                Vec3::new(0.0, 40.0, 0.0),
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

    fn with(&mut self, msaa: Msaa) -> Shown {
        let camera = self.camera;
        self.app.main_mut().world_mut().insert(msaa, camera);
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
        let count = |range: std::ops::RangeInclusive<u8>| {
            image
                .pixels()
                .filter(|pixel| range.contains(&pixel.0[0]))
                .count()
        };
        Shown {
            lit: count(250..=255),
            dark: count(0..=5),
            between: count(6..=249),
        }
    }
}

#[test]
fn a_cutout_edge_inside_a_mesh_is_smoothed_at_four_samples_and_hard_with_msaa_off() {
    let Some(gpu) = Gpu::request_for_test() else {
        return;
    };
    let mut scene = Scene::new(&gpu);

    let smooth = scene.with(Msaa::Sample4);
    assert!(
        smooth.lit > 100 && smooth.dark > 100,
        "the disc and the hole around it are in view"
    );
    assert!(
        smooth.between > 20,
        "the mesh covers every pixel, so only the cutout's edge is part covered: {}",
        smooth.between
    );

    let hard = scene.with(Msaa::Off);
    assert_eq!(hard.between, 0, "one sample is kept or discarded");
    assert!(hard.lit > 100 && hard.dark > 100);
}
