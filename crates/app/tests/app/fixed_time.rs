use std::{thread, time::Duration};

use concerto_app::{
    main_schedule::MainSchedulePlugin,
    plugins::TimePlugin,
    schedule_groups::{FixedUpdate, LateUpdate},
    App,
};
use concerto_ecs::resource::{Res, ResMut, Resource};
use concerto_foundation::time::Time;

#[derive(Resource, Default)]
struct Seen {
    fixed: Vec<Duration>,
    frame: Duration,
}

fn note_fixed_step(time: Res<Time>, mut seen: ResMut<Seen>) {
    seen.fixed.push(time.delta());
}

fn note_frame(time: Res<Time>, mut seen: ResMut<Seen>) {
    seen.frame = time.delta();
}

#[test]
fn a_frame_runs_the_fixed_steps_its_own_delta_pays_for() {
    let mut app = App::new();
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(TimePlugin)
        .insert_resource(Seen::default())
        .add_system(FixedUpdate, note_fixed_step)
        .add_system(LateUpdate, note_frame);
    app.finish_plugin_build();
    app.update();
    app.get_resource_mut::<Seen>().unwrap().fixed.clear();

    thread::sleep(Duration::from_millis(120));
    app.update();

    let seen = app.get_resource::<Seen>().unwrap();
    let steps = (seen.frame.as_secs_f64() / Time::fixed_delta_time().as_secs_f64()) as usize;
    assert!(steps >= 3, "a 120 ms frame pays for at least 3 steps");
    assert!(
        seen.fixed.len() >= steps,
        "the frame of {:?} ran {} fixed steps, not the {steps} it pays for",
        seen.frame,
        seen.fixed.len()
    );
    assert!(
        seen.fixed.iter().all(|delta| *delta == seen.frame),
        "fixed steps saw {:?}, the frame's delta is {:?}",
        seen.fixed,
        seen.frame
    );
}
