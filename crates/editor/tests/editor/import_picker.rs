//! What the editor does with the picker's result, without opening a picker.
use concerto_editor::{
    import::{ImportStaging, RowState, accept_picked},
    project::{ProjectState, discover_project},
};
use std::path::PathBuf;

#[test]
fn picked_files_open_the_dialog_validated_with_a_file_inside_the_project_staged_in_place() {
    let project = tempfile::tempdir().expect("tempdir");
    let elsewhere = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(project.path().join("content")).expect("content");
    std::fs::create_dir_all(project.path().join("assets/props")).expect("props");
    let inside = project.path().join("assets/props/hero.obj");
    std::fs::write(&inside, b"o Hero\n").expect("write");
    let outside = elsewhere.path().join("rock.obj");
    std::fs::write(&outside, b"o Rock\n").expect("write");
    let mut state = ProjectState::default();
    state.project = Some(discover_project(project.path()).expect("discover").0);
    let mut staging = ImportStaging::default();

    accept_picked(
        Some(vec![inside, outside, PathBuf::from("/elsewhere/notes.txt")]),
        &mut staging,
        &mut state,
    );

    assert!(staging.visible);
    assert_eq!(staging.rows[0].destination, "assets/props/hero.obj");
    assert_eq!(staging.rows[0].state, RowState::Replaces);
    assert_eq!(staging.rows[1].destination, "assets/rock.obj");
    assert_eq!(staging.rows[1].state, RowState::New);
    assert!(matches!(staging.rows[2].state, RowState::Rejected(_)));
}
