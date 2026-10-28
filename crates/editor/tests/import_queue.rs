//! The queue runs imports one at a time and survives a failing row.
use concerto_app::{App, schedule_groups::Update};
use concerto_editor::{
    asset_editor::{AssetEditorCommand, AssetEditorCommands},
    guard::UnsavedGuard,
    import::{ImportPlugin, ImportQueue, stage_sources},
    project::{EditorCommands, ProjectPlugin, ProjectState, discover_project},
};
use concerto_foundation::assets::{
    AssetId,
    asset_server::AssetServer,
    content::{CONTENT_FORMAT_VERSION, ContentAssetHeader, write_content_asset},
    registry::AssetRegistry,
};
use std::path::{Path, PathBuf};

fn scratch_source() -> (tempfile::TempDir, PathBuf) {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../obj/tests/fixtures");
    let dir = tempfile::tempdir().expect("tempdir");
    for name in ["square.obj", "square.mtl"] {
        std::fs::copy(fixtures.join(name), dir.path().join(name)).expect("copy the fixture");
    }
    let path = dir.path().join("square.obj");
    (dir, path)
}

fn named_sources(names: &[&str]) -> (tempfile::TempDir, Vec<PathBuf>) {
    let (dir, original) = scratch_source();
    let paths = names
        .iter()
        .map(|name| {
            let path = dir.path().join(name);
            std::fs::copy(&original, &path).expect("copy source");
            path
        })
        .collect();
    (dir, paths)
}

fn broken_sources(names: &[&str]) -> (tempfile::TempDir, Vec<PathBuf>) {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../obj/tests/fixtures/square.obj");
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = names
        .iter()
        .map(|name| {
            let path = dir.path().join(name);
            std::fs::copy(&fixture, &path).expect("copy the fixture");
            path
        })
        .collect();
    (dir, paths)
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
    assert_eq!(
        app.get_resource::<AssetRegistry>()
            .expect("registry")
            .iter()
            .count(),
        6,
        "both imports reached the catalogue, a mesh, a material and a scene each"
    );
}

