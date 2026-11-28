use std::f32::consts::FRAC_PI_2;

use concerto_app::{
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
    App,
};
use concerto_ecs::Entity;
use concerto_foundation::{
    assets::{asset_server::AssetServer, asset_store::AssetStore, handle::AssetHandle, AssetId},
    transform::Transform,
};
use concerto_mesh::{Mesh, MeshComponent};
use concerto_render::{
    assets::{material::StandardMaterial, texture::Texture},
    components::{
        camera::{Camera, RenderCamera, RenderTarget},
        light::Light,
        material::MaterialComponent,
        render_entity::RenderEntity,
    },
    draw_target::DrawTarget,
    gpu::Gpu,
    material_plugin::MaterialPlugin,
    plugin::RenderPlugin,
    shadow_pipeline::ShadowPipelinePlugin,
};
use concerto_window::plugin::{Window, WindowPlugin};
use glam::{Quat, Vec3};

const SIZE: u32 = 96;
const SUN_TILT: f32 = 0.6;

fn app(gpu: &Gpu) -> App {
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
        .register_plugin(ShadowPipelinePlugin)
        .register_plugin(MaterialPlugin::<StandardMaterial>::default());
    app.main_mut().world_mut().register_component::<Transform>();
    app
}

fn spawn_box(app: &mut App, size: Vec3, center: Vec3) {
    let server = app.get_resource::<AssetServer>().unwrap();
    let mesh = server.add(Mesh::cuboid(size));
    let material = server.add(StandardMaterial::default());
    app.main_mut().world_mut().spawn((
        MeshComponent { handle: mesh },
        MaterialComponent::<StandardMaterial> { handle: material },
        Transform::from_translation(center),
    ));
}

struct Stage {
    case: &'static str,
    ground: Vec3,
    height: f32,
    camera: Entity,
}

impl Stage {
    fn build(app: &mut App, case: &'static str, ground: Vec3, height: f32) -> Self {
        let slab = height * 0.2;
        spawn_box(
            app,
            Vec3::new(height * 4.0, 0.2, height * 4.0),
            ground - Vec3::Y * 0.1,
        );
        spawn_box(
            app,
            Vec3::new(slab, slab * 0.1, slab),
            ground + Vec3::Y * slab,
        );

        let target = AssetId::from_path(case);
        app.main_mut()
            .world_mut()
            .get_resource_mut::<AssetStore<Texture>>()
            .unwrap()
            .insert(target, Texture::render_target(SIZE, SIZE));
        let camera = app.main_mut().world_mut().spawn((
            Camera {
                render_target: RenderTarget::Texture(AssetHandle::weak(target)),
                zfar: 400.0,
                ..Camera::default()
            },
            Transform::from_translation_rotation(
                ground + Vec3::Y * height,
                Quat::from_rotation_x(-FRAC_PI_2),
            ),
        ));
        Self {
            case,
            ground,
            height,
            camera,
        }
    }

    fn shadowed(&self) -> Vec3 {
        self.ground - Vec3::Z * self.height * 0.18
    }

    fn lit(&self) -> Vec3 {
        self.ground + Vec3::Z * self.height * 0.3
    }

    fn brightness(&self, app: &App, gpu: &Gpu, point: Vec3) -> f32 {
        let world = app.main().world();
        let camera = world
            .get_component_for_entity::<Camera>(self.camera)
            .unwrap();
        let transform = world
            .get_component_for_entity::<Transform>(self.camera)
            .unwrap();
        let clip = camera.build_projection_matrix()
            * transform.compute_matrix().inverse()
            * point.extend(1.0);
        let ndc = clip / clip.w;
        let to_pixel = |unit: f32| ((unit * SIZE as f32) as u32).min(SIZE - 1);

        let render_entity = **world
            .get_component_for_entity::<RenderEntity>(self.camera)
            .unwrap();
        let render_camera = app
            .render()
            .world()
            .get_component_for_entity::<RenderCamera>(render_entity)
            .unwrap();
        let image = gpu.read_texture(render_camera.render_target.as_ref().unwrap().texture());
        let rgba = image
            .get_pixel(to_pixel(ndc.x * 0.5 + 0.5), to_pixel(0.5 - ndc.y * 0.5))
            .0;
        Vec3::new(rgba[0] as f32, rgba[1] as f32, rgba[2] as f32).length()
    }
}

#[test]
fn a_sun_shadows_the_ground_under_each_camera_near_and_far_from_the_origin() {
    let Some(gpu) = Gpu::request_for_test() else {
        return;
    };
    let mut app = app(&gpu);

    app.main_mut().world_mut().spawn((
        Light::directional_light()
            .with_intensity(3.0)
            .with_shadows(),
        Transform::from_rotation(Quat::from_rotation_x(SUN_TILT - FRAC_PI_2)),
    ));
    let stages = [
        Stage::build(
            &mut app,
            "close to the ground, far from the origin",
            Vec3::new(500.0, 0.0, 0.0),
            8.0,
        ),
        Stage::build(
            &mut app,
            "a second camera high above other ground",
            Vec3::new(-500.0, 0.0, 300.0),
            100.0,
        ),
    ];

    app.finish_plugin_build();
    for _ in 0..4 {
        app.update();
    }

    for stage in &stages {
        let shadowed = stage.brightness(&app, &gpu, stage.shadowed());
        let lit = stage.brightness(&app, &gpu, stage.lit());
        assert!(
            shadowed < lit * 0.6,
            "{}: the ground behind the slab reads {shadowed} and open ground {lit}",
            stage.case
        );
    }
}
