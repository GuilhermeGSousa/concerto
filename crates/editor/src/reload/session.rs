use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use concerto_ecs::{
    Entity, Query, Res, ResMut, Resource, World,
    command::CommandQueue,
    events::event_reader::EventReader,
    signal::{On, Signal},
};
use concerto_foundation::assets::{Asset, AssetId, registry::AssetRegistry};
use concerto_scene::scene::Scene;
use concerto_window::plugin::{Window, WindowCommand};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::GameReload;
use crate::{
    asset_editor::{ActiveEditor, AssetEditorCommand, AssetEditorCommands, EditorDocument},
    hosted::HostedApp,
    project::{ProjectOpenFailed, ProjectOpened, ProjectState},
    scene::{SceneOpenFailed, SceneOpened},
    tools::scene::{CameraState, RestoreView},
};

/// The session format this editor writes and reads.
pub const SESSION_FORMAT: u32 = 1;
const SESSION_FILE: &str = "session.json";

/// What a reload carries into the restarted editor: the project, the window, and the open documents.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Session {
    /// [`SESSION_FORMAT`] when it was written.
    pub format: u32,
    /// The root of the project that was open.
    pub project: PathBuf,
    /// Milliseconds since the Unix epoch when the reload's first build began.
    pub reload_started_at: u64,
    /// The window's size and whether it was maximised.
    pub window: WindowLayout,
    /// The open documents, in tab order.
    pub documents: Vec<DocumentSession>,
    /// The index in `documents` of the front tab.
    pub active: Option<usize>,
}

/// The editor window's size in logical pixels and whether it is maximised.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowLayout {
    /// Width and height in logical pixels.
    pub logical_size: [f32; 2],
    /// Whether the window is maximised.
    pub maximised: bool,
}

/// One open document: the asset it shows, its asset type, and for a scene what it was looking at.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DocumentSession {
    /// The asset the document shows.
    pub asset: AssetId,
    /// The asset type of the document's editor.
    pub kind: String,
    /// The selection and the camera of a Scene document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene: Option<SceneSession>,
}

/// The selection and the camera of a Scene document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SceneSession {
    /// The child indices from the scene root to the selected entity, or `None` for no selection.
    pub selection: Option<Vec<usize>>,
    /// The editor camera, or `None` when the edit `App` has no renderer.
    pub camera: Option<CameraState>,
}

impl Session {
    /// Creates `dir` and writes the session into it as `session.json`.
    pub fn write(&self, dir: &Path) -> anyhow::Result<()> {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("could not create {}", dir.display()))?;
        let path = dir.join(SESSION_FILE);
        std::fs::write(&path, serde_json::to_vec_pretty(self)?)
            .with_context(|| format!("could not write {}", path.display()))
    }

    /// Reads the session in `dir`, refusing one of another format.
    pub fn read(dir: &Path) -> anyhow::Result<Session> {
        let path = dir.join(SESSION_FILE);
        let bytes =
            std::fs::read(&path).with_context(|| format!("could not read {}", path.display()))?;
        let value: Value = serde_json::from_slice(&bytes)
            .with_context(|| format!("{} is not a session", path.display()))?;
        let format = value.get("format").and_then(Value::as_u64);
        anyhow::ensure!(
            format == Some(u64::from(SESSION_FORMAT)),
            "the session is format {}, and this editor reads format {SESSION_FORMAT}",
            format.map_or_else(|| "unknown".to_string(), |format| format.to_string())
        );
        serde_json::from_value(value)
            .with_context(|| format!("{} is not a session", path.display()))
    }
}

fn id_text(id: AssetId) -> String {
    serde_json::to_value(id)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("{id:?}"))
}

pub(crate) fn unix_millis(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH).map_or(0, |since| {
        u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
    })
}

