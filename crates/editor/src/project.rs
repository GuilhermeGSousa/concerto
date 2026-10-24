use concerto_app::{App, Plugin, schedule_groups::Update};
use concerto_ecs::{Event, ResMut, Resource, events::event_writer::EventWriter};
use concerto_foundation::assets::{
    AssetId,
    asset_server::AssetServer,
    content::{AssetRegistry, ImportProvenance, read_content_asset_header},
};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    thread::JoinHandle,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetEntry {
    pub id: AssetId,
    pub address: String,
    pub kind: String,
    pub display_name: String,
    pub folder: String,
    /// `None` for an asset authored in the editor rather than produced by `import`.
    pub provenance: Option<ImportProvenance>,
}

impl AssetEntry {
    /// Builds an entry whose display name and folder come from its project-relative `address`.
    pub fn from_address(
        id: AssetId,
        address: &str,
        kind: String,
        provenance: Option<ImportProvenance>,
    ) -> Self {
        let path = Path::new(address);
        let display_name = path
            .file_stem()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_else(|| address.to_owned());
        let folder = path
            .parent()
            .map(|value| value.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        Self {
            id,
            address: address.to_owned(),
            kind,
            display_name,
            folder,
            provenance,
        }
    }

    fn sort_key(&self) -> (&String, &String, &String, &String) {
        (&self.folder, &self.display_name, &self.kind, &self.address)
    }
}

#[derive(Debug)]
pub struct Project {
    pub root: PathBuf,
    pub assets: Vec<AssetEntry>,
    pub registry: AssetRegistry,
    /// The project-relative folder the catalogue is read from, `content` unless `content.toml` names another.
    pub content_root: String,
}

impl Project {
    pub fn scenes(&self) -> impl Iterator<Item = &AssetEntry> {
        self.assets.iter().filter(|asset| asset.kind == "Scene")
    }

    /// Adds `entry` to the catalogue and registry, replacing any entry with the same id.
    pub fn insert(&mut self, entry: AssetEntry) {
        self.registry.insert(entry.id, entry.address.clone());
        self.assets.retain(|asset| asset.id != entry.id);
        let at = self
            .assets
            .partition_point(|asset| asset.sort_key() < entry.sort_key());
        self.assets.insert(at, entry);
    }

    pub fn filtered_assets<'a>(
        &'a self,
        query: &str,
        folder: Option<&str>,
        kind: Option<&str>,
    ) -> Vec<&'a AssetEntry> {
        let query = query.to_lowercase();
        self.assets
            .iter()
            .filter(|asset| {
                folder.is_none_or(|folder| {
                    folder.is_empty()
                        || asset.folder == folder
                        || asset
                            .folder
                            .strip_prefix(folder)
                            .is_some_and(|suffix| suffix.starts_with('/'))
                })
            })
            .filter(|asset| kind.is_none_or(|kind| asset.kind == kind))
            .filter(|asset| {
                query.is_empty()
                    || asset.display_name.to_lowercase().contains(&query)
                    || asset.address.to_lowercase().contains(&query)
                    || asset.kind.to_lowercase().contains(&query)
            })
            .collect()
    }
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
pub fn discover_project(root: &Path) -> anyhow::Result<Project> {
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
    let mut assets = Vec::new();
    for (id, address) in registry.iter() {
        let header = read_content_asset_header(&root.join(address))?;
        assets.push(AssetEntry::from_address(
            id,
            address,
            header.kind,
            header.provenance,
        ));
    }
    assets.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
    Ok(Project {
        root,
        assets,
        registry,
        content_root: config.root,
    })
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
    worker: JoinHandle<Result<Option<Project>, String>>,
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
            Ok(Some(project)) => {
                if let Err(error) =
                    asset_server.publish_project_content(&project.root, project.registry.clone())
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
                    project.assets.len(),
                    project.scenes().count(),
                    project.root.display()
                );
                opened.write(ProjectOpened {
                    root: project.root.clone(),
                });
                state.project = Some(project);
                cmd.trigger(crate::scene::PublishToHosted);
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
                if let Some(asset) = state
                    .project
                    .as_ref()
                    .and_then(|project| project.assets.iter().find(|asset| asset.id == id))
                {
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
                    Ok(project) => {
                        if let Err(error) = asset_server
                            .publish_project_content(&project.root, project.registry.clone())
                        {
                            state.status = format!("Could not refresh project assets: {error:#}");
                            continue;
                        }
                        state.project = Some(project);
                        cmd.trigger(crate::scene::PublishToHosted);
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
        CONTENT_FORMAT_VERSION, ContentAssetHeader, write_content_asset,
    };
    #[test]
    fn discovers_imported_and_authored_assets_without_writing_registry() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(discover_project(dir.path()).unwrap().scenes().count(), 0);
        std::fs::create_dir(dir.path().join("content")).unwrap();
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
                dir.path().join(format!("content/{name}.gasset")),
                write_content_asset(&header, &[]).unwrap(),
            )
            .unwrap();
        }
        let project = discover_project(dir.path()).unwrap();
        assert_eq!(project.assets.len(), 3);
        let scenes: Vec<_> = project.scenes().collect();
        assert_eq!(scenes.len(), 2);
        assert_eq!(scenes[0].display_name, "authored");
        assert!(scenes[0].provenance.is_none());
        assert_eq!(scenes[1].id, id);
        assert!(scenes[1].provenance.is_some());
        assert_eq!(project.assets[0].folder, "content");
        assert_eq!(project.filtered_assets("mesh", None, None).len(), 1);
        assert_eq!(project.filtered_assets("", None, Some("Scene")).len(), 2);
        assert_eq!(project.filtered_assets("", Some("missing"), None).len(), 0);
        assert!(!dir.path().join("content/.registry.toml").exists());
    }

    #[test]
    fn inserting_keeps_the_catalogue_sorted_and_replaces_by_id() {
        let mut project = Project {
            root: PathBuf::new(),
            assets: vec![],
            registry: AssetRegistry::default(),
            content_root: "content".into(),
        };
        let id = AssetId::new();
        for (id, address) in [
            (AssetId::new(), "content/b.gasset"),
            (id, "content/c.gasset"),
            (AssetId::new(), "content/a.gasset"),
            (id, "content/a/renamed.gasset"),
        ] {
            project.insert(AssetEntry::from_address(id, address, "Scene".into(), None));
        }
        let addresses: Vec<_> = project.assets.iter().map(|a| a.address.as_str()).collect();
        assert_eq!(
            addresses,
            [
                "content/a.gasset",
                "content/b.gasset",
                "content/a/renamed.gasset"
            ]
        );
        assert_eq!(project.registry.get(id), Some("content/a/renamed.gasset"));
        assert_eq!(project.assets[2].display_name, "renamed");
        assert_eq!(project.assets[2].folder, "content/a");
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
        result: Result<Option<Project>, String>,
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
            Ok(Some(Project {
                root: gone.clone(),
                assets: vec![],
                registry: AssetRegistry::default(),
                content_root: "content".into(),
            })),
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
