//! The queue runs imports one at a time and survives a failing row.
use concerto_app::{App, schedule_groups::Update};
use concerto_editor::{
    asset_editor::AssetEditorCommands,
    guard::UnsavedGuard,
    import::{ImportPlugin, ImportQueue, stage_sources},
    project::{EditorCommands, ProjectPlugin, ProjectState, discover_project},
};
use concerto_foundation::assets::asset_server::AssetServer;
use std::path::{Path, PathBuf};

fn scratch_source() -> (tempfile::TempDir, PathBuf) {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../obj/tests/fixtures/square.obj");
    let text = std::fs::read_to_string(fixture).expect("the obj fixture exists");
    let dir = tempfile::tempdir().expect("tempdir");
    let without_materials: String = text
        .lines()
        .filter(|line| !line.starts_with("mtllib") && !line.starts_with("usemtl"))
        .map(|line| format!("{line}\n"))
        .collect();
    let path = dir.path().join("square.obj");
    std::fs::write(&path, without_materials).expect("write source");
    (dir, path)
}

fn editor(root: &Path) -> App {
    std::fs::create_dir_all(root.join("content")).expect("content");
    let mut app = App::new();
    app.insert_resource(EditorCommands::default());
    app.insert_resource(AssetEditorCommands::default());
    app.insert_resource(UnsavedGuard::default());
    app.insert_resource(AssetServer::new());
    app.insert_resource(concerto_ui::theme::UITheme::default());
    let mut state = ProjectState::default();
    state.project = Some(discover_project(root).expect("discover"));
    app.insert_resource(state);
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
fn a_batch_imports_every_row_and_shows_up_in_the_catalogue() {
    let (_source, path) = scratch_source();
    let project = tempfile::tempdir().expect("tempdir");
    let mut app = editor(project.path());
    let mut rows = stage_sources(vec![path.clone(), path]);
    rows[1].destination = "assets/other.obj".into();
    app.get_resource_mut::<ImportQueue>()
        .expect("queue")
        .enqueue(rows);
    drain(&mut app);

    assert!(project.path().join("assets/square.obj").is_file());
    assert!(project.path().join("assets/other.obj").is_file());
    let state = app.get_resource::<ProjectState>().expect("state");
    assert!(
        state.project.as_ref().expect("project").assets.len() >= 2,
        "both imports reached the catalogue"
    );
}

#[test]
fn a_failing_row_does_not_abandon_the_rest_of_the_batch() {
    let (_source, path) = scratch_source();
    let project = tempfile::tempdir().expect("tempdir");
    let mut app = editor(project.path());
    let mut rows = stage_sources(vec![PathBuf::from("/nonexistent/missing.obj"), path]);
    rows[1].destination = "assets/square.obj".into();
    app.get_resource_mut::<ImportQueue>()
        .expect("queue")
        .enqueue(rows);
    drain(&mut app);

    assert!(
        project.path().join("assets/square.obj").is_file(),
        "the row after the failing one still imported"
    );
    let state = app.get_resource::<ProjectState>().expect("state");
    assert!(
        state.status.contains("missing.obj"),
        "the failure is reported, got: {}",
        state.status
    );
}

#[test]
fn a_new_batch_on_an_idle_queue_forgets_the_old_failures() {
    let (_source, path) = scratch_source();
    let project = tempfile::tempdir().expect("tempdir");
    let mut app = editor(project.path());
    app.get_resource_mut::<ImportQueue>()
        .expect("queue")
        .enqueue(stage_sources(vec![PathBuf::from(
            "/nonexistent/missing.obj",
        )]));
    drain(&mut app);
    app.get_resource_mut::<ImportQueue>()
        .expect("queue")
        .enqueue(stage_sources(vec![path]));
    drain(&mut app);

    let state = app.get_resource::<ProjectState>().expect("state");
    assert!(
        !state.status.contains("missing.obj"),
        "a fresh batch starts clean, got: {}",
        state.status
    );
}

#[test]
fn only_one_row_is_in_flight_at_a_time() {
    let (_source, path) = scratch_source();
    let project = tempfile::tempdir().expect("tempdir");
    let mut app = editor(project.path());
    let mut rows = stage_sources(vec![path.clone(), path]);
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
