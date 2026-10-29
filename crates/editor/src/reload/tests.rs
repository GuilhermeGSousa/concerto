use std::{
    path::{Path, PathBuf},
    sync::mpsc,
};

use concerto_ecs::{Entity, component::Tick};
use concerto_window::plugin::{Window, WindowCommand};

use crate::{
    asset_editor::{ActiveEditor, AssetEditorCommand, EditorDocument, SaveRequested, Unsaved},
    guard::{GuardedIntent, UnsavedGuard},
    hosted::{
        PlayState, PlayStopped,
        testing::{SceneFixture, test_asset_entry},
    },
    import::{ImportPicker, ImportQueue, RowState, StagedImport},
    inspector::{PropertyCommit, PropertyCommits},
    project::ProjectState,
    reload::{
        BuildDiagnostic, GameReload, ReloadFailed, ReloadGameCode, ReloadPhase, ReloadTarget,
        Restart, RestartSlot, Session, SessionRestore,
        cargo::{BuildJob, BuildUpdate, CargoEvent},
    },
    scene::{Play, SceneSaved},
    tools::{edits::PropertyRow, selection::Select},
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

struct Reloadable {
    f: SceneFixture,
    target: tempfile::TempDir,
    bins: tempfile::TempDir,
}

fn reload_mut(f: &mut SceneFixture) -> &mut GameReload {
    f.world.get_resource_mut::<GameReload>().unwrap()
}

fn build_into(f: &mut SceneFixture, target_dir: &Path) {
    let manifest_dir = f.project_root.join("editor");
    reload_mut(f).target = Ok(ReloadTarget::for_test(
        Path::new("cargo"),
        &manifest_dir,
        "game-editor",
        target_dir,
    ));
}

fn reloadable() -> Reloadable {
    let mut f = SceneFixture::new();
    let target = tempfile::tempdir().unwrap();
    build_into(&mut f, target.path());
    Reloadable {
        f,
        target,
        bins: tempfile::tempdir().unwrap(),
    }
}

impl Reloadable {
    fn executable(&self, name: &str) -> PathBuf {
        let path = self.bins.path().join(name);
        std::fs::write(&path, "").unwrap();
        path
    }

    fn sessions(&self) -> Vec<PathBuf> {
        let Ok(entries) =
            std::fs::read_dir(self.target.path().join("concerto-editor").join("sessions"))
        else {
            return vec![];
        };
        entries.map(|entry| entry.unwrap().path()).collect()
    }

    fn session(&self) -> PathBuf {
        let sessions = self.sessions();
        assert_eq!(sessions.len(), 1, "{sessions:?}");
        sessions[0].clone()
    }

    fn start(&mut self, first: Vec<CargoEvent>, confirming: Vec<CargoEvent>) {
        reload_mut(&mut self.f)
            .fake_builds
            .push_back(job(confirming));
        reload_mut(&mut self.f).phase = ReloadPhase::Building(Box::new(job(first)));
    }

    fn reload(&mut self) {
        let executable = self.executable("game-editor");
        self.start(built(&executable), built(&executable));
        self.until_settled();
    }

    fn until_settled(&mut self) {
        self.f.frames_until("the reload never settled", |f| {
            matches!(
                reload(f).phase,
                ReloadPhase::Idle | ReloadPhase::Failed | ReloadPhase::Closing
            )
        });
    }

    fn restored(&self) -> SceneFixture {
        let dir = self.session();
        restore_from(&self.f.project_root, &dir, Session::read(&dir).unwrap())
    }
}

fn restore_from(root: &Path, dir: &Path, session: Session) -> SceneFixture {
    let mut g = SceneFixture::on(root);
    g.world
        .insert_resource(SessionRestore::new(session, dir.to_path_buf()));
    g.project_opened();
    g.frames_until("the restore never finished", |g| {
        g.world.get_resource::<SessionRestore>().is_none()
    });
    g
}

fn job(events: Vec<CargoEvent>) -> BuildJob {
    let (sender, updates) = mpsc::channel();
    for event in events {
        sender.send(BuildUpdate::Cargo(event)).unwrap();
    }
    BuildJob::for_test(updates, "game-editor")
}

fn built(executable: &Path) -> Vec<CargoEvent> {
    vec![
        CargoEvent::Compiled {
            target_name: "game-editor".into(),
            fresh: false,
            executable: Some(executable.to_path_buf()),
        },
        CargoEvent::Finished { success: true },
    ]
}

fn window_commands(f: &mut SceneFixture) -> Vec<WindowCommand> {
    f.world
        .get_resource_mut::<Window>()
        .unwrap()
        .drain_commands()
}

fn restart(f: &SceneFixture) -> Option<Restart> {
    f.world.get_resource::<RestartSlot>().unwrap().take()
}

fn active(f: &SceneFixture) -> Entity {
    f.world.get_resource::<ActiveEditor>().unwrap().0.unwrap()
}

fn failures(f: &SceneFixture) -> Vec<String> {
    f.written::<ReloadFailed>()
        .into_iter()
        .map(|failed| failed.message)
        .collect()
}

#[test]
fn a_reload_saves_an_unsaved_scene_and_restores_it_clean() {
    let mut r = reloadable();
    let document = r.f.open();
    r.f.rename(document, "first", "renamed");
    assert!(r.f.unsaved(document));

    r.reload();

    assert!(matches!(reload(&r.f).phase, ReloadPhase::Closing));
    assert_eq!(r.f.saved_scene_names(), ["renamed", "second"]);
    assert_eq!(r.f.written::<SceneSaved>().len(), 1);
    let entries: Vec<_> = std::fs::read_dir(r.session())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(entries, ["session.json"]);

    let mut g = r.restored();
    let document = active(&g);
    assert_eq!(g.scene_names(document), ["renamed", "second"]);
    assert!(!g.unsaved(document));
}

#[test]
fn a_scene_that_cannot_be_saved_stops_the_reload() {
    let mut r = reloadable();
    let document = r.f.open();
    r.f.rename(document, "first", "renamed");
    r.f.make_project_unwritable();

    r.reload();

    assert!(matches!(reload(&r.f).phase, ReloadPhase::Idle));
    let failed = failures(&r.f);
    assert_eq!(failed.len(), 1);
    assert!(
        failed[0].starts_with("Reload failed: could not save first: Save failed: "),
        "{failed:?}"
    );
    assert_eq!(reload(&r.f).status(), Some((failed[0].clone(), true)));
    assert!(r.sessions().is_empty());
    assert_eq!(restart(&r.f), None);
    assert!(!window_commands(&mut r.f).contains(&WindowCommand::Close));
    assert!(r.f.unsaved(document));
}

#[test]
fn a_clean_scene_is_reopened_from_its_file_and_is_not_saved_again() {
    let mut r = reloadable();
    r.f.open();

    r.reload();

    assert!(r.f.written::<SceneSaved>().is_empty());
    let mut g = r.restored();
    let document = active(&g);
    assert_eq!(g.scene_names(document), ["first", "second"]);
    assert!(!g.unsaved(document));
    assert!(g.written::<SceneSaved>().is_empty());
}

#[test]
fn the_selection_survives_a_reload() {
    let mut r = reloadable();
    let document = r.f.open();
    let second = r.f.scene_entity(document, "second");
    r.f.signal(Select(Some(second)));

    r.reload();

    let mut g = r.restored();
    let document = active(&g);
    let second = g.scene_entity(document, "second");
    assert_eq!(g.selected(document), Some(second));
}

fn tabs(f: &mut SceneFixture) -> Vec<(&'static str, concerto_foundation::assets::AssetId)> {
    let mut documents: Vec<_> = f
        .world
        .query::<&EditorDocument, ()>()
        .iter(&mut f.world)
        .map(|document| {
            let asset = document.current.as_ref().or(document.pending.as_ref());
            (document.order, document.asset_type, asset.unwrap().id)
        })
        .collect();
    documents.sort_by_key(|(order, ..)| *order);
    documents
        .into_iter()
        .map(|(_, kind, asset)| (kind, asset))
        .collect()
}

fn open_notes(f: &mut SceneFixture) -> Entity {
    let notes = test_asset_entry();
    f.add_to_project(&notes);
    f.request(&notes);
    let document = active(f);
    let doc = f
        .world
        .get_component_for_entity_mut::<EditorDocument>(document)
        .unwrap();
    doc.current = doc.pending.take();
    document
}

#[test]
fn tabs_and_the_active_tab_survive_a_reload() {
    let mut r = reloadable();
    let scene = r.f.open();
    open_notes(&mut r.f);
    r.f.command(AssetEditorCommand::Activate(scene));
    let before = tabs(&mut r.f);

    r.reload();

    let dir = r.session();
    let mut g = SceneFixture::on(&r.f.project_root);
    g.add_to_project(&test_asset_entry());
    g.world.insert_resource(SessionRestore::new(
        Session::read(&dir).unwrap(),
        dir.clone(),
    ));
    g.project_opened();
    g.frames_until("the restore never finished", |g| {
        g.world.get_resource::<SessionRestore>().is_none()
    });

    assert_eq!(tabs(&mut g), before);
    let front = active(&g);
    assert_eq!(
        g.world
            .get_component_for_entity::<EditorDocument>(front)
            .unwrap()
            .asset_type,
        "Scene"
    );
}

#[test]
fn a_restored_window_size_is_requested() {
    let mut r = reloadable();
    r.f.open();
    r.f.world.insert_resource(Window::new((1600, 900), 2.0));

    r.reload();

    let mut g = r.restored();
    assert_eq!(
        window_commands(&mut g),
        [WindowCommand::RequestSize(glam::Vec2::new(800.0, 450.0))]
    );
}

#[test]
fn a_restored_maximised_window_is_maximised_without_a_size_request() {
    let mut r = reloadable();
    r.f.open();
    r.reload();
    let dir = r.session();
    let mut session = Session::read(&dir).unwrap();
    session.window.maximised = true;

    let mut g = restore_from(&r.f.project_root, &dir, session);

    assert_eq!(window_commands(&mut g), [WindowCommand::SetMaximised(true)]);
}

#[test]
fn a_reload_while_playing_stops_the_game_and_keeps_the_saved_scene() {
    let mut r = reloadable();
    let document = r.f.open();
    r.f.rename(document, "first", "renamed");
    r.f.world.trigger(Play(document));
    r.f.frames_until_playing(document);
    assert_eq!(r.f.written::<SceneSaved>().len(), 1);
    r.f.frames_until("the game never spawned the scene", |f| {
        f.scene_names(document).contains(&"second".to_string())
    });
    r.f.rename(document, "second", "changed while playing");

    r.reload();

    assert!(matches!(reload(&r.f).phase, ReloadPhase::Closing));
    assert_eq!(r.f.written::<PlayStopped>(), [PlayStopped { document }]);
    assert_eq!(r.f.play_state(document), Some(PlayState::Editing));
    assert_eq!(r.f.written::<SceneSaved>().len(), 1);
    let mut g = r.restored();
    let document = active(&g);
    assert_eq!(g.scene_names(document), ["renamed", "second"]);
    assert!(!g.unsaved(document));
}

fn property_commit(f: &mut SceneFixture, document: Entity) -> PropertyCommit {
    PropertyCommit {
        row: PropertyRow {
            entity: f.scene_entity(document, "first"),
            component: std::any::TypeId::of::<concerto_foundation::transform::Transform>(),
            path: Default::default(),
            type_id: std::any::TypeId::of::<f32>(),
            editor_type: None,
            registry_tick: Tick::default(),
        },
        edit: std::sync::Arc::new(()),
    }
}

#[test]
fn a_reload_waits_for_in_flight_work() {
    type Hold = fn(&mut SceneFixture, Entity) -> Box<dyn FnOnce(&mut SceneFixture)>;
    let cases: [(&str, Hold); 4] = [
        ("the question about unsaved changes", |f, document| {
            f.world
                .get_resource_mut::<UnsavedGuard>()
                .unwrap()
                .hold(GuardedIntent::Quit, vec![document]);
            Box::new(|f| f.world.insert_resource(UnsavedGuard::default()))
        }),
        ("imports to finish", |f, _| {
            f.world
                .get_resource_mut::<ImportQueue>()
                .unwrap()
                .enqueue(vec![StagedImport {
                    source: "hero.glb".into(),
                    destination: "assets/hero.glb".into(),
                    state: RowState::New,
                }]);
            Box::new(|f| f.world.insert_resource(ImportQueue::default()))
        }),
        ("the file picker to close", |f, _| {
            let (release, wait) = mpsc::channel();
            f.world
                .insert_resource(ImportPicker::picking_for_test(wait));
            Box::new(move |f| {
                let _ = release.send(());
                f.world.insert_resource(ImportPicker::default());
            })
        }),
        ("edits to apply", |f, document| {
            let commit = property_commit(f, document);
            f.world
                .get_resource_mut::<PropertyCommits>()
                .unwrap()
                .0
                .push(commit);
            Box::new(|f| {
                f.world
                    .get_resource_mut::<PropertyCommits>()
                    .unwrap()
                    .0
                    .clear()
            })
        }),
    ];
    for (reason, hold) in cases {
        let mut r = reloadable();
        let document = r.f.open();
        let release = hold(&mut r.f, document);
        let executable = r.executable("game-editor");
        r.start(built(&executable), built(&executable));

        for _ in 0..5 {
            r.f.frame();
        }

        assert!(
            matches!(reload(&r.f).phase, ReloadPhase::Waiting { .. }),
            "{reason}"
        );
        assert_eq!(
            reload(&r.f).status(),
            Some((format!("Built · waiting for {reason} to restart"), false))
        );
        release(&mut r.f);
        r.until_settled();
        assert!(
            matches!(reload(&r.f).phase, ReloadPhase::Closing),
            "{reason}"
        );
    }
}

#[test]
fn the_restart_runs_the_binary_of_the_confirming_build() {
    let mut r = reloadable();
    r.f.open();
    let a = r.executable("a");
    let b = r.executable("b");

    r.start(built(&a), built(&b));
    r.until_settled();

    assert_eq!(restart(&r.f).unwrap().executable, b);
}

#[test]
fn a_failing_confirming_build_fails_the_reload() {
    let mut r = reloadable();
    r.f.open();
    let executable = r.executable("game-editor");

    r.start(
        built(&executable),
        vec![
            CargoEvent::Error(error("cannot find value `x` in this scope", 2)),
            CargoEvent::Finished { success: false },
        ],
    );
    r.until_settled();

    assert!(matches!(reload(&r.f).phase, ReloadPhase::Failed));
    assert_eq!(failures(&r.f), ["Build failed: 1 error"]);
    assert_eq!(restart(&r.f), None);
    assert!(r.sessions().is_empty());
}

#[test]
fn an_unsaved_document_the_reload_cannot_save_refuses_it() {
    let mut r = reloadable();
    r.f.open();
    let notes = open_notes(&mut r.f);
    r.f.world.insert(Unsaved, notes);

    r.reload();

    assert!(matches!(reload(&r.f).phase, ReloadPhase::Idle));
    assert_eq!(
        failures(&r.f),
        ["Reload failed: save notes first; the reload can only save scenes"]
    );
    assert_eq!(restart(&r.f), None);
    assert!(r.sessions().is_empty());
}

#[test]
fn a_restore_that_cannot_find_a_scene_keeps_the_session_and_says_where() {
    let mut r = reloadable();
    r.f.open();
    r.reload();
    let dir = r.session();
    let mut session = Session::read(&dir).unwrap();
    let missing = concerto_foundation::assets::AssetId::new();
    session.documents[0].asset = missing;
    let id = serde_json::to_value(missing).unwrap();

    let g = restore_from(&r.f.project_root, &dir, session);

    assert_eq!(
        reload(&g).status(),
        Some((
            format!(
                "Reloaded with problems: {} is no longer in the project; the session is kept in {}",
                id.as_str().unwrap(),
                dir.display()
            ),
            true
        ))
    );
    assert!(dir.join("session.json").is_file());
}

#[test]
fn a_complete_restore_deletes_the_session_and_says_how_long_it_took() {
    let mut r = reloadable();
    r.f.open();
    r.reload();
    let dir = r.session();

    let g = r.restored();

    let (status, warning) = reload(&g).status().unwrap();
    assert!(
        status.starts_with("Game code reloaded in ") && status.ends_with(" s"),
        "{status}"
    );
    assert!(!warning);
    assert!(!dir.exists());
}

#[test]
fn a_session_that_cannot_be_written_does_not_restart() {
    let mut r = reloadable();
    r.f.open();
    let file = r.target.path().join("not-a-directory");
    std::fs::write(&file, "").unwrap();
    build_into(&mut r.f, &file);

    r.reload();

    assert!(matches!(reload(&r.f).phase, ReloadPhase::Idle));
    let failed = failures(&r.f);
    assert_eq!(failed.len(), 1);
    assert!(
        failed[0].starts_with("Reload failed: could not save the session: "),
        "{failed:?}"
    );
    assert_eq!(restart(&r.f), None);
    assert!(!window_commands(&mut r.f).contains(&WindowCommand::Close));
}

#[test]
fn a_saved_session_closes_the_window_and_fills_the_restart_slot() {
    let mut r = reloadable();
    r.f.open();

    r.reload();

    assert_eq!(window_commands(&mut r.f), [WindowCommand::Close]);
    assert_eq!(
        restart(&r.f),
        Some(Restart {
            executable: r.bins.path().join("game-editor"),
            session_dir: r.session(),
        })
    );
    assert!(failures(&r.f).is_empty());
}

#[test]
fn closing_saves_the_session_once() {
    let mut r = reloadable();
    r.f.open();
    r.reload();

    for _ in 0..5 {
        r.f.frame();
    }

    assert_eq!(r.sessions().len(), 1);
    assert!(matches!(reload(&r.f).phase, ReloadPhase::Closing));
}

#[test]
fn a_reload_cancelled_while_waiting_returns_to_idle() {
    let mut r = reloadable();
    let document = r.f.open();
    r.f.world
        .get_resource_mut::<UnsavedGuard>()
        .unwrap()
        .hold(GuardedIntent::Quit, vec![document]);
    let executable = r.executable("game-editor");
    r.start(built(&executable), built(&executable));
    r.f.frames_until("the build never ended", |f| {
        matches!(reload(f).phase, ReloadPhase::Waiting { .. })
    });

    r.f.world.trigger(crate::reload::CancelReload);

    assert!(matches!(reload(&r.f).phase, ReloadPhase::Idle));
    assert_eq!(failures(&r.f), ["Reload cancelled"]);
    assert!(r.sessions().is_empty());
}
