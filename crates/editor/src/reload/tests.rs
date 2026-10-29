use std::sync::mpsc;

use crate::{
    asset_editor::SaveRequested,
    hosted::testing::SceneFixture,
    project::ProjectState,
    reload::{
        BuildDiagnostic, GameReload, ReloadFailed, ReloadGameCode, ReloadPhase, ReloadTarget,
        cargo::{BuildJob, BuildUpdate, CargoEvent},
    },
};

fn reload(f: &SceneFixture) -> &GameReload {
    f.world.get_resource::<GameReload>().unwrap()
}

fn error(message: &str, line: u32) -> BuildDiagnostic {
    BuildDiagnostic {
        message: message.into(),
        location: Some(("src/lib.rs".into(), line, 5)),
        rendered: format!("error: {message}\n --> src/lib.rs:{line}:5\n"),
    }
}

fn fake_build(f: &mut SceneFixture, events: Vec<CargoEvent>) {
    let (sender, updates) = mpsc::channel();
    for event in events {
        sender.send(BuildUpdate::Cargo(event)).unwrap();
    }
    drop(sender);
    f.world.get_resource_mut::<GameReload>().unwrap().phase =
        ReloadPhase::Building(Box::new(BuildJob::for_test(updates, "game-editor")));
}

#[test]
fn a_failed_build_keeps_the_editor_running_and_lists_its_errors() {
    let mut f = SceneFixture::new();
    let document = f.open();
    fake_build(
        &mut f,
        vec![
            CargoEvent::Error(error("cannot find value `x` in this scope", 2)),
            CargoEvent::Warning,
            CargoEvent::Error(error("cannot find value `y` in this scope", 3)),
            CargoEvent::Finished { success: false },
        ],
    );

    f.frames_until("the build never ended", |f| {
        !matches!(reload(f).phase, ReloadPhase::Building(_))
    });

    assert!(matches!(reload(&f).phase, ReloadPhase::Failed));
    assert_eq!(
        reload(&f).status(),
        Some(("Build failed: 2 errors".to_string(), true))
    );
    assert_eq!(
        f.written::<ReloadFailed>(),
        [ReloadFailed {
            message: "Build failed: 2 errors".into()
        }]
    );

    f.rename(document, "first", "renamed");
    assert!(f.unsaved(document));
    f.world.insert(SaveRequested, document);
    f.frame();
    assert!(!f.unsaved(document));
    assert_eq!(f.saved_scene_names(), ["renamed", "second"]);
}

#[test]
fn a_reload_with_no_project_open_is_refused() {
    let mut f = SceneFixture::new();
    let root = f.project_root.clone();
    f.world.get_resource_mut::<GameReload>().unwrap().target = Ok(ReloadTarget::for_test(
        std::path::Path::new("cargo"),
        &root.join("editor"),
        "game-editor",
        &root.join("target"),
    ));
    f.world.get_resource_mut::<ProjectState>().unwrap().project = None;

    f.world.trigger(ReloadGameCode);

    assert!(matches!(reload(&f).phase, ReloadPhase::Idle));
    assert_eq!(
        reload(&f).status(),
        Some(("Open a project before reloading".to_string(), true))
    );
    assert_eq!(
        f.written::<ReloadFailed>(),
        [ReloadFailed {
            message: "Open a project before reloading".into()
        }]
    );
}

#[test]
fn a_reload_target_that_could_not_be_located_is_refused_with_its_reason() {
    let mut f = SceneFixture::new();
    f.world.get_resource_mut::<GameReload>().unwrap().target =
        Err("Reload is not available under --mcp".into());

    f.world.trigger(ReloadGameCode);

    assert!(matches!(reload(&f).phase, ReloadPhase::Idle));
    assert_eq!(
        f.written::<ReloadFailed>(),
        [ReloadFailed {
            message: "Reload is not available under --mcp".into()
        }]
    );
}

#[test]
fn a_successful_build_ends_idle_with_game_code_built() {
    let mut f = SceneFixture::new();
    f.open();
    fake_build(
        &mut f,
        vec![
            CargoEvent::Compiled {
                target_name: "game-editor".into(),
                fresh: false,
                executable: Some("/work/target/debug/game-editor".into()),
            },
            CargoEvent::Finished { success: true },
        ],
    );

    f.frames_until("the build never ended", |f| {
        !matches!(reload(f).phase, ReloadPhase::Building(_))
    });

    assert!(matches!(reload(&f).phase, ReloadPhase::Idle));
    assert_eq!(
        reload(&f).status(),
        Some(("Game code built".to_string(), false))
    );
    assert!(f.written::<ReloadFailed>().is_empty());
}
