use std::{
    sync::{Mutex, Once},
    thread::ThreadId,
};

use concerto_ecs::{
    CommandQueue, IntoSystem, Res, Resource, System, World,
    component::SystemTicks,
    system::{
        access::SystemAccess,
        input::{SystemInput, ValidationError, ValidationKind},
        meta::SystemMetadata,
    },
    world::UnsafeWorldCell,
};

static RECORDS: Mutex<Vec<(ThreadId, log::Level, String)>> = Mutex::new(Vec::new());

struct Capture;

impl log::Log for Capture {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        RECORDS.lock().unwrap().push((
            std::thread::current().id(),
            record.level(),
            record.args().to_string(),
        ));
    }

    fn flush(&self) {}
}

fn warnings() -> Vec<String> {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        log::set_logger(&Capture).unwrap();
        log::set_max_level(log::LevelFilter::Trace);
    });
    let me = std::thread::current().id();
    RECORDS
        .lock()
        .unwrap()
        .iter()
        .filter(|(thread, level, _)| *thread == me && *level == log::Level::Warn)
        .map(|(_, _, message)| message.clone())
        .collect()
}

struct Declines<const MISSING: bool>;

impl<const MISSING: bool> SystemInput for Declines<MISSING> {
    type State = ();
    type Data<'world, 'state> = Self;

    fn init_state(_world: &mut World) -> Self::State {}

    unsafe fn get_data<'world, 'state>(
        _state: &'state mut Self::State,
        _world: UnsafeWorldCell<'world>,
        _ticks: SystemTicks,
    ) -> Result<Self::Data<'world, 'state>, ValidationError> {
        if MISSING {
            Err(ValidationError::missing::<Self>("nothing to read"))
        } else {
            Err(ValidationError::skip::<Self>("not now"))
        }
    }

    fn fill_access(_meta: &mut SystemMetadata, _access: &mut SystemAccess) {}
}

type Skips = Declines<false>;
type Missing = Declines<true>;

#[derive(Resource)]
struct Absent;

#[derive(Resource)]
struct Queued;

fn run_times<M>(system: impl IntoSystem<(), M>, times: usize) -> World {
    let mut world = World::new();
    let mut system = system.into_system();
    system.initialize(&mut world);
    for _ in 0..times {
        system.run_and_apply((), &mut world);
    }
    world
}

fn reads_absent(_: Res<Absent>) {}

#[test]
fn a_missing_resource_warns_on_every_skipped_run_naming_the_system_and_the_input() {
    let before = warnings().len();

    run_times(reads_absent, 3);

    let new = warnings().split_off(before);
    assert_eq!(new.len(), 3, "{new:?}");
    for warning in new {
        assert!(warning.contains("reads_absent"), "{warning}");
        assert!(warning.contains("Absent"), "{warning}");
    }
}

fn skipped(_: Skips) {}

#[test]
fn a_skip_logs_nothing() {
    let before = warnings().len();

    run_times(skipped, 3);

    assert_eq!(warnings().len(), before);
}

fn skip_then_missing(_: (Skips, Missing)) {}

fn missing_then_skip(_: (Missing, Skips)) {}

#[test]
fn a_tuple_reports_its_first_declining_part() {
    let before = warnings().len();
    run_times(skip_then_missing, 1);
    assert_eq!(warnings().len(), before);

    run_times(missing_then_skip, 1);
    let new = warnings().split_off(before);
    assert_eq!(new.len(), 1, "{new:?}");
    assert!(new[0].contains("nothing to read"), "{}", new[0]);
}

fn queues_then_skips(mut cmd: CommandQueue, _: Skips) {
    cmd.insert_resource(Queued);
}

#[test]
fn apply_after_a_skipped_run_applies_nothing() {
    let world = run_times(queues_then_skips, 1);

    assert!(world.get_resource::<Queued>().is_none());
}

#[test]
fn an_error_names_its_input_and_kind_and_displays_both_parts() {
    let error = ValidationError::missing::<Absent>("gone");

    assert!(matches!(error.kind, ValidationKind::Missing));
    assert!(error.input.ends_with("Absent"), "{}", error.input);
    assert_eq!(error.to_string(), format!("{}: gone", error.input));
    assert!(matches!(
        ValidationError::skip::<Absent>("later").kind,
        ValidationKind::Skip
    ));
}