pub(crate) fn capture(world: &mut World) -> Result<Session, String> {
    let project = world
        .get_resource::<ProjectState>()
        .and_then(|state| state.project.as_ref())
        .map(|project| project.root.clone())
        .ok_or("no project is open")?;
    let window = world
        .get_resource::<Window>()
        .map(|window| WindowLayout {
            logical_size: window.logical_size().to_array(),
            maximised: window.is_maximised(),
        })
        .ok_or("the editor has no window")?;
    let active = world
        .get_resource::<ActiveEditor>()
        .and_then(|active| active.0);
    let reload_started_at = world
        .get_resource::<GameReload>()
        .and_then(|reload| reload.started_at)
        .unwrap_or_else(SystemTime::now);
    let mut open: Vec<(u64, Entity, &'static str, AssetId)> = world
        .query::<(Entity, &EditorDocument), ()>()
        .iter(world)
        .filter_map(|(entity, document)| {
            let asset = document.current.as_ref()?.id;
            Some((document.order, entity, document.asset_type, asset))
        })
        .collect();
    open.sort_by_key(|(order, ..)| *order);
    let mut documents = Vec::new();
    let mut active_index = None;
    for (_, entity, kind, asset) in open {
        let scene = if kind == Scene::name() {
            let Some(hosted) = world.get_component_for_entity::<HostedApp>(entity) else {
                continue;
            };
            let view = hosted.scene_view();
            Some(SceneSession {
                selection: view.selection,
                camera: view.camera,
            })
        } else {
            None
        };
        if active == Some(entity) {
            active_index = Some(documents.len());
        }
        documents.push(DocumentSession {
            asset,
            kind: kind.to_owned(),
            scene,
        });
    }
    Ok(Session {
        format: SESSION_FORMAT,
        project,
        reload_started_at: unix_millis(reload_started_at),
        window,
        documents,
        active: active_index,
    })
}

/// A session the editor is restoring, and how far the restore has come.
#[derive(Resource, Clone, Debug)]
pub struct SessionRestore {
    session: Session,
    dir: PathBuf,
    requested: bool,
    opened: HashSet<AssetId>,
    problems: Vec<String>,
}

impl SessionRestore {
    /// Restores `session`, read from `dir`, once its project opens.
    pub fn new(session: Session, dir: PathBuf) -> Self {
        Self {
            session,
            dir,
            requested: false,
            opened: HashSet::new(),
            problems: Vec::new(),
        }
    }

    /// The root of the project the session was saved in.
    pub fn project(&self) -> &Path {
        &self.session.project
    }

    fn problem(&mut self, asset: AssetId, problem: String) {
        self.opened.insert(asset);
        self.problems.push(problem);
    }

    fn take_scene(&mut self, asset: AssetId) -> Option<SceneSession> {
        if !self.requested || self.opened.contains(&asset) {
            return None;
        }
        let document = self
            .session
            .documents
            .iter_mut()
            .find(|document| document.asset == asset && document.kind == Scene::name())?;
        self.opened.insert(asset);
        document.scene.take()
    }

    fn complete(&self) -> bool {
        self.requested
            && self
                .session
                .documents
                .iter()
                .all(|document| self.opened.contains(&document.asset))
    }
}

pub(crate) fn restore_documents(
    mut opened: EventReader<ProjectOpened>,
    mut failed: EventReader<ProjectOpenFailed>,
    restore: Option<ResMut<SessionRestore>>,
    registry: Res<AssetRegistry>,
    mut editors: ResMut<AssetEditorCommands>,
    window: Option<ResMut<Window>>,
) {
    let Some(mut restore) = restore.filter(|restore| !restore.requested) else {
        return;
    };
    let project = restore.session.project.clone();
    let failure = failed
        .read()
        .filter(|failure| failure.root == project)
        .map(|failure| failure.message.clone())
        .last();
    let open = opened.read().filter(|open| open.root == project).count() > 0;
    if !open && failure.is_none() {
        return;
    }
    restore.requested = true;
    if let Some(mut window) = window {
        let layout = restore.session.window;
        window.send(if layout.maximised {
            WindowCommand::SetMaximised(true)
        } else {
            WindowCommand::RequestSize(glam::Vec2::from_array(layout.logical_size))
        });
    }
    let documents = restore.session.documents.clone();
    if !open {
        let message = failure.unwrap_or_default();
        for document in documents {
            restore.problem(
                document.asset,
                format!("{} was not reopened: {message}", id_text(document.asset)),
            );
        }
        return;
    }
    for document in &documents {
        match registry.get(document.asset) {
            Some(entry) => {
                editors.0.push_back(AssetEditorCommand::Open(entry.clone()));
                if document.kind != Scene::name() {
                    restore.opened.insert(document.asset);
                }
            }
            None => restore.problem(
                document.asset,
                format!("{} is no longer in the project", id_text(document.asset)),
            ),
        }
    }
    if let Some(entry) = restore
        .session
        .active
        .and_then(|index| documents.get(index))
        .and_then(|document| registry.get(document.asset))
    {
        editors.0.push_back(AssetEditorCommand::Open(entry.clone()));
    }
}

pub(crate) fn finish_restore(
    mut opened: EventReader<SceneOpened>,
    mut failed: EventReader<SceneOpenFailed>,
    restore: Option<ResMut<SessionRestore>>,
    mut reload: ResMut<GameReload>,
    mut cmd: CommandQueue,
) {
    let Some(mut restore) = restore else {
        return;
    };
    for SceneOpened { document, asset } in opened.read() {
        if let Some(view) = restore.take_scene(asset.id) {
            cmd.trigger(ApplySceneView {
                document: *document,
                view,
            });
        }
    }
    for SceneOpenFailed { asset, message, .. } in failed.read() {
        if restore.take_scene(asset.id).is_some() {
            restore
                .problems
                .push(format!("{}: {message}", asset.address));
        }
    }
    if !restore.complete() {
        return;
    }
    if let Some(first) = restore.problems.first() {
        for problem in &restore.problems {
            log::warn!("{problem}");
        }
        reload.show(
            format!(
                "Reloaded with problems: {first}; the session is kept in {}",
                restore.dir.display()
            ),
            true,
        );
    } else {
        if let Err(error) = std::fs::remove_dir_all(&restore.dir) {
            log::warn!(
                "Could not delete the restored session in {}: {error}",
                restore.dir.display()
            );
        }
        let took = unix_millis(SystemTime::now()).saturating_sub(restore.session.reload_started_at);
        reload.show(
            format!("Game code reloaded in {:.1} s", took as f64 / 1000.0),
            false,
        );
    }
    cmd.trigger(EndRestore);
}

struct EndRestore;
impl Signal for EndRestore {}

fn end_restore(_: On<EndRestore>, world: &mut World) {
    world.remove_resource::<SessionRestore>();
}

pub(crate) struct ApplySceneView {
    pub(crate) document: Entity,
    pub(crate) view: SceneSession,
}
impl Signal for ApplySceneView {}

fn apply_scene_view(on: On<ApplySceneView>, documents: Query<&mut HostedApp>) {
    let ApplySceneView { document, view } = on.signal();
    if let Some(mut hosted) = documents.get_entity(*document) {
        hosted.trigger(RestoreView {
            selection: view.selection.clone(),
            camera: view.camera,
        });
    }
}

pub(crate) fn install_listeners(world: &mut World) {
    world.add_listener(apply_scene_view);
    world.add_listener(end_restore);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        Session {
            format: SESSION_FORMAT,
            project: "/home/me/scratch/tech-demo".into(),
            reload_started_at: 1_761_650_000_123,
            window: WindowLayout {
                logical_size: [1600.0, 900.0],
                maximised: true,
            },
            documents: vec![
                DocumentSession {
                    asset: AssetId::new(),
                    kind: "Scene".into(),
                    scene: Some(SceneSession {
                        selection: None,
                        camera: Some(CameraState {
                            position: [0.0, 2.5, 8.0],
                            yaw: 0.3,
                            pitch: -0.15,
                            speed: 6.0,
                        }),
                    }),
                },
                DocumentSession {
                    asset: AssetId::new(),
                    kind: "Scene".into(),
                    scene: Some(SceneSession {
                        selection: Some(vec![]),
                        camera: None,
                    }),
                },
                DocumentSession {
                    asset: AssetId::new(),
                    kind: "TestAsset".into(),
                    scene: None,
                },
            ],
            active: Some(1),
        }
    }

    #[test]
    fn a_session_round_trips_through_its_directory() {
        let directory = tempfile::tempdir().unwrap();
        let dir = directory.path().join("sessions").join("game-editor-1");

        let written = session();

        written.write(&dir).unwrap();

        assert_eq!(Session::read(&dir).unwrap(), written);
    }

    #[test]
    fn a_session_directory_holds_only_its_json() {
        let directory = tempfile::tempdir().unwrap();
        let dir = directory.path().join("game-editor-1");

        session().write(&dir).unwrap();

        let entries: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(entries, ["session.json"]);
    }

    #[test]
    fn a_session_of_another_format_is_refused() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("session.json"),
            r#"{"format": 2, "project": "/elsewhere"}"#,
        )
        .unwrap();

        let error = Session::read(directory.path()).unwrap_err();

        assert_eq!(
            error.to_string(),
            "the session is format 2, and this editor reads format 1"
        );
    }

    #[test]
    fn a_missing_or_garbled_session_is_an_error() {
        let directory = tempfile::tempdir().unwrap();
        assert!(Session::read(directory.path()).is_err());

        std::fs::write(directory.path().join("session.json"), "{ not json").unwrap();
        assert!(Session::read(directory.path()).is_err());

        std::fs::write(
            directory.path().join("session.json"),
            r#"{"format": 1, "project": 3}"#,
        )
        .unwrap();
        assert!(Session::read(directory.path()).is_err());
    }
}
