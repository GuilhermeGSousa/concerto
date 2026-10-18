//! The scene document: loading, hosting its edit `App`, and saving.
use std::{path::PathBuf, thread::JoinHandle};

use anyhow::Context;

use concerto_app::{
    App, Plugin,
    schedule_groups::{First, Update},
};
use concerto_ecs::{
    Changed, Component, Entity, IntoSystemConfig, Query, Resource, World,
    command::{CommandQueue, EntityCommandQueue},
    events::event_reader::EventReader,
    resource::{Res, ResMut},
    signal::{On, Signal},
    system::NonSendMarker,
};
use concerto_foundation::assets::{
    Asset, AssetId, ContentAssetRoot,
    asset_server::AssetServer,
    content::{read_content_asset, save_content_asset_into},
};
use concerto_render::gpu::Gpu;
use concerto_scene::scene::Scene;
use concerto_window::{input::actions::ActionFired, plugin::Window};

use crate::{
    asset_editor::{
        ActiveEditor, AssetEditor, AssetEditorAppExt, EditorDocument, SaveRequested, Unsaved,
        asset_request_is_current, finish_asset_request,
    },
    hierarchy::HierarchyState,
    hosted::{
        self, HostRequest, HostRole, Hosted, HostedApp, HostedEventsAppExt, ParkedEditApp,
        PendingApp, PendingPlayApp, PlayFailure, PlayState, StopPlay, input::ViewportShown,
        toggle_play,
    },
    inspector::{ComponentEdits, PropertyCommits},
    project::{AssetEntry, ProjectState},
    tools::{
        EditToolsCorePlugin, EditToolsPlugin,
        scene::{OpenScene, PublishContent, RevealEntity, SceneEdited, SceneStatus},
    },
};

/// Shared presentation summary for the scene hierarchy and Chatter.
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

/// What every edit `App` is built from.
#[derive(Resource, Clone)]
pub struct SceneRecipe {
    pub recipe: fn(&mut App),
    pub gpu: Option<Gpu>,
}

#[derive(Component, Default)]
pub(crate) struct SceneEditorState {
    job: Option<SceneJob>,
    building: Option<AssetEntry>,
}

#[cfg(test)]
impl SceneEditorState {
    pub(crate) fn idle(&self) -> bool {
        self.job.is_none() && self.building.is_none()
    }
}

struct SceneJob {
    asset: AssetEntry,
    root: PathBuf,
    worker: JoinHandle<Result<Scene, String>>,
}

/// The built-in editor for `Scene`.
pub struct SceneEditor;
impl AssetEditor for SceneEditor {
    fn build(&self, editor: &mut EntityCommandQueue) {
        editor.insert((SceneEditorState::default(), PlayState::Editing));
    }

    fn closable(&self) -> bool {
        false
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

/// Opens, hosts, and saves scenes; `recipe` is what each scene's edit `App` is built from.
pub struct ScenePlugin {
    pub recipe: fn(&mut App),
}

impl Plugin for ScenePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(SceneState::default());
        let gpu = app.get_resource::<Gpu>().cloned();
        app.insert_resource(SceneRecipe {
            recipe: self.recipe,
            gpu,
        });
        app.register_asset_editor::<Scene>(SceneEditor)
            .expect("ScenePlugin requires the asset-editor registry");
        install_listeners(app.main_mut().world_mut());
        app.forward_hosted_event::<SceneEdited>()
            .forward_hosted_event::<SceneStatus>()
            .forward_hosted_event::<RevealEntity>();
        app.add_system(
            First,
            refuse_edited_replacements.before(hosted::finish_pending),
        )
        .add_system(Update, (follow_unsaved_edits, show_status, reveal))
        .add_system(Update, update_scenes)
        .add_system(Update, request_scene_saves)
        .add_system(Update, scene_actions)
        .add_system(Update, show_play_failures);
    }
}

pub(crate) fn install_listeners(world: &mut World) {
    world.add_listener(save_scene);
    world.add_listener(publish_to_hosted);
    world.add_listener(play);
}

/// Keeps a Scene document's `Unsaved` in step with its edit `App`'s `SceneDirty`.
pub(crate) fn follow_unsaved_edits(
    documents: Query<(Entity, &HostedApp, Option<&PlayState>, Option<&Unsaved>)>,
    mut cmd: CommandQueue,
) {
    for (document, hosted, state, unsaved) in documents.iter() {
        if state.is_some_and(|state| *state != PlayState::Editing) {
            continue;
        }
        match (hosted.scene_dirty(), unsaved.is_some()) {
            (true, false) => cmd.insert(Unsaved, document),
            (false, true) => cmd.remove::<Unsaved>(document),
            _ => {}
        }
    }
}

fn has_unsaved_edits(world: &World, document: Entity) -> bool {
    world
        .get_component_for_entity::<Unsaved>(document)
        .is_some()
        || world
            .get_component_for_entity::<HostedApp>(document)
            .is_some_and(HostedApp::scene_dirty)
}

pub(crate) fn show_status(
    mut statuses: EventReader<Hosted<SceneStatus>>,
    documents: Query<&mut EditorDocument>,
) {
    for Hosted { document, event } in statuses.read() {
        if let Some(mut document) = documents.get_entity(*document) {
            document.status = event.0.clone();
        }
    }
}

pub(crate) fn reveal(
    mut reveals: EventReader<Hosted<RevealEntity>>,
    active: Res<ActiveEditor>,
    mut hierarchy: ResMut<HierarchyState>,
) {
    for Hosted { document, event } in reveals.read() {
        if active.0 == Some(*document) {
            hierarchy.reveal_entity(event.0);
        }
    }
}

struct SaveScene {
    document: Entity,
}
impl Signal for SaveScene {}

pub(crate) fn request_scene_saves(
    requested: Query<(Entity, &SaveRequested, &SceneEditorState)>,
    mut cmd: CommandQueue,
) {
    for (document, ..) in requested.iter() {
        cmd.remove::<SaveRequested>(document);
        cmd.trigger(SaveScene { document });
    }
}

fn save_scene(on: On<SaveScene>, world: &mut World) {
    let document = on.signal().document;
    if game_is_on(world, document) {
        set_status(world, document, STOP_TO_SAVE);
        return;
    }
    let Some(hosted) = world.get_component_for_entity_mut::<HostedApp>(document) else {
        set_status(world, document, "Save failed: No scene is open");
        return;
    };
    let written = hosted
        .capture_scene()
        .map_err(anyhow::Error::msg)
        .and_then(|scene| write_scene(world, document, &scene));
    let status = match written {
        Ok(address) => {
            if let Some(hosted) = world.get_component_for_entity_mut::<HostedApp>(document) {
                hosted.mark_saved();
            }
            world.remove_component::<Unsaved>(document);
            format!("Saved {address}")
        }
        Err(error) => format!("Save failed: {error:#}"),
    };
    if let Some(document) = world.get_component_for_entity_mut::<EditorDocument>(document) {
        document.status = status;
    }
}

