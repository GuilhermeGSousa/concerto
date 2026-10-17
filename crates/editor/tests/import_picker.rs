//! What the editor does with the picker's result, without opening a picker.
use concerto_app::{App, schedule_groups::Update};
use concerto_editor::{
    asset_editor::AssetEditorCommands,
    guard::UnsavedGuard,
    import::{ImportPicker, ImportPlugin, ImportStaging, RowState, accept_picked},
    project::{EditorCommands, ProjectPlugin, ProjectState, discover_project},
};
use concerto_foundation::assets::asset_server::AssetServer;
use std::path::{Path, PathBuf};

fn state(root: &Path) -> ProjectState {
    std::fs::create_dir_all(root.join("content")).expect("content");
    let mut state = ProjectState::default();
    state.project = Some(discover_project(root).expect("discover"));
    state
}

fn editor(state: ProjectState) -> App {
    let mut app = App::new();
    app.insert_resource(EditorCommands::default());
    app.insert_resource(AssetEditorCommands::default());
    app.insert_resource(UnsavedGuard::default());
    app.insert_resource(AssetServer::new());
    app.insert_resource(concerto_ui::theme::UITheme::default());
    app.insert_resource(state);
    app.register_plugin(ProjectPlugin);
    app.register_plugin(ImportPlugin);
    app.finish_plugin_build();
    app
}

fn frame(app: &mut App) {
    app.main_mut().world_mut().run_schedule(Update);
}

#[test]
fn a_cancelled_pick_opens_no_dialog_and_says_so() {
    let project = tempfile::tempdir().expect("tempdir");
    let mut state = state(project.path());
    let mut staging = ImportStaging::default();
    accept_picked(None, &mut staging, &mut state);
    assert!(!staging.visible);
    assert!(staging.rows.is_empty());
    assert!(
        state.status.to_lowercase().contains("cancel"),
        "got: {}",
        state.status
    );
}

#[test]
fn picked_files_open_the_dialog_already_validated() {
    let project = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(project.path().join("assets")).expect("assets");
    std::fs::write(project.path().join("assets/hero.obj"), b"old").expect("write");
    let mut state = state(project.path());
    let mut staging = ImportStaging::default();
    accept_picked(
        Some(vec![
            PathBuf::from("/elsewhere/hero.obj"),
            PathBuf::from("/elsewhere/notes.txt"),
        ]),
        &mut staging,
        &mut state,
    );
    assert!(staging.visible);
    assert_eq!(staging.rows[0].destination, "assets/hero.obj");
    assert_eq!(staging.rows[0].state, RowState::Replaces);
    assert!(matches!(staging.rows[1].state, RowState::Rejected(_)));
}

#[test]
fn an_empty_pick_opens_no_dialog() {
    let project = tempfile::tempdir().expect("tempdir");
    let mut state = state(project.path());
    let mut staging = ImportStaging::default();
    accept_picked(Some(Vec::new()), &mut staging, &mut state);
    assert!(!staging.visible);
}

#[test]
fn no_picker_opens_when_no_project_is_open() {
    let mut app = editor(ProjectState::default());
    app.get_resource_mut::<ImportPicker>()
        .expect("picker")
        .request();
    frame(&mut app);
    assert!(
        !app.get_resource::<ImportPicker>()
            .expect("picker")
            .is_picking()
    );
}

#[test]
fn no_picker_opens_while_the_dialog_is_visible() {
    let project = tempfile::tempdir().expect("tempdir");
    let mut app = editor(state(project.path()));
    app.get_resource_mut::<ImportStaging>()
        .expect("staging")
        .visible = true;
    app.get_resource_mut::<ImportPicker>()
        .expect("picker")
        .request();
    frame(&mut app);
    assert!(
        !app.get_resource::<ImportPicker>()
            .expect("picker")
            .is_picking()
    );
}

#[test]
fn a_frame_without_a_request_is_harmless() {
    let project = tempfile::tempdir().expect("tempdir");
    let mut app = editor(state(project.path()));
    frame(&mut app);
    assert!(
        !app.get_resource::<ImportPicker>()
            .expect("picker")
            .is_picking()
    );
}

#[test]
fn a_source_already_inside_the_project_is_staged_where_it_is() {
    let project = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(project.path().join("assets/props")).expect("props");
    let inside = project.path().join("assets/props/hero.obj");
    std::fs::write(&inside, b"o Hero\n").expect("write");
    let mut state = state(project.path());
    let mut staging = ImportStaging::default();
    accept_picked(Some(vec![inside]), &mut staging, &mut state);
    assert_eq!(staging.rows[0].destination, "assets/props/hero.obj");
    assert_eq!(
        staging.rows[0].state,
        RowState::Replaces,
        "it is imported in place rather than duplicated under assets/"
    );
}

#[test]
fn a_source_outside_the_project_is_still_staged_under_assets() {
    let project = tempfile::tempdir().expect("tempdir");
    let elsewhere = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(elsewhere.path().join("assets/props")).expect("props");
    let outside = elsewhere.path().join("assets/props/hero.obj");
    std::fs::write(&outside, b"o Hero\n").expect("write");
    let mut state = state(project.path());
    let mut staging = ImportStaging::default();
    accept_picked(Some(vec![outside]), &mut staging, &mut state);
    assert_eq!(staging.rows[0].destination, "assets/hero.obj");
    assert_eq!(staging.rows[0].state, RowState::New);
}
