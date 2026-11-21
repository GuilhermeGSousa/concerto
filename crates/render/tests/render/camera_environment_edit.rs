use concerto_app::{
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
    App,
};
use concerto_color::Color;
use concerto_ecs::Entity;
use concerto_foundation::transform::Transform;
use concerto_render::{
    components::{
        camera::{Camera, RenderCamera},
        camera_environment::CameraEnvironment,
    },
    draw_target::DrawTarget,
    gpu::Gpu,
    plugin::RenderPlugin,
};
use concerto_window::plugin::{Window, WindowPlugin};

fn gpu() -> Option<Gpu> {
    Gpu::request_for_test()
}

struct CameraApp {
    app: App,
    camera: Entity,
}

fn camera_app(gpu: &Gpu) -> CameraApp {
    let mut app = App::new();
    app.insert_resource(gpu.clone())
        .insert_resource(DrawTarget::None)
        .insert_resource(Window::new((64, 48), 1.0));
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(AssetManagerPlugin)
        .register_plugin(TimePlugin)
        .register_plugin(WindowPlugin)
        .register_plugin(TransformPlugin)
        .register_plugin(RenderPlugin);
    app.main_mut().world_mut().register_component::<Transform>();
    let camera = app.main_mut().world_mut().spawn((
        Camera::default(),
        CameraEnvironment::default(),
        Transform::IDENTITY,
    ));
    app.finish_plugin_build();
    CameraApp { app, camera }
}

fn uniform(app: &mut App) -> String {
    let world = app.render_mut().world_mut();
    let camera = world
        .query::<&RenderCamera, ()>()
        .iter(world)
        .next()
        .expect("the camera is extracted");
    format!("{:?}", camera.camera_uniform)
}

#[test]
fn removing_a_camera_environment_restores_the_default_uniform() {
    let Some(gpu) = gpu() else {
        return;
    };
    let CameraApp { mut app, camera } = camera_app(&gpu);
    for _ in 0..3 {
        app.update();
    }
    let before = uniform(&mut app);

    *app.main_mut()
        .world_mut()
        .get_component_for_entity_mut::<CameraEnvironment>(camera)
        .unwrap() = CameraEnvironment::default().with_ambient(Color::WHITE);
    for _ in 0..3 {
        app.update();
    }
    assert_ne!(uniform(&mut app), before);

    app.main_mut()
        .world_mut()
        .remove_component::<CameraEnvironment>(camera);
    for _ in 0..3 {
        app.update();
    }
    assert_eq!(uniform(&mut app), before);
}
