//! One scene preview, replaced only after the next asset has loaded successfully.
//! Temporary inspector changes live in ECS; this editor never saves them.
use std::{path::PathBuf, thread::JoinHandle};

use app::{schedule_groups::Update, App, Plugin};
use ecs::{
    command::{CommandQueue, EntityCommandQueue},
    resource::{Res, ResMut},
    Component, Entity, Query, Resource,
};
use essential::{
    assets::{content::read_content_asset, Asset, AssetId},
    transform::Transform,
};
use scene::{scene::Scene, spawner::spawn_scene};

use crate::{
    asset_editor::{
        finish_asset_request, AssetEditor, AssetEditorAppExt, EditorDocument, EditorOwned,
    },
    project::ProjectState,
    selection::Selection,
};

#[derive(Component)]
pub struct SceneRoot {
    pub asset_id: AssetId,
    pub address: String,
}

/// Shared presentation summary for the scene hierarchy and Chatter. The scene
/// editor entity owns the actual preview and pending worker.
#[derive(Resource, Default)]
pub struct SceneState {
    pub status: String,
    loading: bool,
    document: Option<Entity>,
}
impl SceneState {
    pub fn loading(&self) -> bool {
        self.loading
    }
}

#[derive(Component, Default)]
struct SceneEditorState {
    root: Option<Entity>,
    job: Option<SceneJob>,
}
struct SceneJob {
    request: u64,
    project: u64,
    worker: JoinHandle<Result<Scene, String>>,
}

/// The built-in editor for `Scene`. Register the engine's Scene asset plugin first.
pub struct SceneEditor;
impl AssetEditor for SceneEditor {
    fn build(&self, editor: &mut EntityCommandQueue) {
        editor.insert(SceneEditorState::default());
    }
}

fn load_scene(root: PathBuf, address: String, id: AssetId) -> anyhow::Result<Scene> {
    let bytes = std::fs::read(root.join(address))?;
    let (header, payload) = read_content_asset(&bytes)?;
    anyhow::ensure!(
        header.asset_id == id,
        "Asset identity changed; refresh the project catalogue"
    );
    anyhow::ensure!(
        header.kind == Scene::name(),
        "Selected asset is not a Scene"
    );
    Ok(bincode::deserialize(payload)?)
}

pub struct ScenePlugin;
impl Plugin for ScenePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(SceneState::default());
        app.register_asset_editor::<Scene>(SceneEditor)
            .expect("ScenePlugin requires registered Scene assets and the asset-editor registry");
        app.add_system(Update, update_scenes);
    }
}

