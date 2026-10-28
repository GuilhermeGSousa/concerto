use concerto_app::{
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
    App,
};
use concerto_ecs::Entity;
use concerto_foundation::{
    assets::{asset_store::AssetStore, handle::AssetHandle, AssetId},
    transform::Transform,
};
use concerto_render::{
    assets::texture::Texture,
    components::{
        camera::{Camera, RenderCamera, RenderTarget},
        render_entity::{RenderEntity, SyncWithRenderWorld},
    },
    draw_target::DrawTarget,
    gpu::Gpu,
    plugin::RenderPlugin,
    render_asset::{render_texture::RenderTexture, RenderAssets},
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

fn render_target_size(app: &mut App, camera: Entity) -> Option<(u32, u32)> {
    let render_entity = app
        .main_mut()
        .world_mut()
        .get_component_for_entity::<RenderEntity>(camera)
        .map(|render_entity| **render_entity)?;
    let target = app
        .render_mut()
        .world_mut()
        .get_component_for_entity::<RenderCamera>(render_entity)?
        .render_target
        .as_ref()?;
    let size = target.texture().size();
    Some((size.width, size.height))
}

#[test]
fn a_camera_is_extracted_once_its_render_target_texture_loads() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut app = app(&gpu);
    let id = AssetId::from_path("textures/late_target");
    let camera = spawn_camera(&mut app, RenderTarget::Texture(AssetHandle::weak(id)));
    app.finish_plugin_build();

    for _ in 0..3 {
        app.update();
    }
    assert!(!is_extracted(&mut app, camera));

    app.main_mut()
        .world_mut()
        .get_resource_mut::<AssetStore<Texture>>()
        .expect("the render plugin registers the texture store")
        .insert(id, Texture::render_target(32, 16));
    for _ in 0..3 {
        app.update();
    }

    assert_eq!(render_target_size(&mut app, camera), Some((32, 16)));
    assert!(app
        .render_mut()
        .world_mut()
        .get_resource::<RenderAssets<RenderTexture>>()
        .is_some_and(|targets| targets.get(&id).is_some()));
}
