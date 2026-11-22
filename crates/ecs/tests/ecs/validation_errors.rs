use std::{
    sync::{Mutex, Once},
    thread::ThreadId,
};

use concerto_ecs::{
    Component, IntoSystem, Res, Resource, System, World,
    component::SystemTicks,
    system::{
        access::SystemAccess,
        input::{SystemInput, ValidationError},
        meta::SystemMetadata,
    },
    world::UnsafeWorldCell,
};

struct Record {
    thread: ThreadId,
    level: log::Level,
    message: String,
}

static RECORDS: Mutex<Vec<Record>> = Mutex::new(Vec::new());

struct Capture;

impl log::Log for Capture {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        RECORDS.lock().unwrap().push(Record {
            thread: std::thread::current().id(),
            level: record.level(),
            message: record.args().to_string(),
        });
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
        .filter(|record| record.thread == me && record.level == log::Level::Warn)
        .map(|record| record.message.clone())
        .collect()
}

struct Skips;

impl SystemInput for Skips {
    type State = ();
    type Data<'world, 'state> = Self;

    fn init_state(_world: &mut World) -> Self::State {}

    unsafe fn get_data<'world, 'state>(
        _state: &'state mut Self::State,
        _world: UnsafeWorldCell<'world>,
        _ticks: SystemTicks,
    ) -> Result<Self::Data<'world, 'state>, ValidationError> {
        Err(ValidationError::skip::<Self>("not now"))
    }

    fn fill_access(_meta: &mut SystemMetadata, _access: &mut SystemAccess) {}
}

#[derive(Resource)]
struct Absent;

fn run_times<M>(system: impl IntoSystem<(), M>, times: usize) {
    let mut world = World::new();
    let mut system = system.into_system();
    system.initialize(&mut world);
    for _ in 0..times {
        system.run_and_apply((), &mut world);
    }
}

fn reads_absent(_: Res<Absent>) {}

fn skipped(_: Skips) {}

#[test]
fn a_missing_resource_warns_on_every_skipped_run_and_a_skip_logs_nothing() {
    let before = warnings().len();

    run_times(reads_absent, 3);
    run_times(skipped, 3);

    assert_eq!(warnings().len() - before, 3);
}

#[derive(Component)]
struct Left;

#[derive(Component)]
struct Right;

#[derive(Resource)]
struct Shared;

fn initializes<Marker>(system: impl IntoSystem<(), Marker>) -> bool {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut world = World::default();
        system.into_system().initialize(&mut world);
    }))
    .is_ok()
}

#[test]
fn a_system_whose_parameters_conflict_panics_when_it_is_initialized() {
    use concerto_ecs::{Changed, Entity, Query, ResMut, With, Without};

    assert!(!initializes(|_: ResMut<Shared>, _: ResMut<Shared>| {}));
    assert!(!initializes(|_: Res<Shared>, _: ResMut<Shared>| {}));
    assert!(!initializes(|_: Query<(&mut Left, &Left)>| {}));
    assert!(!initializes(|_: Query<&mut Left>, _: Query<&Left>| {}));
    assert!(!initializes(|_: Query<&mut Left>, _: Query<&mut Left>| {}));
    assert!(!initializes(
        |_: Query<&mut Left>, _: Query<Entity, Changed<Left>>| {}
    ));
    assert!(!initializes(
        |_: Query<&mut Left, With<Right>>, _: Query<&mut Left>| {}
    ));

    assert!(initializes(|_: Res<Shared>, _: Res<Shared>| {}));
    assert!(initializes(|_: Query<&Left>, _: Query<&Left>| {}));
    assert!(initializes(|_: Query<&mut Left, Changed<Left>>| {}));
    assert!(initializes(
        |_: Query<(&mut Left, &Right)>, _: Query<&Right>| {}
    ));
    assert!(initializes(
        |_: Query<&mut Left, With<Right>>, _: Query<&mut Left, Without<Right>>| {}
    ));
    assert!(initializes(
        |_: Query<(&mut Left, &Right)>, _: Query<&mut Left, Without<Right>>| {}
    ));
}
