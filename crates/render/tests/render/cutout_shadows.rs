use std::f32::consts::FRAC_PI_2;

use concerto_app::{
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
    App,
};
use concerto_foundation::{
    assets::{asset_server::AssetServer, asset_store::AssetStore, handle::AssetHandle, AssetId},
    transform::Transform,
};
use concerto_mesh::{Mesh, MeshComponent};
use concerto_render::{
    assets::{
        material::{AlphaMode, StandardMaterial},
        texture::Texture,
    },
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
use glam::{Quat, Vec2, Vec3};

const SIZE: u32 = 96;
const TEXELS: u32 = 16;
const SUN_TILT: f32 = 0.9;
const SHEET_HEIGHT: f32 = 3.0;
const SHEET_SIZE: f32 = 2.0;

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

fn one_opaque_quarter() -> Texture {
    let mut data = Vec::with_capacity((TEXELS * TEXELS * 4) as usize);
    for y in 0..TEXELS {
        for x in 0..TEXELS {
            let opaque = x < TEXELS / 2 && y < TEXELS / 2;
            data.extend([255, 255, 255, if opaque { 255 } else { 0 }]);
        }
    }
    Texture::new(TEXELS, TEXELS, wgpu::TextureFormat::Rgba8UnormSrgb, data)
}

fn brightness(app: &App, gpu: &Gpu, camera: concerto_ecs::Entity, point: Vec3) -> f32 {
    let world = app.main().world();
    let main_camera = world.get_component_for_entity::<Camera>(camera).unwrap();
    let transform = world.get_component_for_entity::<Transform>(camera).unwrap();
    let clip = main_camera.build_projection_matrix()
        * transform.compute_matrix().inverse()
        * point.extend(1.0);
    let ndc = clip / clip.w;
    let to_pixel = |unit: f32| ((unit * SIZE as f32) as u32).min(SIZE - 1);
    let render_entity = **world
        .get_component_for_entity::<RenderEntity>(camera)
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

#[test]
fn a_masked_sheet_shadows_the_ground_only_under_what_its_cutout_leaves() {
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

    let server = app.get_resource::<AssetServer>().unwrap().clone();
    app.main_mut().world_mut().spawn((
        MeshComponent {
            handle: server.add(Mesh::cuboid(Vec3::new(40.0, 0.2, 40.0))),
        },
        MaterialComponent::<StandardMaterial> {
            handle: server.add(StandardMaterial::default()),
        },
        Transform::from_translation(Vec3::new(0.0, -0.1, 0.0)),
    ));
    let mut cutout = StandardMaterial::default().with_alpha_mode(AlphaMode::Mask { cutoff: 0.5 });
    cutout.set_base_color_texture(server.add(one_opaque_quarter()));
    app.main_mut().world_mut().spawn((
        MeshComponent {
            handle: server.add(Mesh::plane(Vec2::splat(SHEET_SIZE))),
        },
        MaterialComponent::<StandardMaterial> {
            handle: server.add(cutout),
        },
        Transform::from_translation(Vec3::new(0.0, SHEET_HEIGHT, 0.0)),
    ));

    let target = AssetId::from_path("textures/cutout_shadows");
    app.main_mut()
        .world_mut()
        .get_resource_mut::<AssetStore<Texture>>()
        .unwrap()
        .insert(target, Texture::render_target(SIZE, SIZE));
    let reach = SHEET_HEIGHT * SUN_TILT.tan();
    let under = Vec3::new(0.0, 0.0, -reach);
    let camera = app.main_mut().world_mut().spawn((
        Camera {
            render_target: RenderTarget::Texture(AssetHandle::weak(target)),
            ..Camera::default()
        },
        Transform::from_translation_rotation(
            Vec3::new(0.0, 12.0, -2.0),
            Quat::from_rotation_x(-FRAC_PI_2),
        ),
    ));

    app.finish_plugin_build();
    for _ in 0..6 {
        app.update();
    }

    let open = brightness(&app, &gpu, camera, Vec3::new(3.5, 0.0, -reach));
    let quarter = SHEET_SIZE * 0.25;
    let dark = [
        Vec2::new(-1.0, -1.0),
        Vec2::new(1.0, -1.0),
        Vec2::new(-1.0, 1.0),
        Vec2::new(1.0, 1.0),
    ]
    .iter()
    .filter(|corner| {
        let point = under + Vec3::new(corner.x, 0.0, corner.y) * quarter;
        brightness(&app, &gpu, camera, point) < open * 0.6
    })
    .count();
    assert_eq!(
        dark, 1,
        "one quarter of the sheet is opaque, so one quarter of its shadow is dark"
    );
}
