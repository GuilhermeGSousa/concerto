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
    DepthOfField, DepthOfFieldPlugin, Msaa, Tonemapping,
};
use concerto_window::plugin::{Window, WindowPlugin};
use glam::{Quat, Vec2, Vec3};

const SIZE: u32 = 64;

struct Scene {
    app: App,
    gpu: Gpu,
    camera: Entity,
}

struct Shown {
    lit: usize,
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
            .register_plugin(MaterialPlugin::<StandardMaterial>::default())
            .register_plugin(DepthOfFieldPlugin);
        app.main_mut().world_mut().register_component::<Transform>();

        let server = app.get_resource::<AssetServer>().unwrap().clone();
        let mut material =
            StandardMaterial::default().with_base_color_factor(Color::rgba(0.0, 0.0, 0.0, 1.0));
        material.set_emissive_factor(Vec3::ONE);
        material.set_metallic_factor(1.0);
        app.main_mut().world_mut().spawn((
            MeshComponent {
                handle: server.add(plane(Vec2::splat(3.0))),
            },
            MaterialComponent::<StandardMaterial> {
                handle: server.add(material),
            },
            Transform::from_rotation(Quat::from_rotation_y(0.4)),
        ));

        let target = AssetId::from_path("textures/depth_of_field");
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
            Msaa::Off,
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

    fn with(&mut self, lens: DepthOfField) -> Shown {
        let camera = self.camera;
        self.app.main_mut().world_mut().insert(lens, camera);
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
        let count = |range: std::ops::RangeInclusive<u8>| {
            image
                .pixels()
                .filter(|pixel| range.contains(&pixel.0[0]))
                .count()
        };
        Shown {
            lit: count(250..=255),
            between: count(6..=249),
        }
    }
}

fn wide_open(focus_distance: f32) -> DepthOfField {
    DepthOfField::at(focus_distance)
        .with_f_stop(0.1)
        .with_max_blur(0.2)
}

#[test]
fn a_surface_out_of_focus_is_blurred_and_one_in_focus_keeps_its_edge() {
    let Some(gpu) = Gpu::request_for_test() else {
        return;
    };
    let mut scene = Scene::new(&gpu);

    let plain = scene.shown();
    assert_eq!(plain.between, 0, "without a lens the quad's edge is hard");

    let focused = scene.with(wide_open(10.0));
    let unfocused = scene.with(wide_open(2.0));
    assert!(
        unfocused.between > 300 && unfocused.between > focused.between * 3,
        "focused eight units short of the quad, its edge spreads: {} against {}",
        unfocused.between,
        focused.between
    );
    assert!(
        unfocused.lit < plain.lit,
        "and less of it is at full brightness: {} against {}",
        unfocused.lit,
        plain.lit
    );

    let multisampled = {
        let camera = scene.camera;
        scene
            .app
            .main_mut()
            .world_mut()
            .insert(Msaa::Sample4, camera);
        scene.shown()
    };
    assert!(
        multisampled.between > 300,
        "the blur reads multisampled depth too: {}",
        multisampled.between
    );

    let closed = scene.with(wide_open(2.0).with_max_blur(0.0));
    assert!(
        closed.between < 150,
        "a widest blur of nothing leaves only the multisampled edge: {}",
        closed.between
    );
}