#[test]
fn the_outcome_counts_the_files_that_came_along_with_the_source() {
    let (_source, path) = scratch_source();
    let project = tempfile::tempdir().expect("tempdir");
    let mut app = editor(project.path());
    app.get_resource_mut::<ImportQueue>()
        .expect("queue")
        .enqueue(stage_sources(vec![path]));
    drain(&mut app);

    assert!(project.path().join("assets/square.mtl").is_file());
    assert_eq!(
        status(&app),
        "Imported square.obj · 3 assets · 1 sibling file"
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

#[test]
fn a_batch_starts_its_rows_in_the_order_they_were_queued() {
    let (_sources, paths) = named_sources(&["first.obj", "second.obj", "third.obj", "fourth.obj"]);
    let project = tempfile::tempdir().expect("tempdir");
    let mut app = editor(project.path());
    app.get_resource_mut::<ImportQueue>()
        .expect("queue")
        .enqueue(stage_sources(paths));

    let mut started = Vec::new();
    for _ in 0..500 {
        frame(&mut app);
        let status = status(&app);
        if status.starts_with("Importing ") && started.last() != Some(&status) {
            started.push(status);
        }
        let queue = app.get_resource::<ImportQueue>().expect("queue");
        if queue.remaining() == 0 && !queue.is_running() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    assert_eq!(
        started,
        [
            "Importing first.obj (1 of 4)…",
            "Importing second.obj (2 of 4)…",
            "Importing third.obj (3 of 4)…",
            "Importing fourth.obj (4 of 4)…",
        ]
    );
}

#[test]
fn a_failure_leads_the_final_status_in_a_form_that_can_be_read() {
    let (_sources, good) = named_sources(&["ok_a.obj", "ok_b.obj"]);
    let (broken_dir, broken) = broken_sources(&["crate.obj"]);
    let not_found = std::fs::File::open(broken_dir.path().join("square.mtl"))
        .expect_err("the material library is missing")
        .to_string();
    let not_found = not_found
        .split(" (os error")
        .next()
        .expect("split yields at least one part");
    let project = tempfile::tempdir().expect("tempdir");
    let mut app = editor(project.path());
    app.get_resource_mut::<ImportQueue>()
        .expect("queue")
        .enqueue(stage_sources(vec![
            good[0].clone(),
            broken[0].clone(),
            good[1].clone(),
        ]));
    drain(&mut app);

    let status = status(&app);
    assert!(
        status.starts_with("1 of 3 failed: crate.obj — ")
            && status.contains(&format!("square.mtl: {not_found}")),
        "got: {status}"
    );
    assert_eq!(status.matches("crate.obj").count(), 1, "got: {status}");
    let root = project.path().canonicalize().expect("root");
    for noise in [
        root.to_str().expect("utf-8 tempdir"),
        "SourceUnreadable",
        "os error",
        "ok_b.obj",
    ] {
        assert!(!status.contains(noise), "'{noise}' in: {status}");
    }
    assert!(project.path().join("assets/ok_a.obj").is_file());
    assert!(project.path().join("assets/ok_b.obj").is_file());
}

#[test]
fn several_failures_name_the_first_and_count_the_rest() {
    let (_sources, good) = named_sources(&["ok.obj"]);
    let (_broken, broken) = broken_sources(&["one.obj", "two.obj", "three.obj"]);
    let project = tempfile::tempdir().expect("tempdir");
    let mut app = editor(project.path());
    let mut sources = broken;
    sources.push(good[0].clone());
    app.get_resource_mut::<ImportQueue>()
        .expect("queue")
        .enqueue(stage_sources(sources));
    drain(&mut app);

    let status = status(&app);
    assert!(
        status.starts_with("3 of 4 failed: one.obj — "),
        "got: {status}"
    );
    assert!(status.ends_with("(+2 more, see log)"), "got: {status}");
    assert!(
        !status.contains("two.obj") && !status.contains("three.obj"),
        "got: {status}"
    );
}

#[test]
fn rows_still_waiting_when_the_project_is_swapped_are_dropped() {
    let (_sources, paths) = named_sources(&["first.obj", "second.obj", "third.obj"]);
    let project = tempfile::tempdir().expect("tempdir");
    let other = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(other.path().join("content")).expect("content");
    let mut app = editor(project.path());
    app.get_resource_mut::<ImportQueue>()
        .expect("queue")
        .enqueue(stage_sources(paths));
    frame(&mut app);
    assert_eq!(
        app.get_resource::<ImportQueue>()
            .expect("queue")
            .remaining(),
        2
    );

    app.get_resource_mut::<ProjectState>()
        .expect("state")
        .project = Some(discover_project(other.path()).expect("discover").0);
    drain(&mut app);

    assert!(project.path().join("assets/first.obj").is_file());
    assert!(!project.path().join("assets/second.obj").exists());
    assert!(
        !other.path().join("assets").exists(),
        "rows validated against one project never land in another"
    );
    let status = status(&app);
    assert!(
        status.contains("2 queued imports") && status.contains("project"),
        "got: {status}"
    );
}

#[test]
fn rows_queued_with_no_project_open_are_dropped() {
    let (_sources, paths) = named_sources(&["first.obj"]);
    let project = tempfile::tempdir().expect("tempdir");
    let mut app = editor(project.path());
    app.get_resource_mut::<ProjectState>()
        .expect("state")
        .project = None;
    app.get_resource_mut::<ImportQueue>()
        .expect("queue")
        .enqueue(stage_sources(paths));
    frame(&mut app);

    let queue = app.get_resource::<ImportQueue>().expect("queue");
    assert_eq!(queue.remaining(), 0);
    assert!(!queue.is_running());
    let status = status(&app);
    assert!(
        status.contains("1 queued import:") && status.contains("no project"),
        "got: {status}"
    );
}

fn authored_asset(root: &Path, name: &str) -> AssetId {
    let header = ContentAssetHeader {
        format_version: CONTENT_FORMAT_VERSION,
        asset_id: AssetId::new(),
        references: vec![],
        kind: "Scene".into(),
        provenance: None,
    };
    std::fs::create_dir_all(root.join("content")).expect("content");
    std::fs::write(
        root.join(format!("content/{name}.gasset")),
        write_content_asset(&header, &[]).expect("encode"),
    )
    .expect("write");
    header.asset_id
}

#[test]
fn an_import_finishing_adds_its_entries_without_a_rescan_or_closing_documents() {
    let (_source, path) = scratch_source();
    let project = tempfile::tempdir().expect("tempdir");
    let mut app = editor(project.path());
    let unseen = authored_asset(project.path(), "pulled");
    app.get_resource_mut::<ImportQueue>()
        .expect("queue")
        .enqueue(stage_sources(vec![path]));
    drain(&mut app);

    let registry = app.get_resource::<AssetRegistry>().expect("registry");
    let rescanned = discover_project(project.path()).expect("rescan").1;
    assert_eq!(registry.iter().count(), 3, "a mesh, a material and a scene");
    for entry in registry.iter() {
        assert_eq!(
            entry
                .provenance
                .as_ref()
                .map(|provenance| provenance.source.as_str()),
            Some("assets/square.obj")
        );
        assert_eq!(rescanned.get(entry.id), Some(entry));
    }
    assert!(
        registry.get(unseen).is_none(),
        "the catalogue is not rescanned after an import"
    );
    assert!(!project.path().join("content/.registry.toml").exists());
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
fn an_import_whose_project_was_replaced_while_it_ran_adds_nothing_to_the_new_registry() {
    let (_source, path) = scratch_source();
    let project = tempfile::tempdir().expect("tempdir");
    let other = tempfile::tempdir().expect("tempdir");
    authored_asset(other.path(), "level");
    let mut app = editor(project.path());
    app.get_resource_mut::<ImportQueue>()
        .expect("queue")
        .enqueue(stage_sources(vec![path]));
    frame(&mut app);
    assert!(
        app.get_resource::<ImportQueue>()
            .expect("queue")
            .is_running()
    );

    let (opened, registry) = discover_project(other.path()).expect("discover");
    let expected: Vec<_> = registry.iter().cloned().collect();
    app.get_resource_mut::<ProjectState>()
        .expect("state")
        .project = Some(opened);
    app.insert_resource(registry);
    drain(&mut app);

    assert!(project.path().join("assets/square.obj").is_file());
    let registry = app.get_resource::<AssetRegistry>().expect("registry");
    assert_eq!(registry.iter().cloned().collect::<Vec<_>>(), expected);
}
