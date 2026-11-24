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
    components::{camera::Camera, camera_environment::UnlitView},
    draw_target::{DrawTarget, SharedTexture},
    gpu::Gpu,
    plugin::RenderPlugin,
    readback::read_texture,
    tonemapping::Tonemapping,
    wgpu, MaterialComponent, MaterialPlugin,
};
use concerto_window::plugin::{Window, WindowPlugin};
use glam::Vec3;

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

#[test]
fn an_unlit_view_draws_base_colour_without_lights_and_a_lit_one_does_not() {
    let Some(gpu) = Gpu::request_for_test() else {
        return;
    };
    let shared = SharedTexture::new(FORMAT);
    let mut app = App::new();
    app.insert_resource(gpu.clone())
        .insert_resource(DrawTarget::Texture(shared.clone()))
        .insert_resource(Window::new((32, 32), 1.0));
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
        Tonemapping::None,
        UnlitView,
        Transform::IDENTITY,
    ));
    let (mesh, material) = {
        let server = app.get_resource::<AssetServer>().unwrap();
        (
            server.add(cuboid(Vec3::new(10.0, 10.0, 1.0))),
            server.add(
                StandardMaterial::default().with_base_color_factor(Color::rgba(1.0, 0.0, 0.0, 1.0)),
            ),
        )
    };
    app.main_mut().world_mut().spawn((
        MeshComponent { handle: mesh },
        MaterialComponent::<StandardMaterial> { handle: material },
        Transform::from_translation(Vec3::new(0.0, 0.0, -3.0)),
    ));
    app.finish_plugin_build();

    let pixel = |app: &mut App| {
        for _ in 0..100 {
            app.update();
            if let Some(published) = shared.get() {
                let pixel = read_texture(&gpu, published.texture()).get_pixel(16, 16).0;
                if pixel != [0, 0, 255, 255] {
                    return pixel;
                }
            }
        }
        panic!("the cube was never drawn");
    };

    assert_eq!(pixel(&mut app), [255, 0, 0, 255], "unlit shows base colour");

    app.main_mut()
        .world_mut()
        .remove_component::<UnlitView>(camera);
    for _ in 0..3 {
        app.update();
    }
    let lit = pixel(&mut app);
    assert!(
        lit[0] < 64,
        "with no lights, a lit view shows only the dim default ambient: {lit:?}"
    );
}