fn update_scenes(
    editors: Query<(Entity, &mut EditorDocument, &mut SceneEditorState)>,
    project: Res<ProjectState>,
    mut summary: ResMut<SceneState>,
    mut viewport: ResMut<crate::viewport::ViewportCommands>,
    mut selection: ResMut<Selection>,
    mut commands: CommandQueue,
) {
    let mut present = false;
    for (editor, mut doc, mut state) in editors.iter() {
        present = true;
        if summary.document.is_some_and(|previous| previous != editor) {
            selection.clear();
            viewport.reset = true;
        }
        if summary.document != Some(editor) {
            summary.document = Some(editor);
        }
        if state.job.as_ref().is_some_and(|job| {
            job.project != project.generation
                || !crate::asset_editor::asset_request_is_current(
                    &doc,
                    job.request,
                    project.generation,
                )
        }) {
            state.job = None;
        }
        if state.job.is_none()
            && doc.pending.is_some()
            && doc.project_generation == project.generation
        {
            let request = doc.request_generation;
            if let Some(root) = project.project.as_ref().map(|p| p.root.clone()) {
                let asset = doc.pending.as_ref().unwrap();
                let id = asset.id;
                let address = asset.address.clone();
                state.job = Some(SceneJob {
                    request,
                    project: project.generation,
                    worker: std::thread::spawn(move || {
                        load_scene(root, address, id).map_err(|e| format!("{e:#}"))
                    }),
                });
            } else {
                finish_asset_request(
                    &mut doc,
                    request,
                    project.generation,
                    Err(anyhow::anyhow!("Open a project before opening a scene")),
                );
            }
        }
        if state
            .job
            .as_ref()
            .is_some_and(|job| job.worker.is_finished())
        {
            let job = state.job.take().unwrap();
            let result = job
                .worker
                .join()
                .unwrap_or_else(|_| Err("Scene worker failed".into()));
            // Validate again before queuing any changes to the old preview.
            if crate::asset_editor::asset_request_is_current(&doc, job.request, project.generation)
            {
                match result {
                    Ok(scene) => {
                        let asset = doc.pending.as_ref().unwrap();
                        if let Some(root) = state.root {
                            commands.despawn(root);
                        }
                        let root = commands
                            .spawn((
                                Transform::IDENTITY,
                                SceneRoot {
                                    asset_id: asset.id,
                                    address: asset.address.clone(),
                                },
                                EditorOwned(editor),
                            ))
                            .entity();
                        spawn_scene(&mut commands, &scene, root);
                        state.root = Some(root);
                        selection.select_entity(root);
                        viewport.reset = true;
                        finish_asset_request(&mut doc, job.request, project.generation, Ok(()));
                    }
                    Err(error) => {
                        finish_asset_request(
                            &mut doc,
                            job.request,
                            project.generation,
                            Err(anyhow::anyhow!("Could not open scene: {error}")),
                        );
                    }
                }
            }
        }
        if summary.status != doc.status {
            summary.status = doc.status.clone();
        }
        if summary.loading != doc.pending.is_some() {
            summary.loading = doc.pending.is_some();
        }
    }
    if !present && summary.document.is_some() {
        summary.document = None;
        selection.clear();
        viewport.reset = true;
        summary.status.clear();
        summary.loading = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ecs::{IntoSystem, System, World};
    fn process_editor_commands(world: &mut World) {
        let mut system = crate::asset_editor::process_editor_commands.into_system();
        system.initialize(world);
        system.run_and_apply(world);
    }
    fn update_scenes(world: &mut World) {
        let mut system = super::update_scenes.into_system();
        system.initialize(world);
        system.run_and_apply(world);
        let mut reset = (|mut commands: ResMut<crate::viewport::ViewportCommands>,
                          mut fly: ResMut<crate::viewport::FlyCamera>| {
            crate::viewport::apply_workspace_navigation(&mut commands, &mut fly);
        })
        .into_system();
        reset.initialize(world);
        reset.run_and_apply(world);
    }
    use crate::asset_editor::{
        ActiveEditor, AssetEditorCommand, AssetEditorCommands, AssetEditorRegistry,
    };
    use crate::project::AssetEntry;
    use essential::assets::content::{
        write_content_asset, AssetRegistry, ContentAssetHeader, ImportProvenance,
        CONTENT_FORMAT_VERSION,
    };
    use std::time::{Duration, Instant};

    fn entry(name: &str) -> AssetEntry {
        AssetEntry {
            id: AssetId::new(),
            address: format!("{name}.gasset"),
            kind: Scene::name().into(),
            display_name: name.into(),
            folder: String::new(),
            provenance: ImportProvenance {
                source: "fixture".into(),
                sub_asset: name.into(),
            },
        }
    }
    fn fixture(root: &std::path::Path, asset: &AssetEntry) {
        let mut node = scene::scene::SceneNode {
            name: asset.display_name.clone(),
            children: vec![],
            components: vec![],
        };
        node.push_component(&Transform::IDENTITY).unwrap();
        let scene = Scene {
            nodes: vec![node],
            referenced_assets: vec![],
        };
        let header = ContentAssetHeader {
            format_version: CONTENT_FORMAT_VERSION,
            asset_id: asset.id,
            references: vec![],
            kind: Scene::name().into(),
            provenance: Some(asset.provenance.clone()),
        };
        std::fs::write(
            root.join(&asset.address),
            write_content_asset(&header, &bincode::serialize(&scene).unwrap()).unwrap(),
        )
        .unwrap();
    }
    fn world(root: &std::path::Path) -> World {
        let mut world = World::new();
        world.register_component::<Transform>();
        world.register_component_type::<Transform>();
        world.insert_resource(SceneState::default());
        world.insert_resource(crate::viewport::ViewportCommands::default());
        world.insert_resource(crate::viewport::FlyCamera::default());
        world.insert_resource(Selection::default());
        world.insert_resource(ActiveEditor::default());
        world.insert_resource(AssetEditorCommands::default());
        let mut registry = AssetEditorRegistry::default();
        registry.register::<Scene>(SceneEditor).unwrap();
        world.insert_resource(registry);
        let mut project = ProjectState::default();
        project.generation = 1;
        project.project = Some(crate::project::Project {
            root: root.into(),
            assets: vec![],
            registry: AssetRegistry::default(),
        });
        world.insert_resource(project);
        world
    }
    fn open(world: &mut World, asset: &AssetEntry) -> Entity {
        world
            .get_resource_mut::<AssetEditorCommands>()
            .unwrap()
            .0
            .push_back(AssetEditorCommand::Open {
                asset: asset.clone(),
                project_generation: 1,
            });
        process_editor_commands(world);
        world.get_resource::<ActiveEditor>().unwrap().0.unwrap()
    }
    fn settle(world: &mut World, editor: Entity) {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            update_scenes(world);
            if world
                .get_component_for_entity::<SceneEditorState>(editor)
                .is_none_or(|state| state.job.is_none())
            {
                break;
            }
            assert!(Instant::now() < deadline, "scene loading timed out");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn root(world: &World, editor: Entity) -> Entity {
        world
            .get_component_for_entity::<SceneEditorState>(editor)
            .unwrap()
            .root
            .unwrap()
    }

    #[test]
    fn successful_replacement_reuses_tab_discards_edits_and_keeps_helpers() {
        let dir = tempfile::tempdir().unwrap();
        let a = entry("first");
        let b = entry("second");
        fixture(dir.path(), &a);
        fixture(dir.path(), &b);
        let mut world = world(dir.path());
        let helper = world.spawn(crate::viewport::EditorHelper);
        let editor = open(&mut world, &a);
        settle(&mut world, editor);
        let old = root(&world, editor);
        world
            .get_component_for_entity_mut::<Transform>(old)
            .unwrap()
            .translation
            .x = 42.0;
        assert_eq!(open(&mut world, &a), editor);
        assert_eq!(root(&world, editor), old);
        assert_eq!(open(&mut world, &b), editor);
        assert_eq!(
            world
                .get_component_for_entity::<Transform>(old)
                .unwrap()
                .translation
                .x,
            42.0
        );
        settle(&mut world, editor);
        assert!(!world.entity_is_valid(old));
        assert!(world.entity_is_valid(helper));
        assert_eq!(
            world
                .get_component_for_entity::<SceneRoot>(root(&world, editor))
                .unwrap()
                .asset_id,
            b.id
        );
        assert_eq!(world.query::<&SceneRoot, ()>().iter(&mut world).count(), 1);
        assert_eq!(
            world
                .query::<&EditorDocument, ()>()
                .iter(&mut world)
                .count(),
            1
        );
        assert_eq!(
            world.get_resource::<Selection>().unwrap().entity(),
            Some(root(&world, editor))
        );
    }

    #[test]
    fn failed_replacement_retains_live_edits_selection_and_camera() {
        let dir = tempfile::tempdir().unwrap();
        let a = entry("first");
        let bad = entry("missing");
        fixture(dir.path(), &a);
        let mut world = world(dir.path());
        let editor = open(&mut world, &a);
        settle(&mut world, editor);
        let old = root(&world, editor);
        world
            .get_component_for_entity_mut::<Transform>(old)
            .unwrap()
            .translation
            .y = 17.0;
        world
            .get_resource_mut::<crate::viewport::FlyCamera>()
            .unwrap()
            .position
            .x = 23.0;
        open(&mut world, &bad);
        settle(&mut world, editor);
        assert_eq!(root(&world, editor), old);
        assert_eq!(
            world
                .get_component_for_entity::<Transform>(old)
                .unwrap()
                .translation
                .y,
            17.0
        );
        assert_eq!(
            world
                .get_resource::<crate::viewport::FlyCamera>()
                .unwrap()
                .position
                .x,
            23.0
        );
        assert_eq!(
            world.get_resource::<Selection>().unwrap().entity(),
            Some(old)
        );
        let doc = world
            .get_component_for_entity::<EditorDocument>(editor)
            .unwrap();
        assert_eq!(doc.current.as_ref().unwrap().id, a.id);
        assert!(doc.pending.is_none());
        assert!(doc.status.contains("Could not open scene"));
    }

    #[test]
    fn canceled_and_superseded_workers_cannot_replace_the_preview() {
        let dir = tempfile::tempdir().unwrap();
        let a = entry("a");
        let b = entry("b");
        let c = entry("c");
        for asset in [&a, &b, &c] {
            fixture(dir.path(), asset);
        }
        let mut world = world(dir.path());
        let editor = open(&mut world, &a);
        settle(&mut world, editor);
        let old = root(&world, editor);
        open(&mut world, &b);
        open(&mut world, &a);
        settle(&mut world, editor);
        assert_eq!(root(&world, editor), old);
        open(&mut world, &b);
        open(&mut world, &c);
        settle(&mut world, editor);
        assert_eq!(
            world
                .get_component_for_entity::<SceneRoot>(root(&world, editor))
                .unwrap()
                .asset_id,
            c.id
        );
        open(&mut world, &b);
        world
            .get_resource_mut::<AssetEditorCommands>()
            .unwrap()
            .0
            .push_back(AssetEditorCommand::CloseAll);
        process_editor_commands(&mut world);
        update_scenes(&mut world);
        assert_eq!(world.query::<&SceneRoot, ()>().iter(&mut world).count(), 0);
        assert!(world.get_resource::<ActiveEditor>().unwrap().0.is_none());
        assert!(world
            .get_resource::<Selection>()
            .unwrap()
            .entity()
            .is_none());
    }

    #[test]
    fn finished_worker_from_old_project_cannot_replace_scene() {
        let dir = tempfile::tempdir().unwrap();
        let a = entry("a");
        let b = entry("b");
        fixture(dir.path(), &a);
        fixture(dir.path(), &b);
        let mut world = world(dir.path());
        let editor = open(&mut world, &a);
        settle(&mut world, editor);
        let old = root(&world, editor);
        open(&mut world, &b);
        let request = world
            .get_component_for_entity::<EditorDocument>(editor)
            .unwrap()
            .request_generation;
        let scene = load_scene(dir.path().into(), b.address, b.id).unwrap();
        let worker = std::thread::spawn(move || Ok(scene));
        while !worker.is_finished() {
            std::thread::yield_now();
        }
        world
            .get_component_for_entity_mut::<SceneEditorState>(editor)
            .unwrap()
            .job = Some(SceneJob {
            request,
            project: 1,
            worker,
        });
        world.get_resource_mut::<ProjectState>().unwrap().generation = 2;
        update_scenes(&mut world);
        assert_eq!(root(&world, editor), old);
        assert!(world
            .get_component_for_entity::<SceneEditorState>(editor)
            .unwrap()
            .job
            .is_none());
        assert_eq!(
            world
                .get_component_for_entity::<EditorDocument>(editor)
                .unwrap()
                .current
                .as_ref()
                .unwrap()
                .id,
            a.id
        );
    }

    #[test]
    fn close_and_reopen_in_one_update_clears_old_scene_selection() {
        let dir = tempfile::tempdir().unwrap();
        let a = entry("a");
        fixture(dir.path(), &a);
        let mut world = world(dir.path());
        let editor = open(&mut world, &a);
        settle(&mut world, editor);
        let old = root(&world, editor);
        world
            .get_resource_mut::<AssetEditorCommands>()
            .unwrap()
            .0
            .extend([
                AssetEditorCommand::CloseAll,
                AssetEditorCommand::Open {
                    asset: entry("missing"),
                    project_generation: 1,
                },
            ]);
        process_editor_commands(&mut world);
        update_scenes(&mut world);
        assert!(!world.entity_is_valid(old));
        assert!(world
            .get_resource::<Selection>()
            .unwrap()
            .entity()
            .is_none());
    }

    #[test]
    fn loader_rejects_wrong_identity_kind_and_corrupt_payload() {
        let dir = tempfile::tempdir().unwrap();
        let asset = entry("a");
        fixture(dir.path(), &asset);
        assert!(load_scene(dir.path().into(), asset.address.clone(), AssetId::new()).is_err());
        let path = dir.path().join(&asset.address);
        let bytes = std::fs::read(&path).unwrap();
        let (mut header, _) = read_content_asset(&bytes).unwrap();
        header.kind = "Other".into();
        std::fs::write(&path, write_content_asset(&header, &[]).unwrap()).unwrap();
        assert!(load_scene(dir.path().into(), asset.address.clone(), asset.id).is_err());
        header.kind = Scene::name().into();
        std::fs::write(&path, write_content_asset(&header, &[1]).unwrap()).unwrap();
        assert!(load_scene(dir.path().into(), asset.address, asset.id).is_err());
    }
}
