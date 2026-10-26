use concerto_app::{App, Plugin, schedule_groups::Update};
use concerto_ecs::{Event, ResMut, Resource, events::event_writer::EventWriter};
use concerto_foundation::assets::{AssetId, asset_server::AssetServer, registry::AssetRegistry};
use concerto_scene::scene::Scene;
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    thread::JoinHandle,
};

#[derive(Debug)]
pub struct Project {
    pub root: PathBuf,
    /// The project-relative folder the catalogue is read from, `content` unless `content.toml` names another.
    pub content_root: String,
}

#[derive(serde::Deserialize)]
#[serde(default)]
struct CatalogueConfig {
    root: String,
    extension: String,
}
impl Default for CatalogueConfig {
    fn default() -> Self {
        Self {
            root: "content".into(),
            extension: "gasset".into(),
        }
    }
}

/// Read-only catalogue discovery. Never creates or rewrites a project's registry.
pub fn discover_project(root: &Path) -> anyhow::Result<(Project, AssetRegistry)> {
    let root = root.canonicalize()?;
    anyhow::ensure!(root.is_dir(), "Project must be a directory");
    let config_path = root.join("content.toml");
    let config: CatalogueConfig = if config_path.try_exists()? {
        toml::from_str(&std::fs::read_to_string(config_path)?)?
    } else {
        CatalogueConfig::default()
    };
    anyhow::ensure!(
        !config.root.is_empty()
            && Path::new(&config.root)
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_))),
        "Content root must be project-relative without parent traversal"
    );
    anyhow::ensure!(
        !config.extension.is_empty() && !config.extension.contains(['/', '\\', '.']),
        "Invalid content extension"
    );
    let registry = AssetRegistry::from_content_tree(&root, &config.root, &config.extension)?;
    Ok((
        Project {
            root,
            content_root: config.root,
        },
        registry,
    ))
}

pub enum EditorCommand {
    OpenProject(PathBuf),
    OpenAsset(AssetId),
    RefreshCatalogue,
}

#[derive(Resource, Default)]
pub struct EditorCommands(pub VecDeque<EditorCommand>);

/// The project at `root` finished opening and is the editor's project now.
#[derive(Event, Clone, Debug, PartialEq)]
pub struct ProjectOpened {
    pub root: PathBuf,
}

/// The project asked for at `root` did not open; `message` is the status the editor shows.
#[derive(Event, Clone, Debug, PartialEq)]
pub struct ProjectOpenFailed {
    pub root: PathBuf,
    pub message: String,
}

struct ProjectJob {
    root: PathBuf,
    worker: JoinHandle<Result<Option<(Project, AssetRegistry)>, String>>,
}

#[derive(Resource)]
pub struct ProjectState {
    pub project: Option<Project>,
    pub status: String,
    job: Option<ProjectJob>,
}

impl Default for ProjectState {
    fn default() -> Self {
        Self {
            project: None,
            status: "Choose a project folder to get started.".into(),
            job: None,
        }
    }
}

impl ProjectState {
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    #[cfg(test)]
    pub(crate) fn begin_job_for_test(&mut self, release: std::sync::mpsc::Receiver<()>) {
        self.job = Some(ProjectJob {
            root: PathBuf::new(),
            worker: std::thread::spawn(move || {
                let _ = release.recv();
                Ok(None)
            }),
        });
    }
}

pub struct ProjectPlugin;
impl Plugin for ProjectPlugin {
    fn build(&self, app: &mut App) {
        app.register_event::<ProjectOpened>()
            .register_event::<ProjectOpenFailed>()
            .add_system(Update, process_commands);
    }
}

