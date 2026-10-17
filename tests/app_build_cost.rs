use std::time::Instant;

use concerto::headless;
use concerto_app::{
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
    App,
};
use concerto_render::{
    assets::material::StandardMaterial, material_plugin::MaterialPlugin,
    shadow_pipeline::ShadowPipelinePlugin,
};

fn scene_plugins(app: &mut App) {
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(AssetManagerPlugin)
        .register_plugin(TimePlugin)
        .register_plugin(concerto_window::plugin::WindowPlugin)
        .register_plugin(TransformPlugin)
        .register_plugin(concerto_render::plugin::RenderPlugin)
        .register_plugin(ShadowPipelinePlugin)
        .register_plugin(MaterialPlugin::<StandardMaterial>::default())
        .register_plugin(concerto_world_grid::plugin::WorldGridPlugin)
        .register_plugin(concerto_scene::plugin::ScenePlugin);
}

fn build_and_step() -> Option<std::time::Duration> {
    let started = Instant::now();
    let mut app = headless(scene_plugins)?;
    for _ in 0..3 {
        app.update();
    }
    Some(started.elapsed())
}

#[test]
#[ignore = "a measurement, run by hand"]
fn the_cost_of_building_a_scene_app() {
    let Some(first) = build_and_step() else {
        println!("no GPU adapter: nothing measured");
        return;
    };
    let second = build_and_step().unwrap();
    let third = build_and_step().unwrap();
    println!("first build and three frames:  {first:?}");
    println!("second build and three frames: {second:?}");
    println!("third build and three frames:  {third:?}");
}
