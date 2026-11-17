use std::collections::HashSet;

use concerto_app::{
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
    App,
};
use concerto_color::Color;
use concerto_foundation::{assets::asset_server::AssetServer, transform::Transform};
use concerto_mesh::{primitives::cuboid, MeshComponent};
use concerto_render::{
    assets::material::StandardMaterial,
    components::{camera::Camera, camera_environment::CameraEnvironment, msaa::Msaa},
    draw_target::{DrawTarget, SharedTexture},
    gpu::Gpu,
    plugin::RenderPlugin,
    readback::read_texture,
    wgpu, MaterialComponent, MaterialPlugin,
};
use concerto_window::plugin::{Window, WindowPlugin};
use glam::{Quat, Vec3};

const BLUE: [u8; 4] = [0, 0, 255, 255];

fn colours_of_a_tilted_box(gpu: &Gpu, msaa: Option<Msaa>) -> HashSet<[u8; 4]> {
    let shared = SharedTexture::new(wgpu::TextureFormat::Rgba8Unorm);
    let mut app = App::new();
    app.insert_resource(gpu.clone())
        .insert_resource(DrawTarget::Texture(shared.clone()))
        .insert_resource(Window::new((64, 64), 1.0));
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(AssetManagerPlugin)
        .register_plugin(TimePlugin)
        .register_plugin(WindowPlugin)
        .register_plugin(TransformPlugin)
        .register_plugin(RenderPlugin)
        .register_plugin(MaterialPlugin::<StandardMaterial>::new());
    app.main_mut().world_mut().register_component::<Transform>();

    let camera = app.main_mut().world_mut().spawn((
        Camera {
            clear_color: Color::rgba(0.0, 0.0, 1.0, 1.0),
            ..Camera::default()
        },
        CameraEnvironment {
            fog: None,
            ambient: Color::rgba(1.0, 1.0, 1.0, 1.0),
        },
        Transform::IDENTITY,
    ));
    if let Some(msaa) = msaa {
        app.main_mut().world_mut().insert(msaa, camera);
    }
    let (mesh, material) = {
        let server = app.get_resource::<AssetServer>().unwrap();
        (
            server.add(cuboid(Vec3::new(1.0, 1.0, 0.1))),
            server.add(StandardMaterial::default()),
        )
    };
    app.main_mut().world_mut().spawn((
        MeshComponent { handle: mesh },
        MaterialComponent::<StandardMaterial> { handle: material },
        Transform::from_translation(Vec3::new(0.0, 0.0, -3.0))
            .with_rotation(Quat::from_rotation_z(0.5)),
    ));
    app.finish_plugin_build();

    let centre = |shared: &SharedTexture| {
        shared
            .get()
            .map(|published| read_texture(gpu, published.texture()).get_pixel(32, 32).0)
    };
    for _ in 0..100 {
        app.update();
        if centre(&shared).is_some_and(|pixel| pixel != BLUE) {
            break;
        }
    }
    let published = shared.get().expect("a texture was published");
    read_texture(gpu, published.texture())
        .pixels()
        .map(|pixel| pixel.0)
        .collect()
}

#[test]
fn a_camera_smooths_edges_by_default_and_not_with_msaa_off() {
    let Some(gpu) = Gpu::request_for_test() else {
        return;
    };

    let hard = colours_of_a_tilted_box(&gpu, Some(Msaa::Off));
    assert_eq!(
        hard.len(),
        2,
        "without MSAA each pixel is the box or the clear colour: {hard:?}"
    );

    let smooth = colours_of_a_tilted_box(&gpu, None);
    assert!(
        smooth.is_superset(&hard) && smooth.len() > 2,
        "at the default 4x, edge pixels blend the box into the clear colour: {smooth:?}"
    );
}
