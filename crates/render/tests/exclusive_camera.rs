use concerto_app::{
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
    App,
};
use concerto_ecs::Entity;
use concerto_foundation::transform::Transform;
use concerto_render::{
    components::{
        camera::{Camera, RenderCamera},
        render_entity::SyncWithRenderWorld,
    },
    draw_target::DrawTarget,
    gpu::Gpu,
    plugin::RenderPlugin,
    ExclusiveCamera,
};
use concerto_window::plugin::{Window, WindowPlugin};

fn gpu() -> Option<Gpu> {
    Gpu::request_for_test()
}

fn two_cameras(gpu: &Gpu, exclusive: Option<usize>) -> App {
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
    let cameras: Vec<Entity> = (0..2)
        .map(|_| {
            app.main_mut().world_mut().spawn((
                Camera::default(),
                Transform::IDENTITY,
                SyncWithRenderWorld,
            ))
        })
        .collect();
    if let Some(index) = exclusive {
        app.insert_resource(ExclusiveCamera(cameras[index]));
    }
    app.finish_plugin_build();
    app
}

fn render_cameras(app: &mut App) -> usize {
    let world = app.render_mut().world_mut();
    world.query::<&RenderCamera, ()>().iter(world).count()
}

#[test]
fn every_synced_camera_is_extracted_without_an_exclusive_camera() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut app = two_cameras(&gpu, None);

    for _ in 0..3 {
        app.update();
    }

    assert_eq!(render_cameras(&mut app), 2);
}

#[test]
fn only_the_exclusive_camera_is_extracted() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut app = two_cameras(&gpu, Some(0));

    for _ in 0..3 {
        app.update();
    }

    assert_eq!(render_cameras(&mut app), 1);
}
