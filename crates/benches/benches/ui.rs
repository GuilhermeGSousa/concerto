use criterion::{criterion_group, criterion_main, BatchSize, Criterion};

use concerto::headless;
use concerto_app::{
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
    App,
};
use concerto_color::Color;
use concerto_ecs::entity::{hierarchy::ChildOf, Entity};
use concerto_ui::{
    material::UIMaterial,
    node::{FlexDirection, UICulled, UINode, UIRect},
    plugin::UIPlugin,
    text::UIText,
    transform::UIValue,
};
use concerto_window::plugin::Window;

const LABELS: usize = 1_000;

fn ui_plugins(app: &mut App) {
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(AssetManagerPlugin)
        .register_plugin(TimePlugin)
        .register_plugin(concerto_window::plugin::WindowPlugin)
        .register_plugin(TransformPlugin)
        .register_plugin(concerto_render::plugin::RenderPlugin)
        .register_plugin(UIPlugin);
}

fn settle(app: &mut App) {
    for _ in 0..3 {
        app.update();
    }
}

struct Labels {
    app: App,
    root: Entity,
    labels: Vec<Entity>,
    readout: Entity,
    quad: Entity,
}

fn text(text: impl Into<String>) -> UIText {
    UIText {
        text: text.into(),
        ..Default::default()
    }
}

fn labels() -> Option<Labels> {
    let mut app = headless(ui_plugins)?;
    let world = app.main_mut().world_mut();
    let root = world.spawn(
        UINode::default()
            .with_size(UIValue::Percent(100.0), UIValue::Percent(100.0))
            .with_flex_direction(FlexDirection::Column),
    );
    let labels = (0..LABELS)
        .map(|index| {
            world.spawn((
                UINode::default()
                    .with_flex_shrink(0.0)
                    .with_z_index(index as i32 % 10),
                text(format!("label {index}")),
                UIMaterial::flat(Color::WHITE),
                ChildOf::new(root),
            ))
        })
        .collect();
    let readout = world.spawn((
        UINode::default()
            .with_size(UIValue::Px(200.0), UIValue::Px(20.0))
            .with_flex_shrink(0.0),
        text("66 fps"),
        ChildOf::new(root),
    ));
    let quad = world.spawn((
        UINode::default()
            .with_size(UIValue::Px(32.0), UIValue::Px(32.0))
            .with_flex_shrink(0.0),
        UIMaterial::flat(Color::WHITE),
        ChildOf::new(root),
    ));
    settle(&mut app);
    Some(Labels {
        app,
        root,
        labels,
        readout,
        quad,
    })
}

struct Culled {
    app: App,
    shown: Entity,
}

fn culled() -> Option<Culled> {
    let mut app = headless(ui_plugins)?;
    let world = app.main_mut().world_mut();
    let row = || {
        UINode::default()
            .with_size(UIValue::Px(100.0), UIValue::Px(20.0))
            .with_flex_shrink(0.0)
    };
    let root = world.spawn(
        UINode::default()
            .with_size(UIValue::Percent(100.0), UIValue::Percent(100.0))
            .with_flex_direction(FlexDirection::Column),
    );
    let shown = world.spawn((row(), ChildOf::new(root)));
    for _ in 0..LABELS {
        let hidden = world.spawn((row(), UICulled, ChildOf::new(root)));
        world.spawn((row(), ChildOf::new(hidden)));
    }
    settle(&mut app);
    Some(Culled { app, shown })
}

fn node(app: &mut App, entity: Entity) -> &mut UINode {
    app.main_mut()
        .world_mut()
        .get_component_for_entity_mut::<UINode>(entity)
        .unwrap()
}

fn label_text(app: &mut App, entity: Entity) -> &mut UIText {
    app.main_mut()
        .world_mut()
        .get_component_for_entity_mut::<UIText>(entity)
        .unwrap()
}

fn ui_frames(c: &mut Criterion) {
    let Some(mut fixture) = labels() else {
        eprintln!("SKIPPED: no GPU adapter, nothing measured");
        return;
    };
    let mut group = c.benchmark_group("ui frame with 1,000 labels");

    group.bench_function("idle", |b| b.iter(|| fixture.app.update()));

    let changing = fixture.labels[LABELS / 2];
    let mut flip = false;
    group.bench_function("one label's text changed", |b| {
        b.iter(|| {
            flip = !flip;
            label_text(&mut fixture.app, changing).text = if flip {
                "a label that changed"
            } else {
                "label"
            }
            .into();
            fixture.app.update();
        })
    });

    group.bench_function("every node and text rewritten identically", |b| {
        b.iter(|| {
            for &label in &fixture.labels {
                let same = node(&mut fixture.app, label).clone();
                *node(&mut fixture.app, label) = same;
                let component = label_text(&mut fixture.app, label);
                component.text = component.text.clone();
            }
            fixture.app.update();
        })
    });

    let mut tick = 0_u32;
    group.bench_function("a fixed-size readout ticking", |b| {
        b.iter(|| {
            tick += 1;
            label_text(&mut fixture.app, fixture.readout).text = format!("{tick} fps");
            fixture.app.update();
        })
    });

    group.bench_function("one quad resized", |b| {
        b.iter(|| {
            flip = !flip;
            let size = if flip { 48.0 } else { 32.0 };
            *node(&mut fixture.app, fixture.quad) = UINode::default()
                .with_size(UIValue::Px(size), UIValue::Px(size))
                .with_flex_shrink(0.0);
            fixture.app.update();
        })
    });

    group.bench_function("one label raised to a new layer", |b| {
        b.iter(|| {
            flip = !flip;
            node(&mut fixture.app, changing).z_index = if flip { 11 } else { 12 };
            fixture.app.update();
        })
    });

    group.bench_function("every label scrolled by one pixel", |b| {
        b.iter(|| {
            flip = !flip;
            node(&mut fixture.app, fixture.root).padding = UIRect {
                top: if flip { 1.0 } else { 0.0 },
                ..UIRect::default()
            };
            fixture.app.update();
        })
    });

    group.bench_function("window resized", |b| {
        b.iter_batched(
            || {
                flip = !flip;
                if flip {
                    Window::new((1280, 720), 1.0)
                } else {
                    Window::new((1024, 768), 1.0)
                }
            },
            |window| {
                fixture.app.insert_resource(window);
                fixture.app.update();
            },
            BatchSize::SmallInput,
        )
    });
    group.finish();

    let Some(mut fixture) = culled() else {
        return;
    };
    let mut group = c.benchmark_group("ui frame with 1,000 culled rows");
    group.bench_function("the one shown row resized", |b| {
        b.iter(|| {
            flip = !flip;
            node(&mut fixture.app, fixture.shown).height =
                UIValue::Px(if flip { 30.0 } else { 20.0 });
            fixture.app.update();
        })
    });
    group.finish();
}

criterion_group!(benches, ui_frames);
criterion_main!(benches);
