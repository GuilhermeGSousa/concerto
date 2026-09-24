//! Event lifetime must be a property of the frame, not of the order in which
//! plugins happened to call `register_event`.

use concerto_app::{
    main_schedule::MainSchedulePlugin,
    plugins::TimePlugin,
    schedule_groups::{LateUpdate, Update},
    App,
};
use concerto_ecs::{
    events::{event_reader::EventReader, event_writer::EventWriter, Event},
    resource::{ResMut, Resource},
};

#[derive(Event)]
struct Ping(usize);

#[derive(Resource)]
struct Emitted(usize);

#[derive(Resource, Default)]
struct Collected {
    reads: usize,
    seen: Vec<usize>,
}

fn emit(mut emitted: ResMut<Emitted>, mut writer: EventWriter<Ping>) {
    writer.write(Ping(emitted.0));
    emitted.0 += 1;
}

/// Reads on every other frame, so how long an event survives becomes observable.
fn collect_every_other_frame(mut reader: EventReader<Ping>, mut collected: ResMut<Collected>) {
    collected.reads += 1;
    if collected.reads % 2 == 1 {
        return;
    }
    for ping in reader.read() {
        collected.seen.push(ping.0);
    }
}

enum RegistrationOrder {
    BeforeSystems,
    AfterSystems,
}

fn collect_every_frame(mut reader: EventReader<Ping>, mut collected: ResMut<Collected>) {
    for ping in reader.read() {
        collected.seen.push(ping.0);
    }
}

fn run_frames(order: RegistrationOrder, frames: usize) -> Vec<usize> {
    run_with(order, frames, collect_every_other_frame)
}

fn run_frames_reading_every_frame(order: RegistrationOrder, frames: usize) -> Vec<usize> {
    run_with(order, frames, collect_every_frame)
}

fn run_with<M>(
    order: RegistrationOrder,
    frames: usize,
    reader: impl concerto_ecs::IntoSystemConfig<M> + Copy + 'static,
) -> Vec<usize> {
    let mut app = App::new();
    app.register_plugin(MainSchedulePlugin);
    app.register_plugin(TimePlugin);
    app.insert_resource(Emitted(0));
    app.insert_resource(Collected::default());

    match order {
        RegistrationOrder::BeforeSystems => {
            app.register_event::<Ping>();
            app.add_system(LateUpdate, emit);
            app.add_system(Update, reader);
        }
        RegistrationOrder::AfterSystems => {
            app.add_system(LateUpdate, emit);
            app.add_system(Update, reader);
            app.register_event::<Ping>();
        }
    }

    app.finish_plugin_build();
    for _ in 0..frames {
        app.update();
    }

    app.get_resource::<Collected>().unwrap().seen.clone()
}

#[test]
fn event_lifetime_does_not_depend_on_when_the_event_was_registered() {
    let before = run_frames(RegistrationOrder::BeforeSystems, 6);
    let after = run_frames(RegistrationOrder::AfterSystems, 6);

    assert_eq!(before, after);

    // An event written in `LateUpdate` of frame N is readable for the whole of frame
    // N+1 and expires at the `EventUpdate` of frame N+2, so a reader that skips a frame
    // sees every other event.
    assert_eq!(before, vec![0, 2, 4]);
}

#[test]
fn a_reader_that_runs_every_frame_misses_nothing_whatever_the_registration_order() {
    assert_eq!(
        run_frames_reading_every_frame(RegistrationOrder::BeforeSystems, 6),
        vec![0, 1, 2, 3, 4]
    );
    assert_eq!(
        run_frames_reading_every_frame(RegistrationOrder::AfterSystems, 6),
        vec![0, 1, 2, 3, 4]
    );
}
