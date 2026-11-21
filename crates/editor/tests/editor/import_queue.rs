//! The queue runs imports one at a time and survives a failing row.
use concerto_app::{App, schedule_groups::Update};
use concerto_ecs::events::{event_channel::EventChannel, event_cursor::EventCursor};
use concerto_editor::{
    asset_editor::{AssetEditorCommand, AssetEditorCommands},
    guard::UnsavedGuard,
    import::{ImportPlugin, ImportQueue, stage_sources},
    project::{AssetsWritten, EditorCommands, ProjectPlugin, ProjectState, discover_project},
};
use concerto_foundation::assets::{AssetId, asset_server::AssetServer, registry::AssetRegistry};
use std::path::{Path, PathBuf};

struct Sources {
    _dir: tempfile::TempDir,
    paths: Vec<PathBuf>,
}

fn sources(names: &[&str]) -> Sources {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../obj/tests/fixtures");
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::copy(fixtures.join("square.mtl"), dir.path().join("square.mtl")).expect("copy");
    let paths = names
        .iter()
        .map(|name| {
            let path = dir.path().join(name);
            std::fs::copy(fixtures.join("square.obj"), &path).expect("copy");
            path
        })
        .collect();
    Sources { _dir: dir, paths }
}

fn status(app: &App) -> String {
    app.get_resource::<ProjectState>()
        .expect("state")
        .status
        .clone()
}

fn editor(root: &Path) -> App {
    std::fs::create_dir_all(root.join("content")).expect("content");
    let mut app = App::new();
    app.insert_resource(EditorCommands::default());
    app.insert_resource(AssetEditorCommands::default());
    app.insert_resource(UnsavedGuard::default());
    app.insert_resource(AssetServer::new());
    app.insert_resource(concerto_ui::theme::UITheme::default());
    let (opened, registry) = discover_project(root).expect("discover");
    let mut state = ProjectState::default();
    state.project = Some(opened);
    app.insert_resource(state);
    app.insert_resource(registry);
    app.register_plugin(ProjectPlugin);
    app.register_plugin(ImportPlugin);
    app.finish_plugin_build();
    app
}

fn frame(app: &mut App) {
    app.main_mut().world_mut().run_schedule(Update);
}

fn drain(app: &mut App) {
    for _ in 0..500 {
        frame(app);
        let queue = app.get_resource::<ImportQueue>().expect("queue");
        if queue.remaining() == 0 && !queue.is_running() {
            frame(app);
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the queue never drained");
}

#[test]
fn a_batch_writes_every_row_past_a_failing_one_into_the_catalogue_and_sends_what_it_wrote() {
    let source = sources(&["square.obj"]);
    let path = &source.paths[0];
    let project = tempfile::tempdir().expect("tempdir");
    let mut app = editor(project.path());
    let mut rows = stage_sources(vec![
        PathBuf::from("/nonexistent/missing.obj"),
        path.clone(),
        path.clone(),
    ]);
    rows[1].destination = "assets/square.obj".into();
    rows[2].destination = "assets/other.obj".into();
    app.get_resource_mut::<ImportQueue>()
        .expect("queue")
        .enqueue(rows);
    drain(&mut app);

    for file in ["assets/square.obj", "assets/square.mtl", "assets/other.obj"] {
        assert!(project.path().join(file).is_file(), "{file}");
    }
    assert!(status(&app).contains("missing.obj"), "{}", status(&app));
    let mut imported: Vec<AssetId> = app
        .get_resource::<AssetRegistry>()
        .expect("registry")
        .iter()
        .map(|entry| entry.id)
        .collect();
    imported.sort();
    assert_eq!(imported.len(), 6, "a mesh, a material and a scene each");
    let mut written: Vec<AssetId> = app
        .get_resource::<EventChannel<AssetsWritten>>()
        .expect("the event is registered")
        .read_from(&mut EventCursor::default())
        .flat_map(|written| written.ids.clone())
        .collect();
    written.sort();
    assert_eq!(written, imported);
    assert!(
        !app.get_resource::<AssetEditorCommands>()
            .expect("documents")
            .0
            .iter()
            .any(|command| matches!(command, AssetEditorCommand::CloseAll)),
        "an import must not close open documents"
    );
}

#[test]
fn only_one_row_is_in_flight_at_a_time() {
    let source = sources(&["square.obj"]);
    let path = &source.paths[0];
    let project = tempfile::tempdir().expect("tempdir");
    let mut app = editor(project.path());
    let mut rows = stage_sources(vec![path.clone(), path.clone()]);
    rows[1].destination = "assets/other.obj".into();
    app.get_resource_mut::<ImportQueue>()
        .expect("queue")
        .enqueue(rows);
    frame(&mut app);
    let queue = app.get_resource::<ImportQueue>().expect("queue");
    assert!(queue.is_running(), "the first row started");
    assert_eq!(queue.remaining(), 1, "the second row waits its turn");
    drain(&mut app);
}

#[test]
fn a_project_swapped_mid_batch_gets_none_of_its_rows_or_entries() {
    let source = sources(&["first.obj", "second.obj"]);
    let project = tempfile::tempdir().expect("tempdir");
    let other = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(other.path().join("content")).expect("content");
    let mut app = editor(project.path());
    app.get_resource_mut::<ImportQueue>()
        .expect("queue")
        .enqueue(stage_sources(source.paths.clone()));
    frame(&mut app);
    assert!(
        app.get_resource::<ImportQueue>()
            .expect("queue")
            .is_running()
    );

    let (opened, registry) = discover_project(other.path()).expect("discover");
    app.get_resource_mut::<ProjectState>()
        .expect("state")
        .project = Some(opened);
    app.insert_resource(registry);
    drain(&mut app);

    assert!(project.path().join("assets/first.obj").is_file());
    assert!(!project.path().join("assets/second.obj").exists());
    assert!(!other.path().join("assets").exists());
    assert_eq!(
        app.get_resource::<AssetRegistry>()
            .expect("registry")
            .iter()
            .count(),
        0
    );
}