fn process_commands(
    mut commands: ResMut<EditorCommands>,
    mut state: ResMut<ProjectState>,
    mut registry: ResMut<AssetRegistry>,
    mut documents: ResMut<crate::asset_editor::AssetEditorCommands>,
    mut guard: ResMut<crate::guard::UnsavedGuard>,
    open_documents: concerto_ecs::Query<
        concerto_ecs::Entity,
        (
            concerto_ecs::query::filter::With<crate::asset_editor::EditorDocument>,
            concerto_ecs::query::filter::With<crate::asset_editor::Unsaved>,
        ),
    >,
    asset_server: concerto_ecs::Res<AssetServer>,
    states: concerto_ecs::Query<(concerto_ecs::Entity, &crate::hosted::PlayState)>,
    mut cmd: concerto_ecs::command::CommandQueue,
    mut opened: EventWriter<ProjectOpened>,
    mut failed: EventWriter<ProjectOpenFailed>,
) {
    if state.job.as_ref().is_some_and(|j| j.worker.is_finished()) {
        let job = state.job.take().unwrap();
        let result = job
            .worker
            .join()
            .unwrap_or_else(|_| Err("Project worker failed".into()));
        match result {
            Ok(Some((project, discovered))) => {
                if let Err(error) =
                    asset_server.publish_project_content(&project.root, discovered.clone())
                {
                    state.status = format!("Could not activate project assets: {error:#}");
                    failed.write(ProjectOpenFailed {
                        root: job.root,
                        message: state.status.clone(),
                    });
                    return;
                }
                state.status = format!(
                    "{} assets · {} scenes · {}",
                    discovered.iter().count(),
                    discovered.of_type::<Scene>().count(),
                    project.root.display()
                );
                opened.write(ProjectOpened {
                    root: project.root.clone(),
                });
                state.project = Some(project);
                *registry = discovered;
                documents
                    .0
                    .push_back(crate::asset_editor::AssetEditorCommand::CloseAll);
            }
            Ok(None) => {
                state.status = "Folder selection cancelled.".into();
                failed.write(ProjectOpenFailed {
                    root: job.root,
                    message: state.status.clone(),
                });
            }
            Err(error) => {
                state.status = format!("Could not open project: {error}");
                failed.write(ProjectOpenFailed {
                    root: job.root,
                    message: state.status.clone(),
                });
            }
        }
    }
    while let Some(command) = commands.0.pop_front() {
        if state.busy() {
            continue;
        }
        match command {
            EditorCommand::OpenAsset(id) => {
                if let Some(asset) = registry.get(id) {
                    documents
                        .0
                        .push_back(crate::asset_editor::AssetEditorCommand::Open(asset.clone()));
                }
            }
            EditorCommand::RefreshCatalogue => {
                let Some(root) = state.project.as_ref().map(|project| project.root.clone()) else {
                    continue;
                };
                match discover_project(&root) {
                    Ok((project, discovered)) => {
                        state.project = Some(project);
                        *registry = discovered;
                    }
                    Err(error) => state.status = format!("Could not refresh catalogue: {error:#}"),
                }
            }
            EditorCommand::OpenProject(path) => {
                crate::guard::stop_every_game(&states, &mut cmd);
                let dirty = crate::guard::dirty_documents(&open_documents);
                if !dirty.is_empty() {
                    guard.hold(
                        crate::guard::GuardedIntent::Project(EditorCommand::OpenProject(path)),
                        dirty,
                    );
                    continue;
                }
                state.status = "Opening project…".into();
                let root = path.clone();
                state.job = Some(ProjectJob {
                    root,
                    worker: std::thread::spawn(move || {
                        discover_project(&path)
                            .map(Some)
                            .map_err(|e| format!("{e:#}"))
                    }),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use concerto_foundation::assets::content::{
        CONTENT_FORMAT_VERSION, ContentAssetHeader, ImportProvenance, write_content_asset,
    };
    #[test]
    fn discovers_imported_and_authored_assets_without_writing_registry() {
        let dir = tempfile::tempdir().unwrap();
        let (_, empty) = discover_project(dir.path()).unwrap();
        assert_eq!(empty.of_type::<Scene>().count(), 0);
        let id = write_assets(dir.path());
        let (project, registry) = discover_project(dir.path()).unwrap();
        assert_eq!(project.content_root, "content");
        assert_eq!(registry.iter().count(), 3);
        let scenes: Vec<_> = registry.of_type::<Scene>().collect();
        assert_eq!(scenes.len(), 2);
        assert_eq!(scenes[0].name(), "authored");
        assert!(scenes[0].provenance.is_none());
        assert_eq!(scenes[1].id, id);
        assert!(scenes[1].provenance.is_some());
        assert_eq!(registry.iter().next().unwrap().folder(), "content");
        assert!(!dir.path().join("content/.registry.toml").exists());
    }

    fn write_assets(root: &Path) -> AssetId {
        std::fs::create_dir_all(root.join("content")).unwrap();
        let id = AssetId::new();
        for (name, kind, imported) in [
            ("scene", "Scene", true),
            ("mesh", "Mesh", true),
            ("authored", "Scene", false),
        ] {
            let header = ContentAssetHeader {
                format_version: CONTENT_FORMAT_VERSION,
                asset_id: if name == "scene" { id } else { AssetId::new() },
                references: vec![],
                kind: kind.into(),
                provenance: imported.then(|| ImportProvenance {
                    source: "assets/model.glb".into(),
                    sub_asset: name.into(),
                }),
            };
            std::fs::write(
                root.join(format!("content/{name}.gasset")),
                write_content_asset(&header, &[]).unwrap(),
            )
            .unwrap();
        }
        id
    }

    #[test]
    fn refreshing_the_catalogue_replaces_the_registry_resource() {
        use concerto_ecs::{IntoSystem, System};

        let dir = tempfile::tempdir().unwrap();
        let id = write_assets(dir.path());
        let mut world = project_world();
        world.get_resource_mut::<ProjectState>().unwrap().project = Some(Project {
            root: dir.path().canonicalize().unwrap(),
            content_root: "content".into(),
        });
        world
            .get_resource_mut::<EditorCommands>()
            .unwrap()
            .0
            .push_back(EditorCommand::RefreshCatalogue);
        let mut system = process_commands.into_system();
        system.initialize(&mut world);

        system.run_and_apply((), &mut world);

        let registry = world.get_resource::<AssetRegistry>().unwrap();
        assert_eq!(registry.iter().count(), 3);
        assert_eq!(registry.get(id).unwrap().name(), "scene");
    }

    #[test]
    fn rejects_non_directory_and_corrupt_content() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("content")).unwrap();
        let path = dir.path().join("content/broken.gasset");
        std::fs::write(&path, b"invalid").unwrap();
        assert!(discover_project(&path).is_err());
        assert!(discover_project(dir.path()).is_err());
    }

    fn project_world() -> concerto_ecs::World {
        use concerto_ecs::events::event_channel::EventChannel;
        let mut world = concerto_ecs::World::new();
        world.insert_resource(EditorCommands::default());
        world.insert_resource(ProjectState::default());
        world.insert_resource(crate::asset_editor::AssetEditorCommands::default());
        world.insert_resource(crate::guard::UnsavedGuard::default());
        world.insert_resource(AssetServer::new());
        world.insert_resource(AssetRegistry::default());
        world.insert_resource(EventChannel::<ProjectOpened>::new());
        world.insert_resource(EventChannel::<ProjectOpenFailed>::new());
        world
    }

    fn written<E: Event + Clone + 'static>(world: &concerto_ecs::World) -> Vec<E> {
        world
            .get_resource::<concerto_ecs::events::event_channel::EventChannel<E>>()
            .unwrap()
            .read_from(&mut concerto_ecs::events::event_cursor::EventCursor::default())
            .cloned()
            .collect()
    }

    fn settle(world: &mut concerto_ecs::World, command: Option<EditorCommand>) {
        use concerto_ecs::{IntoSystem, System};
        let mut system = process_commands.into_system();
        system.initialize(world);
        if let Some(command) = command {
            world
                .get_resource_mut::<EditorCommands>()
                .unwrap()
                .0
                .push_back(command);
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            system.run_and_apply((), world);
            if !world.get_resource::<ProjectState>().unwrap().busy() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the project never opened"
            );
            std::thread::yield_now();
        }
        system.run_and_apply((), world);
    }

    fn finished_job(
        world: &mut concerto_ecs::World,
        root: PathBuf,
        result: Result<Option<(Project, AssetRegistry)>, String>,
    ) {
        world.get_resource_mut::<ProjectState>().unwrap().job = Some(ProjectJob {
            root,
            worker: std::thread::spawn(move || result),
        });
    }

    fn status(world: &concerto_ecs::World) -> String {
        world.get_resource::<ProjectState>().unwrap().status.clone()
    }

    #[test]
    fn an_opened_project_is_reported_once() {
        let dir = tempfile::tempdir().unwrap();
        let mut world = project_world();

        settle(
            &mut world,
            Some(EditorCommand::OpenProject(dir.path().into())),
        );

        assert_eq!(
            written::<ProjectOpened>(&world),
            [ProjectOpened {
                root: dir.path().canonicalize().unwrap()
            }]
        );
        assert!(written::<ProjectOpenFailed>(&world).is_empty());
    }

    #[test]
    fn a_project_that_cannot_be_read_is_reported_failed_once_with_its_status() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing");
        let mut world = project_world();

        settle(
            &mut world,
            Some(EditorCommand::OpenProject(missing.clone())),
        );

        assert!(status(&world).starts_with("Could not open project"));
        assert_eq!(
            written::<ProjectOpenFailed>(&world),
            [ProjectOpenFailed {
                root: missing,
                message: status(&world)
            }]
        );
        assert!(written::<ProjectOpened>(&world).is_empty());
    }

    #[test]
    fn a_cancelled_folder_pick_is_reported_failed_with_its_message() {
        let mut world = project_world();
        finished_job(&mut world, "picked".into(), Ok(None));

        settle(&mut world, None);

        assert_eq!(
            written::<ProjectOpenFailed>(&world),
            [ProjectOpenFailed {
                root: "picked".into(),
                message: "Folder selection cancelled.".into()
            }]
        );
        assert!(written::<ProjectOpened>(&world).is_empty());
    }

    #[test]
    fn a_project_whose_assets_cannot_be_activated_is_reported_failed_once() {
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("gone");
        let mut world = project_world();
        finished_job(
            &mut world,
            gone.clone(),
            Ok(Some((
                Project {
                    root: gone.clone(),
                    content_root: "content".into(),
                },
                AssetRegistry::default(),
            ))),
        );

        settle(&mut world, None);

        assert!(status(&world).starts_with("Could not activate project assets"));
        assert_eq!(
            written::<ProjectOpenFailed>(&world),
            [ProjectOpenFailed {
                root: gone,
                message: status(&world)
            }]
        );
        assert!(written::<ProjectOpened>(&world).is_empty());
        assert!(
            world
                .get_resource::<ProjectState>()
                .unwrap()
                .project
                .is_none()
        );
    }

    #[test]
    fn opening_a_project_is_held_only_while_a_document_is_unsaved() {
        use crate::asset_editor::{EditorDocument, Unsaved};
        use crate::guard::UnsavedGuard;
        use concerto_ecs::{IntoSystem, System, World};

        let dir = tempfile::tempdir().unwrap();
        let mut world = project_world();
        let document = world.spawn(EditorDocument {
            asset_type: "Scene",
            title: "level".into(),
            current: None,
            pending: None,
            order: 0,
            status: String::new(),
        });
        world.insert(Unsaved, document);
        let mut system = process_commands.into_system();
        system.initialize(&mut world);
        let mut open = |world: &mut World| {
            world
                .get_resource_mut::<EditorCommands>()
                .unwrap()
                .0
                .push_back(EditorCommand::OpenProject(dir.path().into()));
            system.run_and_apply((), world);
        };

        open(&mut world);
        assert_eq!(
            world.get_resource::<UnsavedGuard>().unwrap().documents(),
            [document]
        );
        assert!(!world.get_resource::<ProjectState>().unwrap().busy());

        world.remove_component::<Unsaved>(document);
        world.insert_resource(UnsavedGuard::default());
        open(&mut world);
        assert!(!world.get_resource::<UnsavedGuard>().unwrap().asking());
        assert!(world.get_resource::<ProjectState>().unwrap().busy());
    }

    #[test]
    fn opening_a_project_while_playing_stops_the_game_first() {
        use crate::hosted::{PlayState, testing::SceneFixture};
        use concerto_ecs::{IntoSystem, System, events::event_channel::EventChannel};

        let mut f = SceneFixture::new();
        let document = f.open();
        f.world.trigger(crate::scene::Play(document));
        f.frames_until_playing(document);
        f.world.insert_resource(EditorCommands::default());
        f.world.insert_resource(AssetServer::new());
        f.world.insert_resource(AssetRegistry::default());
        f.world
            .insert_resource(EventChannel::<ProjectOpened>::new());
        f.world
            .insert_resource(EventChannel::<ProjectOpenFailed>::new());
        f.world
            .get_resource_mut::<EditorCommands>()
            .unwrap()
            .0
            .push_back(EditorCommand::OpenProject(f.project_root.clone()));
        let mut system = process_commands.into_system();
        system.initialize(&mut f.world);

        system.run_and_apply((), &mut f.world);

        assert_eq!(f.play_state(document), Some(PlayState::Editing));
        assert!(f.hosted_is_editing(document));
    }
}
