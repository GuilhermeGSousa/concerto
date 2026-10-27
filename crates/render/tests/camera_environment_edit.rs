use std::any::TypeId;

use concerto_app::{
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
    App,
};
use concerto_color::Color;
use concerto_ecs::{
    reflect::{AppTypeRegistry, ReflectComponent},
    Entity,
};
use concerto_foundation::transform::Transform;
use concerto_reflect::{apply_value, to_value};
use concerto_render::{
    components::{
        camera::{Camera, RenderCamera},
        camera_environment::{CameraEnvironment, Fog},
        render_entity::SyncWithRenderWorld,
    },
    draw_target::DrawTarget,
    gpu::Gpu,
    plugin::RenderPlugin,
};
use concerto_window::plugin::{Window, WindowPlugin};

fn gpu() -> Option<Gpu> {
    pollster::block_on(Gpu::request(&Gpu::instance(), None))
}

fn camera_app(gpu: &Gpu) -> (App, Entity) {
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
        SyncWithRenderWorld,
    ));
    app.finish_plugin_build();
    (app, camera)
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
fn editing_a_camera_environment_changes_the_camera_uniform() {
    let Some(gpu) = gpu() else {
        return;
    };
    let (mut app, camera) = camera_app(&gpu);
    for _ in 0..3 {
        app.update();
    }
    let before = uniform(&mut app);

    let replacement = CameraEnvironment::default()
        .with_fog(Fog {
            color: Color::BLUE,
            density: 0.2,
            start: 4.0,
        })
        .with_ambient(Color::WHITE);
    let stored = to_value(&replacement).unwrap();
    let world = app.main_mut().world_mut();
    let reflect = *world
        .get_resource::<AppTypeRegistry>()
        .unwrap()
        .get_type_data::<ReflectComponent>(TypeId::of::<CameraEnvironment>())
        .expect("a camera environment is an editable component");
    assert!((reflect.edit)(world, camera, &mut |value| {
        apply_value(value, &stored);
    }));
    assert_eq!(
        world.get_component_for_entity::<CameraEnvironment>(camera),
        Some(&replacement)
    );

    for _ in 0..3 {
        app.update();
    }
    assert_ne!(uniform(&mut app), before);
}
