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
        render_entity::{RenderEntity, SyncWithRenderWorld},
    },
    draw_target::DrawTarget,
    gpu::Gpu,
    plugin::RenderPlugin,
    ExclusiveCamera,
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

fn render_entity(app: &mut App, entity: Entity) -> Option<Entity> {
    app.main_mut()
        .world_mut()
        .get_component_for_entity::<RenderEntity>(entity)
        .map(|render_entity| **render_entity)
}

#[test]
fn a_bare_camera_gets_a_render_entity() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut app = app(&gpu);
    let camera = app
        .main_mut()
        .world_mut()
        .spawn((Camera::default(), Transform::IDENTITY));
    app.finish_plugin_build();

    for _ in 0..3 {
        app.update();
    }

    assert!(app
        .main_mut()
        .world_mut()
        .get_component_for_entity::<SyncWithRenderWorld>(camera)
        .is_some());
    assert!(render_entity(&mut app, camera).is_some());
}

#[test]
fn a_scene_camera_under_an_exclusive_camera_is_mirrored_but_not_extracted() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut app = app(&gpu);
    let scene_camera = app
        .main_mut()
        .world_mut()
        .spawn((Camera::default(), Transform::IDENTITY));
    let editor_camera = app
        .main_mut()
        .world_mut()
        .spawn((Camera::default(), Transform::IDENTITY));
    app.insert_resource(ExclusiveCamera(editor_camera));
    app.finish_plugin_build();

    for _ in 0..3 {
        app.update();
    }

    let mirror = render_entity(&mut app, scene_camera).expect("the scene camera has a mirror");
    assert!(app
        .render_mut()
        .world_mut()
        .get_component_for_entity::<RenderCamera>(mirror)
        .is_none());
    let editor_mirror = render_entity(&mut app, editor_camera).unwrap();
    assert!(app
        .render_mut()
        .world_mut()
        .get_component_for_entity::<RenderCamera>(editor_mirror)
        .is_some());
}

#[test]
fn a_bare_mesh_gets_the_marker() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut app = app(&gpu);
    let mesh = app
        .main_mut()
        .world_mut()
        .spawn(concerto_mesh::MeshComponent::default());

    assert!(app
        .main_mut()
        .world_mut()
        .get_component_for_entity::<SyncWithRenderWorld>(mesh)
        .is_some());
}

#[test]
fn a_bare_skeleton_gets_the_marker() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut app = app(&gpu);
    let skeleton = app
        .main_mut()
        .world_mut()
        .spawn(concerto_mesh::SkeletonComponent::default());

    assert!(app
        .main_mut()
        .world_mut()
        .get_component_for_entity::<SyncWithRenderWorld>(skeleton)
        .is_some());
}