fn write_scene(world: &mut World, document: Entity, scene: &Scene) -> anyhow::Result<String> {
    let source = world
        .get_component_for_entity::<EditorDocument>(document)
        .context("The document was closed")?
        .current
        .clone()
        .context("No scene is open")?;
    let project = world
        .get_resource_mut::<ProjectState>()
        .and_then(|state| state.project.as_mut())
        .context("Open a project before saving")?;
    let address = source.address;
    let id = save_content_asset_into(scene, &project.root, &address, &mut project.registry)?;
    project.registry.save(&project.root)?;
    let saved = AssetEntry::from_address(id, &address, Scene::name().into(), None);
    project.insert(saved.clone());
    let project_root = project.root.clone();
    let registry = project.registry.clone();
    if let Some(server) = world.get_resource::<AssetServer>() {
        server.publish_project_content(&project_root, registry)?;
    }
    if let Some(document) = world.get_component_for_entity_mut::<EditorDocument>(document) {
        document.title = saved.display_name.clone();
        document.current = Some(saved);
    }
    world.trigger(PublishToHosted);
    Ok(address)
}

pub(crate) struct PublishToHosted;
impl Signal for PublishToHosted {}

fn publish_to_hosted(_: On<PublishToHosted>, world: &mut World) {
    let Some((root, registry)) = world
        .get_resource::<ProjectState>()
        .and_then(|state| state.project.as_ref())
        .map(|project| (project.root.clone(), project.registry.clone()))
    else {
        return;
    };
    let documents: Vec<Entity> = world
        .query::<(Entity, &HostedApp), ()>()
        .iter(world)
        .map(|(document, _)| document)
        .collect();
    for document in documents {
        hosted::trigger_on(
            world,
            document,
            PublishContent {
                root: root.clone(),
                registry: registry.clone(),
            },
        );
    }
    let parked: Vec<Entity> = world
        .query::<(Entity, &ParkedEditApp), ()>()
        .iter(world)
        .map(|(document, _)| document)
        .collect();
    for document in parked {
        if let Some(parked) = world.get_component_for_entity_mut::<ParkedEditApp>(document) {
            parked.0.trigger(PublishContent {
                root: root.clone(),
                registry: registry.clone(),
            });
        }
    }
}

/// Starts the game in a Scene document's place, saving the document first if it is unsaved.
pub struct Play(pub Entity);
impl Signal for Play {}

const STOP_TO_SAVE: &str = "Stop to save";

fn game_is_on(world: &World, document: Entity) -> bool {
    world
        .get_component_for_entity::<PlayState>(document)
        .is_some_and(|state| state.is_on())
}

fn set_status(world: &mut World, document: Entity, status: impl Into<String>) {
    if let Some(document) = world.get_component_for_entity_mut::<EditorDocument>(document) {
        document.status = status.into();
    }
}

fn play(on: On<Play>, world: &mut World) {
    let document = on.signal().0;
    if game_is_on(world, document)
        || world
            .get_resource::<crate::guard::UnsavedGuard>()
            .is_some_and(crate::guard::UnsavedGuard::asking)
        || world
            .get_resource::<ProjectState>()
            .is_some_and(ProjectState::busy)
    {
        return;
    }
    let Some(loading) = world
        .get_component_for_entity::<EditorDocument>(document)
        .map(|doc| doc.pending.is_some())
    else {
        return;
    };
    if loading
        || world
            .get_component_for_entity::<PendingApp>(document)
            .is_some()
    {
        set_status(world, document, "Wait for the scene to load");
        return;
    }
    if world
        .get_component_for_entity::<HostedApp>(document)
        .is_none()
    {
        set_status(world, document, "No scene is open");
        return;
    }
    if has_unsaved_edits(world, document) {
        world.trigger(SaveScene { document });
        if has_unsaved_edits(world, document) {
            return;
        }
    }
    match begin_play(world, document) {
        Ok(pending) => {
            world.insert(
                (PendingPlayApp::new(pending), PlayState::Starting),
                document,
            );
            if world
                .get_component_for_entity::<hosted::EditTarget>(document)
                .is_some()
            {
                world.trigger(hosted::SetEditTarget(None));
            }
        }
        Err(error) => set_status(
            world,
            document,
            format!("Could not start the game: {error:#}"),
        ),
    }
}

fn begin_play(world: &World, document: Entity) -> anyhow::Result<PendingApp> {
    let scene = world
        .get_component_for_entity::<EditorDocument>(document)
        .and_then(|doc| doc.current.as_ref())
        .context("No scene is open")?
        .id;
    let project = world
        .get_resource::<ProjectState>()
        .and_then(|state| state.project.as_ref())
        .context("Open a project before playing")?;
    let root = project.root.canonicalize().with_context(|| {
        format!(
            "failed to resolve project root '{}'",
            project.root.display()
        )
    })?;
    let registry = project.registry.clone();
    let recipe = world
        .get_resource::<SceneRecipe>()
        .context("No recipe to build the game from")?;
    let request = HostRequest {
        gpu: recipe.gpu.as_ref(),
        recipe: recipe.recipe,
        size: world
            .get_resource::<ViewportShown>()
            .and_then(|shown| shown.0)
            .map_or((0, 0), |view| view.size),
        scale_factor: world
            .get_resource::<Window>()
            .map_or(1.0, Window::scale_factor),
        role: HostRole::Game,
    };
    Ok(hosted::begin(
        request,
        move |app| {
            if let Some(server) = app.get_resource::<AssetServer>() {
                server.publish_content_source(ContentAssetRoot::Directory(root), registry);
            }
            app.register_plugin(EditToolsCorePlugin);
            app.set_startup_scene(scene);
        },
        |_| {},
    ))
}

pub(crate) fn scene_actions(
    mut fired: EventReader<ActionFired>,
    active: Res<ActiveEditor>,
    documents: Query<(&mut EditorDocument, &PlayState)>,
    mut cmd: CommandQueue,
) {
    let (toggle, save) = fired.read().fold((false, false), |(toggle, save), action| {
        (
            toggle | action.is(crate::actions::TogglePlay),
            save | action.is(crate::actions::Save),
        )
    });
    if !toggle && !save {
        return;
    }
    let Some(document) = active.0 else {
        return;
    };
    let Some((mut doc, state)) = documents.get_entity(document) else {
        return;
    };
    if save && state.is_on() {
        doc.status = STOP_TO_SAVE.into();
    }
    if toggle {
        toggle_play(document, Some(*state), &mut cmd);
    }
}

pub(crate) fn show_play_failures(
    failures: Query<(&PlayFailure, &mut EditorDocument), Changed<PlayFailure>>,
) {
    for (failure, mut document) in failures.iter() {
        document.status = format!("Could not start the game: {}", failure.0);
    }
}

