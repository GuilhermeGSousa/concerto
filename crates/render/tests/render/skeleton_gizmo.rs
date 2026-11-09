use concerto_app::{
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
    App,
};
use concerto_foundation::transform::Transform;
use concerto_mesh::SkeletonGizmo;
use concerto_reflect::to_value;
use concerto_render::{draw_target::DrawTarget, gpu::Gpu, plugin::RenderPlugin};
use concerto_window::plugin::{Window, WindowPlugin};

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

#[test]
fn render_plugin_registers_the_skeleton_gizmo_for_scenes() {
    let Some(gpu) = Gpu::request_for_test() else {
        return;
    };
    let mut app = app(&gpu);
    let world = app.main_mut().world_mut();
    let stored = to_value(&SkeletonGizmo::default()).unwrap();
    let entity = world.spawn(());

    assert!(world.apply_scene_component(
        "concerto_mesh::skeleton::SkeletonGizmo",
        &stored,
        entity,
        &[entity],
    ));
    assert_eq!(
        world.get_component_for_entity::<SkeletonGizmo>(entity),
        Some(&SkeletonGizmo::default())
    );
}
