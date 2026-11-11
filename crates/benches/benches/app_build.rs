use criterion::{criterion_group, criterion_main, Criterion};

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

fn build_and_step() -> Option<App> {
    let mut app = headless(scene_plugins)?;
    for _ in 0..3 {
        app.update();
    }
    Some(app)
}

fn scene_app_build(c: &mut Criterion) {
    if build_and_step().is_none() {
        eprintln!("SKIPPED: no GPU adapter, nothing measured");
        return;
    }
    let mut group = c.benchmark_group("app_build");
    group.sample_size(10);
    group.bench_function("scene app, built and stepped three frames", |b| {
        b.iter_with_large_drop(build_and_step)
    });
    group.finish();
}

criterion_group!(benches, scene_app_build);
criterion_main!(benches);
