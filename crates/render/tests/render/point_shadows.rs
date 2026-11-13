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
use concerto_mesh::{primitives::cuboid, MeshComponent};
use concerto_render::{
    assets::{material::StandardMaterial, texture::Texture},
    components::{
        camera::{Camera, RenderCamera, RenderTarget},
        light::{Light, PointShadowSettings},
        material::MaterialComponent,
        render_entity::RenderEntity,
    },
    draw_target::DrawTarget,
    gpu::Gpu,
    material_plugin::MaterialPlugin,
    plugin::RenderPlugin,
    readback::read_texture,
    shadow_pipeline::ShadowPipelinePlugin,
};
use concerto_window::plugin::{Window, WindowPlugin};
use glam::{Quat, Vec3};

const SIZE: u32 = 96;

fn app(gpu: &Gpu) -> App {
    let mut app = App::new();
    app.insert_resource(gpu.clone())
        .insert_resource(DrawTarget::None)
        .insert_resource(Window::new((64, 48), 1.0))
        .insert_resource(PointShadowSettings {
            max_casters: 1,
            face_size: 256,
        });
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
    let mesh = server.add(cuboid(size));
    let material = server.add(StandardMaterial::default());
    app.main_mut().world_mut().spawn((
        MeshComponent { handle: mesh },
        MaterialComponent::<StandardMaterial> { handle: material },
        Transform::from_translation(center),
    ));
}

fn camera_transform() -> Transform {
    Transform::from_translation_rotation(
        Vec3::new(0.0, 10.0, 0.0),
        Quat::from_rotation_x(-FRAC_PI_2),
    )
}

fn pixel_of(camera: &Camera, point: Vec3) -> [u32; 2] {
    let view = camera_transform().compute_matrix().inverse();
    let clip = camera.build_projection_matrix() * view * point.extend(1.0);
    let ndc = clip / clip.w;
    let to_pixel = |unit: f32| ((unit * SIZE as f32) as u32).min(SIZE - 1);
    [to_pixel(ndc.x * 0.5 + 0.5), to_pixel(0.5 - ndc.y * 0.5)]
}

fn brightness(app: &App, gpu: &Gpu, camera: Entity, points: &[Vec3]) -> Vec<f32> {
    let render_entity = **app
        .main()
        .world()
        .get_component_for_entity::<RenderEntity>(camera)
        .unwrap();
    let render_camera = app
        .render()
        .world()
        .get_component_for_entity::<RenderCamera>(render_entity)
        .unwrap();
    let image = read_texture(gpu, render_camera.render_target.as_ref().unwrap().texture());
    let main_camera = app
        .main()
        .world()
        .get_component_for_entity::<Camera>(camera)
        .unwrap();
    points
        .iter()
        .map(|&point| {
            let [x, y] = pixel_of(main_camera, point);
            let rgba = image.get_pixel(x, y).0;
            Vec3::new(rgba[0] as f32, rgba[1] as f32, rgba[2] as f32).length()
        })
        .collect()
}

struct Shade {
    case: &'static str,
    shadowed: Vec3,
    lit: [Vec3; 2],
}

#[test]
fn the_point_light_nearest_the_camera_shadows_the_floor_behind_each_box_and_nowhere_else() {
    let Some(gpu) = Gpu::request_for_test() else {
        return;
    };
    let mut app = app(&gpu);

    app.main_mut().world_mut().spawn((
        Light::point_light().with_intensity(0.0).with_shadows(),
        Transform::from_translation(Vec3::new(0.0, 30.0, 30.0)),
    ));
    app.main_mut().world_mut().spawn((
        Light::point_light().with_intensity(40.0).with_shadows(),
        Transform::from_translation(Vec3::new(0.0, 2.0, 0.0)),
    ));
    spawn_box(
        &mut app,
        Vec3::new(12.0, 0.1, 12.0),
        Vec3::new(0.0, -0.05, 0.0),
    );
    spawn_box(&mut app, Vec3::splat(0.5), Vec3::new(1.0, 1.0, 0.3));
    spawn_box(&mut app, Vec3::splat(0.5), Vec3::new(0.3, 1.0, -1.0));
    spawn_box(&mut app, Vec3::splat(0.5), Vec3::new(0.5, 1.0, 0.5));

    let target = AssetId::from_path("textures/point_shadows");
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
        camera_transform(),
    ));
    app.finish_plugin_build();
    for _ in 0..10 {
        app.update();
    }

    let cases = [
        Shade {
            case: "beyond the box on +X",
            shadowed: Vec3::new(2.5, 0.0, 0.75),
            lit: [Vec3::new(-2.5, 0.0, 0.75), Vec3::new(2.5, 0.0, -0.75)],
        },
        Shade {
            case: "beyond the box on -Z",
            shadowed: Vec3::new(0.75, 0.0, -2.5),
            lit: [Vec3::new(-0.75, 0.0, -2.5), Vec3::new(0.75, 0.0, 2.5)],
        },
        Shade {
            case: "below the box under the light",
            shadowed: Vec3::new(1.0, 0.0, 1.0),
            lit: [Vec3::new(-1.0, 0.0, 1.0), Vec3::new(1.0, 0.0, -1.0)],
        },
    ];
    for case in &cases {
        let values = brightness(
            &app,
            &gpu,
            camera,
            &[case.shadowed, case.lit[0], case.lit[1]],
        );
        let (shadowed, first, second) = (values[0], values[1], values[2]);
        assert!(
            shadowed < first * 0.5 && shadowed < second * 0.5,
            "{}: the shadowed floor reads {shadowed}, its mirrors {first} and {second}",
            case.case
        );
    }
}
