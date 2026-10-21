use concerto_app::{
    main_schedule::MainSchedulePlugin, plugins::TimePlugin, schedule_groups::Update, App,
};
use concerto_ecs::{
    resource::{ResMut, Resource},
    system::schedule::ScheduleLabel,
};

#[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
struct Served;

#[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
struct RenderFrame;

#[derive(Resource, Default)]
struct ServedRuns(u32);

#[derive(Resource, Default)]
struct UpdateRuns(u32);

#[derive(Resource, Default)]
struct RenderRuns(u32);

fn count_served(mut runs: ResMut<ServedRuns>) {
    runs.0 += 1;
}

fn count_update(mut runs: ResMut<UpdateRuns>) {
    runs.0 += 1;
}

fn count_render(mut runs: ResMut<RenderRuns>) {
    runs.0 += 1;
}

fn built_app() -> App {
    let mut app = App::new();
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(TimePlugin)
        .insert_resource(ServedRuns::default())
        .insert_resource(UpdateRuns::default())
        .add_system(Served, count_served)
        .add_system(Update, count_update);
    app.render_mut()
        .set_update_schedule(RenderFrame)
        .insert_resource(RenderRuns::default());
    app.add_render_system(RenderFrame, count_render);
    app.finish_plugin_build();
    app
}

fn served_runs(app: &App) -> u32 {
    app.get_resource::<ServedRuns>().unwrap().0
}

fn update_runs(app: &App) -> u32 {
    app.get_resource::<UpdateRuns>().unwrap().0
}

fn render_runs(app: &App) -> u32 {
    app.render().get_resource::<RenderRuns>().unwrap().0
}

#[test]
fn serve_runs_one_main_schedule_without_advancing_the_frame() {
    let mut app = built_app();
    app.update();
    let tick = app.main().world().current_tick();
    assert_eq!(update_runs(&app), 1);
    assert_eq!(render_runs(&app), 1);

    app.serve(Served);
    assert_eq!(served_runs(&app), 1);
    assert_eq!(
        app.main().world().resource_changed_tick::<ServedRuns>(),
        Some(tick)
    );

    app.serve(Served);
    assert_eq!(served_runs(&app), 2);
    assert_eq!(
        app.main().world().resource_changed_tick::<ServedRuns>(),
        Some(tick)
    );

    assert_eq!(app.main().world().current_tick(), tick);
    assert_eq!(update_runs(&app), 1);
    assert_eq!(render_runs(&app), 1);

    app.update();
    assert_ne!(app.main().world().current_tick(), tick);
    assert_eq!(served_runs(&app), 2);
}
