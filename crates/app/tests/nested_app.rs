use std::sync::mpsc;
use std::time::Duration;

use concerto_app::{
    main_schedule::MainSchedulePlugin,
    plugins::TimePlugin,
    schedule_groups::{First, Update},
    App,
};
use concerto_ecs::{
    resource::{ResMut, Resource},
    system::NonSendMarker,
};
use concerto_tasks::{compute_pool::ComputeTaskPool, task_pool::TaskPool};

#[derive(Resource)]
struct Inner(App);

#[derive(Resource, Default)]
struct Steps(usize);

fn count(mut steps: ResMut<Steps>) {
    steps.0 += 1;
}

fn step_inner(_: NonSendMarker, mut inner: ResMut<Inner>) {
    inner.0.update();
}

fn plain_app() -> App {
    let mut app = App::new();
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(TimePlugin)
        .insert_resource(Steps::default())
        .add_system(Update, count);
    app
}

#[test]
fn an_app_stepped_inside_another_apps_system_finishes_on_one_worker() {
    ComputeTaskPool::get_or_init(|| TaskPool::with_threads("compute", 1));

    let (done, finished) = mpsc::channel();
    std::thread::spawn(move || {
        let mut inner = plain_app();
        inner.finish_plugin_build();

        let mut outer = plain_app();
        outer.insert_resource(Inner(inner));
        outer.add_system(First, step_inner);
        outer.finish_plugin_build();

        outer.update();
        outer.update();

        let outer_steps = outer.get_resource::<Steps>().unwrap().0;
        let inner_steps = outer
            .get_resource::<Inner>()
            .unwrap()
            .0
            .get_resource::<Steps>()
            .unwrap()
            .0;
        done.send((outer_steps, inner_steps)).unwrap();
    });

    let (outer_steps, inner_steps) = finished
        .recv_timeout(Duration::from_secs(20))
        .expect("the nested frame deadlocked");
    assert_eq!(outer_steps, 2);
    assert_eq!(inner_steps, 2);
}
