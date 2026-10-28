use concerto_app::{
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
    App,
};
use concerto_ecs::Entity;
use concerto_foundation::{assets::handle::AssetHandle, transform::Transform};
use concerto_render::{
    components::{
        camera::{Camera, RenderCamera, RenderTarget},
        render_entity::{RenderEntity, SyncWithRenderWorld},
    },
    draw_target::DrawTarget,
    gpu::Gpu,
    plugin::RenderPlugin,
};
use concerto_window::plugin::{Window, WindowPlugin};

fn gpu() -> Option<Gpu> {
    pollster::block_on(Gpu::request(&Gpu::instance(), None))
}

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
        .register_plugin(RenderPlugin);
    app.main_mut().world_mut().register_component::<Transform>();
    app
}

fn spawn_camera(app: &mut App, render_target: RenderTarget) -> Entity {
    app.main_mut().world_mut().spawn((
        Camera {
            render_target,
            ..Camera::default()
        },
        Transform::IDENTITY,
        SyncWithRenderWorld,
    ))
}

fn is_extracted(app: &mut App, camera: Entity) -> bool {
    let Some(render_entity) = app
        .main_mut()
        .world_mut()
        .get_component_for_entity::<RenderEntity>(camera)
        .map(|render_entity| **render_entity)
    else {
        return false;
    };
    app.render_mut()
        .world_mut()
        .get_component_for_entity::<RenderCamera>(render_entity)
        .is_some()
}

#[test]
fn a_camera_whose_render_target_texture_is_not_loaded_is_skipped() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut app = app(&gpu);
    let waiting = spawn_camera(&mut app, RenderTarget::Texture(AssetHandle::default()));
    let window = spawn_camera(&mut app, RenderTarget::MainWindow);
    app.finish_plugin_build();

    for _ in 0..3 {
        app.update();
    }

    assert!(!is_extracted(&mut app, waiting));
    assert!(is_extracted(&mut app, window));
}