pub(crate) fn refuse_edited_replacements(
    building: Query<(
        Entity,
        &mut EditorDocument,
        &mut SceneEditorState,
        &mut PendingApp,
    )>,
    unsaved: Query<&Unsaved>,
    active: Res<ActiveEditor>,
    commits: Res<PropertyCommits>,
    edits: Res<ComponentEdits>,
    mut commands: CommandQueue,
) {
    let queued = !commits.0.is_empty() || !edits.0.is_empty();
    for (editor, mut doc, mut state, mut pending) in building.iter() {
        let Some(request) = state.building.clone() else {
            continue;
        };
        let edited = unsaved.get_entity(editor).is_some() || (queued && active.0 == Some(editor));
        if !edited || !asset_request_is_current(&doc, &request) {
            continue;
        }
        pending.cancel();
        state.building = None;
        commands.remove::<PendingApp>(editor);
        finish_asset_request(
            &mut doc,
            &request,
            Err(anyhow::anyhow!(
                "Could not open scene: unsaved changes were made while it loaded"
            )),
        );
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn update_scenes(
    _: NonSendMarker,
    editors: Query<(Entity, &mut EditorDocument, &mut SceneEditorState)>,
    unsaved: Query<&Unsaved>,
    pending: Query<&PendingApp>,
    hosted_apps: Query<&HostedApp>,
    states: Query<&PlayState>,
    project: Res<ProjectState>,
    recipe: Res<SceneRecipe>,
    shown: Res<ViewportShown>,
    window: Res<Window>,
    mut summary: ResMut<SceneState>,
    mut commands: CommandQueue,
) {
    let mut present = false;
    for (editor, mut doc, mut state) in editors.iter() {
        present = true;
        if summary.document != Some(editor) {
            summary.document = Some(editor);
        }

        if doc.pending.is_some() && states.get_entity(editor).is_some_and(|state| state.is_on()) {
            commands.trigger(StopPlay(editor));
        }

        let root = project.project.as_ref().map(|project| &project.root);
        if state.job.as_ref().is_some_and(|job| {
            root != Some(&job.root) || !asset_request_is_current(&doc, &job.asset)
        }) {
            state.job = None;
        }

        if let Some(request) = state.building.clone() {
            let current = asset_request_is_current(&doc, &request);
            let building = pending.get_entity(editor);
            let still_building = building.is_some();
            let failure = building.and_then(|pending| pending.failure().map(str::to_owned));
            if !current {
                state.building = None;
                commands.remove::<PendingApp>(editor);
            } else if let Some(failure) = failure {
                state.building = None;
                commands.remove::<PendingApp>(editor);
                finish_asset_request(
                    &mut doc,
                    &request,
                    Err(anyhow::anyhow!("Could not open scene: {failure}")),
                );
            } else if !still_building && hosted_apps.get_entity(editor).is_some() {
                state.building = None;
                finish_asset_request(&mut doc, &request, Ok(()));
            }
        }

        if state.job.is_none()
            && state.building.is_none()
            && let Some(asset) = doc.pending.clone()
        {
            if let Some(root) = project.project.as_ref().map(|p| p.root.clone()) {
                let id = asset.id;
                let address = asset.address.clone();
                let read_root = root.clone();
                state.job = Some(SceneJob {
                    asset,
                    root,
                    worker: std::thread::spawn(move || {
                        load_scene(read_root, address, id).map_err(|e| format!("{e:#}"))
                    }),
                });
            } else {
                finish_asset_request(
                    &mut doc,
                    &asset,
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
            let loaded = job
                .worker
                .join()
                .unwrap_or_else(|_| Err("Scene worker failed".into()));
            if asset_request_is_current(&doc, &job.asset) {
                let loaded = if unsaved.get_entity(editor).is_some() {
                    Err("unsaved changes were made while it loaded".into())
                } else {
                    loaded
                };
                match loaded {
                    Ok(scene) => {
                        let content = project
                            .project
                            .as_ref()
                            .map(|p| (p.root.clone(), p.registry.clone()));
                        let request = HostRequest {
                            gpu: recipe.gpu.as_ref(),
                            recipe: recipe.recipe,
                            size: shown.0.map_or((0, 0), |view| view.size),
                            scale_factor: window.scale_factor(),
                            role: HostRole::Edit,
                        };
                        commands.insert(
                            hosted::begin(
                                request,
                                |app| {
                                    app.register_plugin(EditToolsPlugin);
                                },
                                move |hosted| {
                                    if let Some((root, registry)) = content {
                                        hosted.trigger(PublishContent { root, registry });
                                    }
                                    hosted.trigger(OpenScene(scene));
                                },
                            ),
                            editor,
                        );
                        state.building = Some(job.asset);
                    }
                    Err(error) => {
                        finish_asset_request(
                            &mut doc,
                            &job.asset,
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
        summary.status.clear();
        summary.loading = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hosted::testing::{ENTERED, GATED_DROPS, GatedAsset, RELEASED, lock_gate, wait_for};
    use crate::{
        asset_editor::{AssetEditorCommand, AssetEditorCommands},
        hosted::{
            EditWorldSignals, PlayState,
            testing::{SceneFixture, scene_entry, write_scene_file},
        },
        inspector::InspectorData,
        tools::{entity_ops::EntityEdit, selection::Select},
    };
    use concerto_app::{main_schedule::MainSchedulePlugin, plugins::TimePlugin};
    use concerto_ecs::{
        IntoSystem, System, events::event_channel::EventChannel, world_id::WorldId,
    };
    use concerto_foundation::{
        assets::content::{AssetRegistry, write_content_asset},
        transform::Transform,
    };

    #[derive(Resource)]
    struct RenameTarget(Entity);

    fn scene_id(f: &SceneFixture, document: Entity) -> WorldId {
        f.world
            .get_component_for_entity::<HostedApp>(document)
            .unwrap()
            .world_for_test()
            .id()
    }

    fn save(f: &mut SceneFixture, document: Entity) {
        f.world.insert(SaveRequested, document);
        f.frame();
    }

    #[test]
    fn opening_a_scene_builds_an_edit_app_holding_its_nodes() {
        let mut f = SceneFixture::new();
        let document = f.open();

        let names = f.scene_names(document);
        assert!(names.contains(&"first".to_string()) && names.contains(&"second".to_string()));
        assert!(!f.unsaved(document));
    }

    #[test]
    fn an_edit_marks_the_document_unsaved_once_the_editor_follows_its_scene() {
        let mut f = SceneFixture::new();
        let document = f.open();
        let first = f.scene_entity(document, "first");
        fn rename(mut scene: EditWorldSignals, target: Res<RenameTarget>) {
            scene.trigger(EntityEdit::Rename {
                entity: target.0,
                name: "renamed".into(),
            });
        }
        f.world.insert_resource(RenameTarget(first));
        let mut system = rename.into_system();
        system.initialize(&mut f.world);

        system.run_and_apply((), &mut f.world);
        assert!(
            f.world
                .get_component_for_entity::<Unsaved>(document)
                .is_none()
        );

        f.deliver();

        assert!(
            f.world
                .get_component_for_entity::<Unsaved>(document)
                .is_some()
        );
    }

    #[test]
    fn a_save_writes_the_file_and_clears_unsaved() {
        let mut f = SceneFixture::new();
        let document = f.open();
        f.rename(document, "first", "renamed");
        f.world.insert(SaveRequested, document);

        f.frame();

        assert!(!f.unsaved(document));
        assert!(f.status(document).starts_with("Saved"));
        assert!(f.saved_scene_names().contains(&"renamed".to_string()));
    }

    #[test]
    fn an_edit_and_a_save_in_the_same_frame_end_clean() {
        let mut f = SceneFixture::new();
        let document = f.open();
        f.rename(document, "first", "renamed");
        f.world.insert(SaveRequested, document);
        f.rename(document, "second", "also renamed");

        f.frame();

        assert!(!f.unsaved(document));
        let names = f.saved_scene_names();
        assert!(names.contains(&"renamed".to_string()));
        assert!(names.contains(&"also renamed".to_string()));
    }

    #[test]
    fn a_failed_save_stays_unsaved() {
        let mut f = SceneFixture::new();
        let document = f.open();
        f.rename(document, "first", "renamed");
        f.make_project_unwritable();
        f.world.insert(SaveRequested, document);

        f.frame();

        assert!(f.unsaved(document));
        assert!(f.status(document).starts_with("Save failed"));
        assert_eq!(
            f.document(document).current.as_ref().unwrap().id,
            f.scene.id
        );
        assert!(
            f.world
                .get_component_for_entity::<SaveRequested>(document)
                .is_none()
        );
    }

    #[test]
    fn an_edit_while_a_replacement_loads_refuses_it() {
        let mut f = SceneFixture::new();
        let document = f.open();
        let before = scene_id(&f, document);
        f.request_other_scene(document);
        f.rename(document, "first", "renamed");

        f.frames_until_idle();

        assert_eq!(scene_id(&f, document), before);
        assert!(f.unsaved(document));
        assert!(f.status(document).contains("unsaved changes"));
        assert_eq!(
            f.document(document).current.as_ref().unwrap().id,
            f.scene.id
        );
    }

    #[test]
    fn a_scene_that_finishes_loading_on_an_unsaved_document_is_never_built() {
        let mut f = SceneFixture::new();
        let document = f.open();
        f.request_other_scene(document);
        f.rename(document, "first", "renamed");

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while f.document(document).pending.is_some() {
            f.frame();
            assert!(
                f.world
                    .get_component_for_entity::<PendingApp>(document)
                    .is_none()
            );
            assert!(std::time::Instant::now() < deadline, "the load never ended");
        }

        assert!(f.status(document).contains("unsaved changes"));
    }

    #[test]
    fn an_edit_made_after_the_last_check_before_the_build_is_ready_refuses_it() {
        let mut f = SceneFixture::new();
        let document = f.open();
        let before = scene_id(&f, document);
        f.request_other_scene(document);
        f.frames_until_building(document);
        f.frame();
        f.wait_until_the_build_is_ready(document);
        f.rename(document, "first", "renamed");

        f.frame();

        assert_eq!(scene_id(&f, document), before);
        assert!(f.status(document).contains("unsaved changes"));
        assert!(
            f.world
                .get_component_for_entity::<PendingApp>(document)
                .is_none()
        );
        assert!(f.scene_names(document).contains(&"renamed".to_string()));
        assert!(f.unsaved(document));
        f.frames_until_idle();
        assert_eq!(scene_id(&f, document), before);
    }

    fn editor_with_an_edited_document_whose_replacement_is_ready() -> (App, Entity, WorldId) {
        use crate::{
            asset_editor::AssetEditorRegistry,
            hosted::testing::{hosted_now, plain_recipe, request},
        };
        let mut app = App::new();
        let project = ProjectState::default();
        app.register_plugin(MainSchedulePlugin)
            .register_plugin(TimePlugin)
            .register_plugin(concerto_window::plugin::WindowPlugin)
            .insert_resource(ActiveEditor::default())
            .insert_resource(AssetEditorRegistry::default())
            .insert_resource(HierarchyState::default())
            .insert_resource(PropertyCommits::default())
            .insert_resource(ComponentEdits::default())
            .insert_resource(project)
            .register_plugin(hosted::HostedPlugin)
            .register_plugin(ScenePlugin {
                recipe: plain_recipe,
            });
        let old = hosted_now(plain_recipe, |_| {});
        let before = old.world_for_test().id();
        let next = scene_entry("next");
        let world = app.main_mut().world_mut();
        let document = world.spawn((
            EditorDocument {
                asset_type: Scene::name(),
                title: "first".into(),
                current: Some(scene_entry("first")),
                pending: Some(next.clone()),
                order: 0,
                status: String::new(),
            },
            SceneEditorState {
                job: None,
                building: Some(next),
            },
            old,
            hosted::begin(request(plain_recipe), |_| {}, |_| {}),
            Unsaved,
        ));
        world.get_resource_mut::<ActiveEditor>().unwrap().0 = Some(document);
        app.finish_plugin_build();
        (app, document, before)
    }

    #[test]
    fn an_edited_document_refuses_a_ready_replacement_in_the_editors_own_schedule() {
        for _ in 0..20 {
            let (mut app, document, before) =
                editor_with_an_edited_document_whose_replacement_is_ready();

            app.update();

            let world = app.main().world();
            assert_eq!(
                world
                    .get_component_for_entity::<HostedApp>(document)
                    .unwrap()
                    .world_for_test()
                    .id(),
                before
            );
            let status = &world
                .get_component_for_entity::<EditorDocument>(document)
                .unwrap()
                .status;
            assert!(status.contains("unsaved changes"), "{status}");
            assert!(
                world
                    .get_component_for_entity::<PendingApp>(document)
                    .is_none()
            );
        }
    }

    #[test]
    fn an_edit_while_the_replacement_is_being_built_refuses_it() {
        let mut f = SceneFixture::new();
        let document = f.open();
        let before = scene_id(&f, document);
        f.request_other_scene(document);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while f
            .world
            .get_component_for_entity::<PendingApp>(document)
            .is_none()
        {
            f.frame();
            assert!(std::time::Instant::now() < deadline, "nothing was built");
        }
        f.world.insert(Unsaved, document);

        f.frames_until_idle();

        assert_eq!(scene_id(&f, document), before);
        assert!(
            f.world
                .get_component_for_entity::<PendingApp>(document)
                .is_none()
        );
        assert!(f.status(document).contains("unsaved changes"));
    }

    #[test]
    fn a_background_document_saves_without_being_stepped() {
        let mut f = SceneFixture::new();
        let document = f.open();
        f.rename(document, "first", "renamed");
        f.activate(None);
        f.frame();
        let frames_before = f.hosted_frames(document);
        f.world.insert(SaveRequested, document);

        f.frame();

        assert_eq!(f.hosted_frames(document), frames_before);
        assert!(!f.unsaved(document));
        assert!(f.saved_scene_names().contains(&"renamed".to_string()));
    }

    #[test]
    fn a_replacement_keeps_the_old_scene_until_the_new_one_is_ready() {
        let mut f = SceneFixture::new();
        let document = f.open();
        let before = scene_id(&f, document);
        f.request_other_scene(document);

        f.frame();
        assert_eq!(scene_id(&f, document), before);

        f.frames_until_idle();
        assert_ne!(scene_id(&f, document), before);
    }

    #[test]
    fn a_catalogue_change_reaches_every_hosted_app() {
        let mut f = SceneFixture::new();
        let document = f.open();

        f.world.trigger(PublishToHosted);

        assert_eq!(
            f.published_roots(document),
            vec![f.project_root.clone(), f.project_root.clone()]
        );
    }

    #[test]
    fn a_save_tells_every_hosted_app_of_the_new_catalogue() {
        let mut f = SceneFixture::new();
        let document = f.open();
        f.rename(document, "first", "renamed");

        save(&mut f, document);

        assert_eq!(f.published_roots(document).len(), 2);
        assert!(f.status(document).starts_with("Saved"));
    }

    #[test]
    fn what_the_scene_says_while_a_save_publishes_its_content_is_shown_after_saved() {
        let mut f = SceneFixture::new();
        let document = f.open();
        f.scene_world_mut(document)
            .insert_resource(crate::hosted::testing::ReportPublishing);
        f.rename(document, "first", "renamed");

        save(&mut f, document);

        assert_eq!(f.status(document), "published");
        assert!(!f.unsaved(document));
    }

    #[test]
    fn an_edit_not_yet_forwarded_when_a_save_captures_the_scene_ends_clean() {
        let mut f = SceneFixture::new();
        let document = f.open();
        let first = f.scene_entity(document, "first");
        f.activate(None);
        f.frame();
        let frames = f.hosted_frames(document);
        let scene = f.scene_world_mut(document);
        scene
            .get_component_for_entity_mut::<concerto_ecs::component::name::Name>(first)
            .unwrap()
            .set("renamed");
        crate::tools::scene::write(scene, SceneEdited);
        assert!(!f.unsaved(document));

        save(&mut f, document);

        assert_eq!(
            f.hosted_frames(document),
            frames,
            "the edit was not forwarded by a step"
        );
        assert!(!f.unsaved(document));
        assert!(f.status(document).starts_with("Saved"));
        assert!(f.saved_scene_names().contains(&"renamed".to_string()));
    }

    #[test]
    fn saving_a_document_that_holds_no_scene_says_so() {
        let mut f = SceneFixture::new();
        let missing = scene_entry("missing");
        f.add_to_project(&missing);
        f.request(&missing);
        f.frames_until_idle();
        let document = f.world.get_resource::<ActiveEditor>().unwrap().0.unwrap();
        assert!(
            f.world
                .get_component_for_entity::<HostedApp>(document)
                .is_none()
        );

        save(&mut f, document);

        assert_eq!(f.status(document), "Save failed: No scene is open");
        assert!(
            f.world
                .get_component_for_entity::<SaveRequested>(document)
                .is_none()
        );
    }

    #[test]
    fn a_successful_replacement_reuses_the_tab_and_shows_only_the_new_scene() {
        let mut f = SceneFixture::new();
        let document = f.open();
        let before = scene_id(&f, document);

        let same = f.scene.clone();
        f.request(&same);
        f.frames_until_idle();
        assert_eq!(scene_id(&f, document), before);

        f.request_other_scene(document);
        f.frames_until_idle();

        assert_eq!(f.scene_names(document), ["third"]);
        assert_eq!(
            f.document(document).current.as_ref().unwrap().id,
            f.other.id
        );
        assert_eq!(
            f.world
                .query::<&EditorDocument, ()>()
                .iter(&mut f.world)
                .count(),
            1
        );
        let root = f.scene_root(document);
        assert_eq!(f.selected(document), Some(root));
    }

    #[test]
    fn a_failed_replacement_keeps_the_scene_its_edits_and_its_selection() {
        let mut f = SceneFixture::new();
        let document = f.open();
        let before = scene_id(&f, document);
        let first = f.scene_entity(document, "first");
        let scene = f.scene_world_mut(document);
        scene
            .get_component_for_entity_mut::<Transform>(first)
            .unwrap()
            .translation
            .y = 17.0;
        scene.trigger(Select(Some(first)));

        let missing = scene_entry("missing");
        f.add_to_project(&missing);
        f.request(&missing);
        f.frames_until_idle();

        assert_eq!(scene_id(&f, document), before);
        assert_eq!(
            f.scene_world(document)
                .get_component_for_entity::<Transform>(first)
                .unwrap()
                .translation
                .y,
            17.0
        );
        assert_eq!(f.selected(document), Some(first));
        let doc = f.document(document);
        assert_eq!(doc.current.as_ref().unwrap().id, f.scene.id);
        assert!(doc.pending.is_none());
        assert!(
            doc.status.contains("Could not open scene"),
            "{}",
            doc.status
        );
    }

    #[test]
    fn cancelled_and_superseded_requests_cannot_replace_the_scene() {
        let mut f = SceneFixture::new();
        let b = f.add_scene("b");
        let c = f.add_scene("c");
        let document = f.open();
        let before = scene_id(&f, document);
        let a = f.scene.clone();

        f.request(&b);
        f.request(&a);
        f.frames_until_idle();
        assert_eq!(scene_id(&f, document), before);

        f.request(&b);
        f.request(&c);
        f.frames_until_idle();
        assert_eq!(f.scene_names(document), ["c"]);
        assert_eq!(f.document(document).current.as_ref().unwrap().id, c.id);

        f.request(&b);
        f.command(AssetEditorCommand::CloseAll);
        f.frames_until_idle();
        assert!(!f.world.entity_is_valid(document));
        assert!(f.world.get_resource::<ActiveEditor>().unwrap().0.is_none());
        assert_eq!(
            f.world.query::<&HostedApp, ()>().iter(&mut f.world).count(),
            0
        );
        assert_eq!(
            f.world
                .query::<&PendingApp, ()>()
                .iter(&mut f.world)
                .count(),
            0
        );
    }

    #[test]
    fn a_request_superseded_while_its_scene_is_being_built_is_dropped() {
        let mut f = SceneFixture::new();
        let document = f.open();
        let before = scene_id(&f, document);
        f.request_other_scene(document);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while f
            .world
            .get_component_for_entity::<PendingApp>(document)
            .is_none()
        {
            f.frame();
            assert!(std::time::Instant::now() < deadline, "nothing was built");
        }

        let same = f.scene.clone();
        f.request(&same);
        f.frames_until_idle();

        assert_eq!(scene_id(&f, document), before);
        assert_eq!(
            f.document(document).current.as_ref().unwrap().id,
            f.scene.id
        );
    }

    #[test]
    fn a_finished_worker_from_an_old_project_cannot_replace_the_scene() {
        let mut f = SceneFixture::new();
        let document = f.open();
        let before = scene_id(&f, document);
        f.request_other_scene(document);
        let scene =
            load_scene(f.project_root.clone(), f.other.address.clone(), f.other.id).unwrap();
        let worker = std::thread::spawn(move || Ok(scene));
        while !worker.is_finished() {
            std::thread::yield_now();
        }
        f.world
            .get_component_for_entity_mut::<SceneEditorState>(document)
            .unwrap()
            .job = Some(SceneJob {
            asset: f.other.clone(),
            root: f.project_root.clone(),
            worker,
        });
        f.world
            .get_resource_mut::<ProjectState>()
            .unwrap()
            .project
            .as_mut()
            .unwrap()
            .root = f.project_root.join("another-project");

        f.frames_until(
            "the old project's worker is dropped and the request settles",
            |f| {
                let state = f
                    .world
                    .get_component_for_entity::<SceneEditorState>(document)
                    .unwrap();
                state.job.is_none() && state.building.is_none()
            },
        );

        assert_eq!(scene_id(&f, document), before);
        assert!(
            f.world
                .get_component_for_entity::<PendingApp>(document)
                .is_none()
        );
        assert_eq!(
            f.document(document).current.as_ref().unwrap().id,
            f.scene.id
        );
    }

    #[test]
    fn closing_and_reopening_in_one_update_leaves_nothing_of_the_old_scene() {
        let mut f = SceneFixture::new();
        let document = f.open();
        f.frame();
        assert!(
            !f.world
                .get_resource::<HierarchyState>()
                .unwrap()
                .rows()
                .is_empty()
        );

        f.world
            .get_resource_mut::<AssetEditorCommands>()
            .unwrap()
            .0
            .push_back(AssetEditorCommand::CloseAll);
        let missing = scene_entry("missing");
        f.add_to_project(&missing);
        f.request(&missing);
        f.frames_until_idle();
        f.frame();

        assert!(!f.world.entity_is_valid(document));
        assert_eq!(
            f.world.query::<&HostedApp, ()>().iter(&mut f.world).count(),
            0
        );
        assert!(
            f.world
                .get_resource::<HierarchyState>()
                .unwrap()
                .rows()
                .is_empty()
        );
        assert_eq!(
            *f.world.get_resource::<InspectorData>().unwrap(),
            InspectorData::default()
        );
    }

    #[test]
    fn saving_an_imported_scene_overwrites_it_in_place() {
        let mut f = SceneFixture::new();
        let document = f.open();
        f.rename(document, "first", "renamed");
        assert!(f.unsaved(document));

        save(&mut f, document);

        assert!(!f.unsaved(document));
        let doc = f.document(document);
        let saved = doc.current.clone().unwrap();
        assert_eq!(doc.status, "Saved first.gasset");
        assert_eq!(doc.title, "first");
        assert_eq!(saved.address, f.scene.address);
        assert_eq!(saved.id, f.scene.id);
        assert_eq!(f.saved_scene_names(), ["renamed", "second"]);
        let mut files: Vec<_> = std::fs::read_dir(&f.project_root)
            .unwrap()
            .map(|file| file.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".gasset"))
            .collect();
        files.sort();
        assert_eq!(files, ["first.gasset", "other.gasset"]);
        assert!(
            f.world
                .get_component_for_entity::<SaveRequested>(document)
                .is_none()
        );
        let project = f.world.get_resource::<ProjectState>().unwrap();
        let catalogue = &project.project.as_ref().unwrap().assets;
        assert_eq!(catalogue.len(), 1);
        assert_eq!(catalogue[0].id, f.scene.id);
        assert_eq!(
            AssetRegistry::load(&f.project_root)
                .unwrap()
                .get(f.scene.id),
            Some("first.gasset")
        );

        f.rename(document, "second", "again");
        save(&mut f, document);

        assert_eq!(f.document(document).current, Some(saved));
        assert_eq!(f.saved_scene_names(), ["renamed", "again"]);
    }

    #[test]
    fn a_save_writes_back_the_asset_ids_the_scene_file_listed() {
        let mut f = SceneFixture::new();
        let referenced = AssetId::new();
        let mut scene = f.saved_scene();
        scene.referenced_assets = vec![referenced];
        let path = f.project_root.join(&f.scene.address);
        let bytes = std::fs::read(&path).unwrap();
        let (header, _) = read_content_asset(&bytes).unwrap();
        std::fs::write(
            &path,
            write_content_asset(&header, &bincode::serialize(&scene).unwrap()).unwrap(),
        )
        .unwrap();
        let document = f.open();
        f.rename(document, "first", "renamed");

        save(&mut f, document);

        assert_eq!(f.saved_scene().referenced_assets, [referenced]);
    }

    #[test]
    fn created_duplicated_and_renamed_entities_survive_a_save_and_reload() {
        let mut f = SceneFixture::new();
        let document = f.open();
        let original = f.scene_entity(document, "first");

        f.signal(EntityEdit::Create {
            parent: Some(original),
        });
        let created = f.selected(document).unwrap();
        f.signal(EntityEdit::Rename {
            entity: created,
            name: "lamp".into(),
        });
        f.signal(EntityEdit::Duplicate(original));
        assert!(f.unsaved(document));
        save(&mut f, document);

        let scene = f.saved_scene();
        let names: Vec<_> = scene.nodes.iter().map(|node| node.name.as_str()).collect();
        assert_eq!(names, ["first", "lamp", "second", "first", "lamp"]);
        assert_eq!(scene.nodes[0].children, [1]);
        assert_eq!(scene.nodes[3].children, [4]);
        assert!(scene.nodes[1].children.is_empty());
    }

    #[test]
    fn an_edit_is_reported_once_and_an_edit_after_a_save_marks_the_document_again() {
        let mut f = SceneFixture::new();
        let document = f.open();
        f.deliver();
        assert!(!f.unsaved(document), "nothing was reported");

        f.rename(document, "first", "renamed");
        assert!(f.unsaved(document));

        save(&mut f, document);
        f.deliver();
        assert!(
            !f.unsaved(document),
            "an edit that was already forwarded is not forwarded again"
        );

        f.rename(document, "second", "again");
        assert!(
            f.unsaved(document),
            "an edit after the capture is still unsaved"
        );
    }

    #[test]
    fn a_refusal_becomes_the_document_status_and_a_created_entity_is_revealed() {
        let mut f = SceneFixture::new();
        let document = f.open();
        let root = f.scene_root(document);

        f.signal(EntityEdit::Delete(root));
        let status = f.status(document);
        assert!(status.starts_with("Delete failed: "), "{status}");
        assert!(!f.unsaved(document));

        f.signal(EntityEdit::Create { parent: Some(root) });
        let created = f.selected(document);
        assert_ne!(created, Some(root));
        assert_eq!(
            f.world
                .get_resource::<HierarchyState>()
                .unwrap()
                .pending_scene_reveal(),
            created
        );
        assert!(f.unsaved(document));
    }

    #[test]
    fn a_reveal_from_a_document_that_is_not_in_front_is_ignored() {
        let mut f = SceneFixture::new();
        let document = f.open();
        let root = f.scene_root(document);
        f.activate(None);

        hosted::trigger_on(
            &mut f.world,
            document,
            EntityEdit::Create { parent: Some(root) },
        );

        assert!(f.unsaved(document));
        assert_eq!(
            f.world
                .get_resource::<HierarchyState>()
                .unwrap()
                .pending_scene_reveal(),
            None
        );
    }

    #[test]
    fn a_delivery_with_nothing_reported_leaves_the_document_and_the_tree_unchanged() {
        let mut f = SceneFixture::new();
        let document = f.open();
        f.world.tick();
        let since = f.world.current_tick();
        let tree = f.world.resource_changed_tick::<HierarchyState>();
        f.world.tick();

        f.deliver();

        assert!(!f.world.has_component_changed_since(
            document,
            std::any::TypeId::of::<EditorDocument>(),
            since
        ));
        assert_eq!(f.world.resource_changed_tick::<HierarchyState>(), tree);
    }

    #[test]
    fn an_edit_and_the_save_action_in_one_frame_request_a_save() {
        use concerto_window::input::actions::{ActionFired, ActionLabel};
        let mut f = SceneFixture::new();
        let document = f.open();
        let root = f.scene_root(document);

        f.signal(EntityEdit::Create { parent: Some(root) });
        f.world
            .insert_resource(EventChannel::<ActionFired>::default());
        f.world
            .get_resource_mut::<EventChannel<ActionFired>>()
            .unwrap()
            .push_event(ActionFired {
                action: crate::actions::Save.intern(),
            });
        let mut system = crate::asset_editor::request_save.into_system();
        system.initialize(&mut f.world);
        system.run_and_apply((), &mut f.world);

        assert!(
            f.world
                .get_component_for_entity::<SaveRequested>(document)
                .is_some()
        );
    }

    fn panicking_recipe(_: &mut App) {
        panic!("the recipe broke");
    }

    struct BreaksOnFinish;

    impl Plugin for BreaksOnFinish {
        fn build(&self, _: &mut App) {}

        fn finish(&self, _: &mut App) {
            panic!("finish broke");
        }
    }

    fn recipe_breaking_on_finish(app: &mut App) {
        crate::hosted::testing::scene_recipe(app);
        app.register_plugin(BreaksOnFinish);
    }

    fn opening_with_a_broken_recipe_reports_it_and_the_editor_keeps_going(
        broken: fn(&mut App),
        message: &str,
    ) {
        let mut f = SceneFixture::new();
        let working = f.world.get_resource::<SceneRecipe>().unwrap().recipe;
        f.world.get_resource_mut::<SceneRecipe>().unwrap().recipe = broken;
        let scene = f.scene.clone();

        f.request(&scene);
        f.frames_until_idle();

        let document = f.world.get_resource::<ActiveEditor>().unwrap().0.unwrap();
        assert_eq!(
            f.status(document),
            format!("Could not open scene: {message}")
        );
        assert!(f.document(document).pending.is_none());
        assert!(
            f.world
                .get_component_for_entity::<PendingApp>(document)
                .is_none()
        );
        assert!(
            f.world
                .get_component_for_entity::<HostedApp>(document)
                .is_none()
        );
        f.frame();

        f.world.get_resource_mut::<SceneRecipe>().unwrap().recipe = working;
        let reopened = f.open();
        assert!(f.scene_names(reopened).contains(&"first".to_string()));
    }

    #[test]
    fn a_recipe_that_panics_while_building_becomes_the_requests_error() {
        opening_with_a_broken_recipe_reports_it_and_the_editor_keeps_going(
            panicking_recipe,
            "the recipe broke",
        );
    }

    #[test]
    fn a_plugin_that_panics_in_finish_becomes_the_requests_error() {
        opening_with_a_broken_recipe_reports_it_and_the_editor_keeps_going(
            recipe_breaking_on_finish,
            "finish broke",
        );
    }

    fn startup_breaks() {
        panic!("startup broke");
    }

    fn startup_sleeps() {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    fn recipe_breaking_in_startup(app: &mut App) {
        use concerto_app::{edit_time::AtEditTime, schedule_groups::Startup};
        crate::hosted::testing::scene_recipe(app);
        app.add_system(Startup, startup_breaks.at_edit_time())
            .add_system(Startup, startup_sleeps.at_edit_time());
    }

    #[test]
    fn a_startup_system_that_panics_beside_a_running_one_becomes_the_requests_error() {
        for _ in 0..3 {
            opening_with_a_broken_recipe_reports_it_and_the_editor_keeps_going(
                recipe_breaking_in_startup,
                "startup broke",
            );
        }
    }

    #[test]
    fn a_scene_whose_project_closed_while_it_was_asked_for_says_to_open_a_project() {
        let mut f = SceneFixture::new();
        let scene = f.scene.clone();
        f.request(&scene);
        f.world.get_resource_mut::<ProjectState>().unwrap().project = None;

        f.frames_until_idle();

        let document = f.world.get_resource::<ActiveEditor>().unwrap().0.unwrap();
        assert_eq!(f.status(document), "Open a project before opening a scene");
        assert!(
            f.world
                .get_component_for_entity::<HostedApp>(document)
                .is_none()
        );
    }

    #[test]
    fn loader_rejects_wrong_identity_kind_and_corrupt_payload() {
        let dir = tempfile::tempdir().unwrap();
        let asset = scene_entry("a");
        write_scene_file(dir.path(), &asset, &["a"]);
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

    fn play(f: &mut SceneFixture, document: Entity) {
        f.world.trigger(Play(document));
    }

    fn playing() -> (SceneFixture, Entity) {
        let mut f = SceneFixture::new();
        let document = f.open();
        play(&mut f, document);
        f.frames_until_playing(document);
        (f, document)
    }

    fn holds(f: &mut SceneFixture, document: Entity, names: &[&str]) -> bool {
        let held = f.scene_names(document);
        names
            .iter()
            .all(|name| held.iter().any(|held| held == name))
    }

    fn saved_bytes(f: &SceneFixture) -> Vec<u8> {
        std::fs::read(f.project_root.join(&f.scene.address)).unwrap()
    }

    fn fire(f: &mut SceneFixture, action: impl concerto_window::input::actions::ActionLabel) {
        use concerto_window::input::actions::ActionFired;
        f.world
            .insert_resource(EventChannel::<ActionFired>::default());
        f.world
            .get_resource_mut::<EventChannel<ActionFired>>()
            .unwrap()
            .push_event(ActionFired {
                action: action.intern(),
            });
        let mut system = scene_actions.into_system();
        system.initialize(&mut f.world);
        system.run_and_apply((), &mut f.world);
    }

    #[test]
    fn play_starts_a_game_app_that_spawns_the_saved_scene_from_the_project_and_runs_the_game() {
        let mut f = SceneFixture::new();
        let document = f.open();
        let edit = scene_id(&f, document);

        play(&mut f, document);

        assert_eq!(f.play_state(document), Some(PlayState::Starting));
        assert!(f.starting_world(document).is_some());
        assert_eq!(scene_id(&f, document), edit);
        f.frames_until_playing(document);
        assert!(!f.hosted_is_editing(document));
        assert_eq!(f.parked_world(document).id(), edit);
        f.frames_until("the game never spawned the scene", |f| {
            holds(f, document, &["first", "second"])
        });
        assert!(f.game_ran(document));
        assert!(!f.unsaved(document));
    }

    #[test]
    fn play_on_an_unsaved_document_saves_it_first() {
        let mut f = SceneFixture::new();
        let document = f.open();
        f.rename(document, "first", "renamed");

        play(&mut f, document);

        assert!(!f.unsaved(document));
        assert!(f.saved_scene_names().contains(&"renamed".to_string()));
        assert_eq!(f.play_state(document), Some(PlayState::Starting));
        f.frames_until_playing(document);
        f.frames_until("the game never spawned the saved scene", |f| {
            holds(f, document, &["renamed", "second"])
        });
    }

    #[test]
    fn play_right_after_an_edit_the_editor_has_not_followed_yet_saves_it_first() {
        let mut f = SceneFixture::new();
        let document = f.open();
        let first = f.scene_entity(document, "first");
        f.scene_world_mut(document).trigger(EntityEdit::Rename {
            entity: first,
            name: "renamed".into(),
        });
        assert!(
            f.world
                .get_component_for_entity::<Unsaved>(document)
                .is_none()
        );

        play(&mut f, document);

        assert!(f.saved_scene_names().contains(&"renamed".to_string()));
        assert_eq!(f.play_state(document), Some(PlayState::Starting));
    }

    #[test]
    fn a_failed_save_does_not_start_the_game() {
        let mut f = SceneFixture::new();
        let document = f.open();
        f.rename(document, "first", "renamed");
        f.make_project_unwritable();

        play(&mut f, document);

        assert!(f.unsaved(document));
        assert!(
            f.status(document).starts_with("Save failed"),
            "{}",
            f.status(document)
        );
        assert_eq!(f.play_state(document), Some(PlayState::Editing));
        assert!(f.starting_world(document).is_none());
    }

    #[test]
    fn play_while_a_replacement_loads_is_refused() {
        let mut f = SceneFixture::new();
        let document = f.open();
        f.request_other_scene(document);

        play(&mut f, document);

        assert_eq!(f.status(document), "Wait for the scene to load");
        assert_eq!(f.play_state(document), Some(PlayState::Editing));
        assert!(f.starting_world(document).is_none());
    }

    #[test]
    fn an_edit_while_playing_changes_the_play_world_and_leaves_the_document_clean() {
        let (mut f, document) = playing();
        f.frames_until("the game never spawned the scene", |f| {
            holds(f, document, &["first"])
        });

        let first = f.scene_entity(document, "first");

        f.signal(crate::tools::edits::EditComponent(
            crate::tools::edits::ComponentEdit::Remove {
                entity: first,
                component: std::any::TypeId::of::<Transform>(),
            },
        ));

        let has_transform = |f: &SceneFixture, entity| {
            f.scene_world(document)
                .get_component_for_entity::<Transform>(entity)
                .is_some()
        };
        assert!(!has_transform(&f, first));
        assert!(!f.unsaved(document));
        stop(&mut f, document);
        let first = f.scene_entity(document, "first");
        assert!(has_transform(&f, first));
        assert!(!f.unsaved(document));
    }

    fn stop(f: &mut SceneFixture, document: Entity) {
        f.world.trigger(crate::hosted::StopPlay(document));
    }

    #[test]
    fn a_save_while_the_game_starts_or_plays_is_refused_and_writes_nothing() {
        let mut f = SceneFixture::new();
        let document = f.open();
        let before = saved_bytes(&f);
        play(&mut f, document);

        f.world.trigger(SaveScene { document });
        assert_eq!(f.status(document), "Stop to save");

        f.frames_until_playing(document);
        save(&mut f, document);
        assert_eq!(f.status(document), "Stop to save");
        assert!(
            f.world
                .get_component_for_entity::<SaveRequested>(document)
                .is_none()
        );

        f.world
            .get_component_for_entity_mut::<EditorDocument>(document)
            .unwrap()
            .status
            .clear();
        fire(&mut f, crate::actions::Save);
        assert_eq!(f.status(document), "Stop to save");
        assert_eq!(saved_bytes(&f), before);
    }

    #[test]
    fn opening_another_scene_while_playing_stops_the_game_then_opens_it() {
        let (mut f, document) = playing();

        f.request_other_scene(document);
        f.frames_until_idle();

        assert_eq!(f.play_state(document), Some(PlayState::Editing));
        assert!(
            f.world
                .get_component_for_entity::<crate::hosted::ParkedEditApp>(document)
                .is_none()
        );
        assert!(f.hosted_is_editing(document));
        assert_eq!(f.scene_names(document), ["third"]);
        assert_eq!(
            f.document(document).current.as_ref().unwrap().id,
            f.other.id
        );
    }

    #[test]
    fn a_game_that_fails_to_start_returns_to_editing_with_its_message() {
        let mut f = SceneFixture::new();
        let document = f.open();
        let edit = scene_id(&f, document);
        f.world.get_resource_mut::<SceneRecipe>().unwrap().recipe = panicking_recipe;

        play(&mut f, document);
        f.frame();

        assert_eq!(f.play_state(document), Some(PlayState::Editing));
        assert_eq!(
            f.status(document),
            "Could not start the game: the recipe broke"
        );
        assert_eq!(scene_id(&f, document), edit);
    }

    #[test]
    fn the_play_action_plays_while_editing_and_stops_while_starting_or_playing() {
        let mut f = SceneFixture::new();
        let document = f.open();
        let edit = scene_id(&f, document);

        fire(&mut f, crate::actions::TogglePlay);
        assert_eq!(f.play_state(document), Some(PlayState::Starting));
        fire(&mut f, crate::actions::TogglePlay);
        assert_eq!(f.play_state(document), Some(PlayState::Editing));
        assert!(f.starting_world(document).is_none());

        fire(&mut f, crate::actions::TogglePlay);
        f.frames_until_playing(document);
        fire(&mut f, crate::actions::TogglePlay);

        assert_eq!(f.play_state(document), Some(PlayState::Editing));
        assert_eq!(scene_id(&f, document), edit);
    }

    #[test]
    fn a_second_play_is_ignored_while_the_game_starts_or_plays() {
        let mut f = SceneFixture::new();
        let document = f.open();
        play(&mut f, document);
        let starting = f.starting_world(document);

        play(&mut f, document);
        assert_eq!(f.starting_world(document), starting);

        f.frames_until_playing(document);
        let game = scene_id(&f, document);
        play(&mut f, document);

        assert_eq!(f.play_state(document), Some(PlayState::Playing));
        assert_eq!(scene_id(&f, document), game);
        assert!(f.starting_world(document).is_none());
    }

    #[test]
    fn a_catalogue_change_while_playing_reaches_the_parked_edit_app_and_the_game() {
        let (mut f, document) = playing();
        let parked = f.parked_published_roots(document).len();
        let game = f.published_roots(document).len();

        f.world.trigger(PublishToHosted);

        assert_eq!(f.parked_published_roots(document).len(), parked + 1);
        assert_eq!(f.published_roots(document).len(), game + 1);
    }

    #[test]
    fn closing_a_playing_document_cancels_the_games_loads_and_drops_both_apps() {
        use std::sync::atomic::Ordering;
        let _gate = lock_gate();
        let mut f = SceneFixture::new();
        let project = f.world.get_resource_mut::<ProjectState>().unwrap();
        let project = project.project.as_mut().unwrap();
        let gated = save_content_asset_into(
            &GatedAsset,
            &project.root,
            "gated.gasset",
            &mut project.registry,
        )
        .unwrap();
        let document = f.open();
        play(&mut f, document);
        f.frames_until_playing(document);
        let mut server = f
            .scene_world(document)
            .get_resource::<AssetServer>()
            .unwrap()
            .clone();
        let store = concerto_foundation::assets::asset_store::AssetStore::<GatedAsset>::new();
        server.register_asset(&store);
        let dropped_before = GATED_DROPS.load(Ordering::SeqCst);
        let _handle = server.load::<GatedAsset>(gated);
        wait_for("the load never started", || ENTERED.load(Ordering::SeqCst));

        f.close(document);
        RELEASED.store(true, Ordering::SeqCst);

        assert!(!f.world.entity_is_valid(document));
        wait_for(
            "the game's load was delivered after its document closed",
            || GATED_DROPS.load(Ordering::SeqCst) > dropped_before,
        );
    }

    fn guard_asking(f: &SceneFixture) -> bool {
        f.world
            .get_resource::<crate::guard::UnsavedGuard>()
            .unwrap()
            .asking()
    }

    #[test]
    fn opening_another_scene_over_unsaved_changes_while_playing_stops_the_game_before_asking() {
        let (mut f, document) = playing();
        f.world.insert(Unsaved, document);

        f.request_other_scene(document);

        assert!(guard_asking(&f));
        assert_eq!(f.play_state(document), Some(PlayState::Editing));
        assert!(f.hosted_is_editing(document));
    }

    #[test]
    fn closing_a_document_with_unsaved_changes_while_playing_stops_the_game_before_asking() {
        let (mut f, document) = playing();
        f.world.insert(Unsaved, document);

        f.close(document);

        assert!(guard_asking(&f));
        assert!(f.world.entity_is_valid(document));
        assert_eq!(f.play_state(document), Some(PlayState::Editing));
        assert!(f.hosted_is_editing(document));
    }

    #[test]
    fn play_is_refused_while_the_unsaved_changes_prompt_is_open() {
        let mut f = SceneFixture::new();
        let document = f.open();
        f.world
            .get_resource_mut::<crate::guard::UnsavedGuard>()
            .unwrap()
            .hold(crate::guard::GuardedIntent::Quit, vec![document]);

        play(&mut f, document);
        assert_eq!(f.play_state(document), Some(PlayState::Editing));
        assert!(f.starting_world(document).is_none());

        fire(&mut f, crate::actions::TogglePlay);
        assert_eq!(f.play_state(document), Some(PlayState::Editing));
        assert!(f.starting_world(document).is_none());
    }

    #[test]
    fn play_is_refused_while_a_project_is_opening() {
        let mut f = SceneFixture::new();
        let document = f.open();
        let (release, held) = std::sync::mpsc::channel();
        f.world
            .get_resource_mut::<ProjectState>()
            .unwrap()
            .begin_job_for_test(held);

        play(&mut f, document);
        assert_eq!(f.play_state(document), Some(PlayState::Editing));
        assert!(f.starting_world(document).is_none());

        fire(&mut f, crate::actions::TogglePlay);
        assert_eq!(f.play_state(document), Some(PlayState::Editing));
        assert!(f.starting_world(document).is_none());
        drop(release);
    }
}
