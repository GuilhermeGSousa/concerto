use app::{App, Plugin, schedule_groups::Update};
use ecs::{ResMut, Resource};
use essential::assets::{
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
    pub provenance: ImportProvenance,
}

#[derive(Debug)]
pub struct Project {
    pub root: PathBuf,
    pub assets: Vec<AssetEntry>,
    pub registry: AssetRegistry,
}

impl Project {
    pub fn scenes(&self) -> impl Iterator<Item = &AssetEntry> {
        self.assets.iter().filter(|asset| asset.kind == "Scene")
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
        let Some(provenance) = header.provenance else {
            continue;
        };
        let path = Path::new(address);
        let display_name = path
            .file_stem()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_else(|| address.to_owned());
        let folder = path
            .parent()
            .map(|value| value.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        assets.push(AssetEntry {
            id,
            address: address.to_owned(),
            kind: header.kind,
            display_name,
            folder,
            provenance,
        });
    }
    assets.sort_by(|a, b| {
        (&a.folder, &a.display_name, &a.kind, &a.address).cmp(&(
            &b.folder,
            &b.display_name,
            &b.kind,
            &b.address,
        ))
    });
    Ok(Project {
        root,
        assets,
        registry,
    })
}

pub enum EditorCommand {
    OpenProject(PathBuf),
    OpenAsset(AssetId),
}

#[derive(Resource, Default)]
pub struct EditorCommands(pub VecDeque<EditorCommand>);

#[derive(Resource)]
pub struct ProjectState {
    pub project: Option<Project>,
    pub status: String,
    pub generation: u64,
    job: Option<JoinHandle<Result<Option<Project>, String>>>,
}

impl Default for ProjectState {
    fn default() -> Self {
        Self {
            project: None,
            status: "Choose a project folder to get started.".into(),
            generation: 0,
            job: None,
        }
    }
}

impl ProjectState {
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }
}

pub struct ProjectPlugin;
impl Plugin for ProjectPlugin {
    fn build(&self, app: &mut App) {
        app.add_system(Update, process_commands);
    }
}

fn process_commands(
    mut commands: ResMut<EditorCommands>,
    mut state: ResMut<ProjectState>,
    mut documents: ResMut<crate::asset_editor::AssetEditorCommands>,
    asset_server: ecs::Res<AssetServer>,
) {
    if state.job.as_ref().is_some_and(|j| j.is_finished()) {
        let result = state
            .job
            .take()
            .unwrap()
            .join()
            .unwrap_or_else(|_| Err("Project worker failed".into()));
        match result {
            Ok(Some(project)) => {
                if let Err(error) =
                    asset_server.publish_project_content(&project.root, project.registry.clone())
                {
                    state.status = format!("Could not activate project assets: {error:#}");
                    return;
                }
                state.status = format!(
                    "{} imported assets · {} scenes · {}",
                    project.assets.len(),
                    project.scenes().count(),
                    project.root.display()
                );
                state.project = Some(project);
                state.generation += 1;
                documents
                    .0
                    .push_back(crate::asset_editor::AssetEditorCommand::CloseAll);
            }
            Ok(None) => state.status = "Folder selection cancelled.".into(),
            Err(error) => state.status = format!("Could not open project: {error}"),
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
                        .push_back(crate::asset_editor::AssetEditorCommand::Open {
                            asset: asset.clone(),
                            project_generation: state.generation,
                        });
                }
            }
            EditorCommand::OpenProject(path) => {
                state.status = "Opening project…".into();
                state.job = Some(std::thread::spawn(move || {
                    discover_project(&path)
                        .map(Some)
                        .map_err(|e| format!("{e:#}"))
                }));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use essential::assets::content::{
        CONTENT_FORMAT_VERSION, ContentAssetHeader, write_content_asset,
    };
    #[test]
    fn discovers_only_scenes_without_writing_registry() {
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
        assert_eq!(project.assets.len(), 2);
        assert_eq!(project.scenes().next().unwrap().id, id);
        assert_eq!(project.assets[0].folder, "content");
        assert_eq!(project.filtered_assets("mesh", None, None).len(), 1);
        assert_eq!(project.filtered_assets("", None, Some("Scene")).len(), 1);
        assert_eq!(project.filtered_assets("", Some("missing"), None).len(), 0);
        assert!(!dir.path().join("content/.registry.toml").exists());
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
}
