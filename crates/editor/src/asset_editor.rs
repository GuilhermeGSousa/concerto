//! Lifecycle and registration for asset editors.

use anyhow::{Result, bail};
use concerto_app::App;
use concerto_ecs::events::event_reader::EventReader;
use concerto_ecs::{
    Component, Entity, Query, Resource,
    command::{CommandQueue, EntityCommandQueue},
    resource::{Res, ResMut},
};
use concerto_foundation::assets::Asset;
use concerto_window::input::actions::ActionFired;
use derive_more::Deref;
use std::collections::{HashMap, HashSet, VecDeque};

use crate::project::AssetEntry;

/// Installs an editor's components when its document is created.
pub trait AssetEditor: Send + Sync + 'static {
    fn build(&self, _editor: &mut EntityCommandQueue) {}
    /// Whether this editor's tabs show a close button.
    fn closable(&self) -> bool {
        true
    }
}

/// Registered asset editor implementations, keyed by asset type.
#[derive(Resource, Default)]
pub struct AssetEditorRegistry {
    editors: HashMap<&'static str, Box<dyn AssetEditor>>,
}

impl AssetEditorRegistry {
    pub fn register<T: Asset>(&mut self, editor: impl AssetEditor) -> Result<()> {
        if self.editors.contains_key(T::name()) {
            bail!("an editor is already registered for {}", T::name());
        }
        self.editors.insert(T::name(), Box::new(editor));
        Ok(())
    }

    /// Returns the registered asset kind matching `kind`, if it has an editor.
    pub fn editor_kind(&self, kind: &str) -> Option<&'static str> {
        self.editors.get_key_value(kind).map(|(kind, _)| *kind)
    }

    /// Whether a tab of `kind` shows a close button; a kind with no editor does.
    pub fn closable(&self, kind: &str) -> bool {
        self.editors
            .get(kind)
            .is_none_or(|editor| editor.closable())
    }
}

/// Application convenience API for asset editor registration.
pub trait AssetEditorAppExt {
    fn register_asset_editor<T: Asset>(&mut self, editor: impl AssetEditor) -> Result<&mut Self>;
}
impl AssetEditorAppExt for App {
    fn register_asset_editor<T: Asset>(&mut self, editor: impl AssetEditor) -> Result<&mut Self> {
        self.get_resource_mut::<AssetEditorRegistry>()
            .ok_or_else(|| anyhow::anyhow!("AssetEditorRegistry is not installed"))?
            .register::<T>(editor)?;
        Ok(self)
    }
}

#[derive(Component, Clone)]
pub struct EditorDocument {
    pub asset_type: &'static str,
    /// Tab title retained even if the initial load fails.
    pub title: String,
    pub current: Option<AssetEntry>,
    pub pending: Option<AssetEntry>,
    pub order: u64,
    pub status: String,
}

/// On a document whose content differs from what is on disk.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Unsaved;

/// Asks the editor owning this document to write it to disk; that editor removes the marker.
#[derive(Component)]
pub struct SaveRequested;

pub(crate) fn request_save(
    mut fired: EventReader<ActionFired>,
    active: Res<ActiveEditor>,
    documents: Query<&EditorDocument>,
    unsaved: Query<&Unsaved>,
    mut commands: CommandQueue,
) {
    let save = fired
        .read()
        .fold(false, |save, action| save | action.is(crate::actions::Save));
    let Some(entity) = active.0.filter(|_| save) else {
        return;
    };
    if unsaved.get_entity(entity).is_some()
        && documents
            .get_entity(entity)
            .is_some_and(|document| document.pending.is_none())
    {
        commands.insert(SaveRequested, entity);
    }
}

#[derive(Resource, Default, Deref)]
pub struct ActiveEditor(pub Option<Entity>);

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditorOwned(pub Entity);

#[derive(Component, Default, Clone, Copy)]
pub struct EditorHosts {
    pub canvas: Option<Entity>,
    pub left: Option<Entity>,
    pub inspector: Option<Entity>,
}

pub enum AssetEditorCommand {
    /// Opens an asset of the current project; one that is not in it is ignored.
    Open(AssetEntry),
    Activate(Entity),
    Close(Entity),
    CloseAll,
}
#[derive(Resource, Default)]
pub struct AssetEditorCommands(pub VecDeque<AssetEditorCommand>);

/// Applies the result of work started for `asset`, if the document still asks for it.
pub fn finish_asset_request(
    doc: &mut EditorDocument,
    asset: &AssetEntry,
    result: Result<()>,
) -> bool {
    if !asset_request_is_current(doc, asset) {
        return false;
    }
    match result {
        Ok(()) => {
            doc.current = doc.pending.take();
            if let Some(asset) = &doc.current {
                doc.title = asset.display_name.clone();
            }
            doc.status.clear();
        }
        Err(error) => {
            doc.status = format!("{error:#}");
            doc.pending = None;
        }
    }
    true
}

/// Whether work started for `asset` still matches the document's request.
pub fn asset_request_is_current(doc: &EditorDocument, asset: &AssetEntry) -> bool {
    doc.pending.as_ref() == Some(asset)
}

/// Process tab commands using declared ECS access.
pub fn process_editor_commands(
    mut requests: ResMut<AssetEditorCommands>,
    registry: Res<AssetEditorRegistry>,
    project: Res<crate::project::ProjectState>,
    mut active: ResMut<ActiveEditor>,
    mut guard: ResMut<crate::guard::UnsavedGuard>,
    documents: Query<(Entity, &EditorDocument)>,
    unsaved: Query<&Unsaved>,
    owned: Query<(Entity, &EditorOwned)>,
    states: Query<&crate::hosted::PlayState>,
    mut commands: CommandQueue,
) {
    if requests.0.is_empty() {
        return;
    }
    let stop_game = |entity: Entity, commands: &mut CommandQueue| {
        if states.get_entity(entity).is_some_and(|state| state.is_on()) {
            commands.trigger(crate::hosted::StopPlay(entity));
        }
    };
    let mut next_active = active.0;
    let mut staged: Vec<_> = documents.iter().map(|(e, d)| (e, d.clone())).collect();
    let mut created = HashSet::new();
    let mut changed = HashSet::new();
    let mut closed = HashSet::new();
    for request in requests.0.drain(..) {
        match request {
            AssetEditorCommand::Open(asset) => {
                if !project
                    .project
                    .as_ref()
                    .is_some_and(|project| project.registry.get(asset.id).is_some())
                {
                    continue;
                }
                let Some(asset_type) = registry.editor_kind(&asset.kind) else {
                    continue;
                };
                if let Some((entity, doc)) =
                    staged.iter_mut().find(|(_, d)| d.asset_type == asset_type)
                {
                    let is_unsaved =
                        !created.contains(entity) && unsaved.get_entity(*entity).is_some();
                    if is_unsaved && !doc.current.as_ref().is_some_and(|a| a.id == asset.id) {
                        stop_game(*entity, &mut commands);
                        guard.hold(
                            crate::guard::GuardedIntent::Editor(AssetEditorCommand::Open(asset)),
                            vec![*entity],
                        );
                        continue;
                    }
                    if !doc.pending.as_ref().is_some_and(|a| a.id == asset.id) {
                        if doc.current.as_ref().is_some_and(|a| a.id == asset.id) {
                            doc.pending = None;
                            doc.status.clear();
                        } else {
                            doc.status = format!("Opening {}…", asset.address);
                            doc.pending = Some(asset);
                        }
                        changed.insert(*entity);
                    }
                    next_active = Some(*entity);
                } else {
                    let order = staged.iter().map(|(_, d)| d.order).max().unwrap_or(0) + 1;
                    let entity = commands.spawn(()).entity();
                    staged.push((
                        entity,
                        EditorDocument {
                            asset_type,
                            title: asset.display_name.clone(),
                            status: format!("Opening {}…", asset.address),
                            current: None,
                            pending: Some(asset),
                            order,
                        },
                    ));
                    created.insert(entity);
                    changed.insert(entity);
                    next_active = Some(entity);
                }
            }
            AssetEditorCommand::Activate(entity) => {
                if staged.iter().any(|(e, _)| *e == entity) {
                    next_active = Some(entity);
                }
            }
            AssetEditorCommand::Close(entity) => {
                if let Some(index) = staged.iter().position(|(e, _)| *e == entity) {
                    if !created.contains(&entity) && unsaved.get_entity(entity).is_some() {
                        stop_game(entity, &mut commands);
                        guard.hold(
                            crate::guard::GuardedIntent::Editor(AssetEditorCommand::Close(entity)),
                            vec![entity],
                        );
                        continue;
                    }
                    let (_, doc) = staged.remove(index);
                    closed.insert(entity);
                    if next_active == Some(entity) {
                        next_active = staged
                            .iter()
                            .filter(|(_, d)| d.order < doc.order)
                            .max_by_key(|(_, d)| d.order)
                            .or_else(|| staged.iter().min_by_key(|(_, d)| d.order))
                            .map(|(e, _)| *e);
                    }
                }
            }
            AssetEditorCommand::CloseAll => {
                closed.extend(staged.drain(..).map(|(e, _)| e));
                next_active = None;
            }
        }
    }
    if active.0 != next_active {
        active.0 = next_active;
        commands.trigger(crate::hosted::SetEditTarget(next_active));
    }
    for (entity, doc) in staged {
        if created.contains(&entity) {
            if let Some(editor) = registry.editors.get(doc.asset_type) {
                editor.build(&mut commands.entity(entity));
            }
        }
        if changed.contains(&entity) {
            commands.insert(doc, entity);
        }
    }
    for (entity, owner) in owned.iter() {
        if closed.contains(&owner.0) {
            commands.despawn(entity);
        }
    }
    for entity in closed {
        stop_game(entity, &mut commands);
        commands.despawn(entity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use concerto_ecs::{IntoSystem, System, World};
    fn process_editor_commands(world: &mut World) {
        let mut system = super::process_editor_commands.into_system();
        system.initialize(world);
        system.run_and_apply((), world);
    }
    fn finish_asset_request(
        world: &mut World,
        editor: Entity,
        asset: &AssetEntry,
        result: Result<()>,
    ) -> bool {
        world
            .get_component_for_entity_mut::<EditorDocument>(editor)
            .is_some_and(|mut doc| super::finish_asset_request(&mut doc, asset, result))
    }
    use concerto_foundation::assets::AssetId;

    #[derive(serde::Serialize, serde::Deserialize)]
    struct TestAsset;
    impl Asset for TestAsset {
        fn name() -> &'static str {
            "TestAsset"
        }
    }
    #[derive(serde::Serialize, serde::Deserialize)]
    struct OtherAsset;
    impl Asset for OtherAsset {
        fn name() -> &'static str {
            "OtherAsset"
        }
    }
    struct Fake;
    impl AssetEditor for Fake {}
    struct OtherFake;
    impl AssetEditor for OtherFake {}
    fn asset(path: &str) -> AssetEntry {
        AssetEntry {
            id: AssetId::from_path(path),
            address: path.into(),
            kind: "TestAsset".into(),
            display_name: path.into(),
            folder: String::new(),
            provenance: Some(concerto_foundation::assets::content::ImportProvenance {
                source: path.into(),
                sub_asset: String::new(),
            }),
        }
    }
    fn world() -> World {
        let mut w = World::new();
        w.insert_resource(AssetEditorCommands::default());
        w.insert_resource(ActiveEditor::default());
        w.insert_resource(crate::guard::UnsavedGuard::default());
        let mut registry = concerto_foundation::assets::content::AssetRegistry::default();
        for path in ["a", "b", "c", "d", "other", "scene", "third"] {
            registry.insert(asset(path).id, path);
        }
        let mut project = crate::project::ProjectState::default();
        project.project = Some(crate::project::Project {
            root: std::path::PathBuf::from("project"),
            assets: vec![],
            registry,
            content_root: "content".into(),
        });
        w.insert_resource(project);
        let mut r = AssetEditorRegistry::default();
        r.register::<TestAsset>(Fake).unwrap();
        w.insert_resource(r);
        w
    }
    fn open(w: &mut World, path: &str) -> Entity {
        w.get_resource_mut::<AssetEditorCommands>()
            .unwrap()
            .0
            .push_back(AssetEditorCommand::Open(asset(path)));
        process_editor_commands(w);
        w.get_resource::<ActiveEditor>().unwrap().0.unwrap()
    }
    #[test]
    fn selecting_active_document_does_not_mark_active_resource_changed() {
        let mut w = world();
        let e = open(&mut w, "a");
        let opened = w.resource_changed_tick::<ActiveEditor>();
        w.get_resource_mut::<AssetEditorCommands>()
            .unwrap()
            .0
            .push_back(AssetEditorCommand::Activate(e));
        process_editor_commands(&mut w);
        assert_eq!(w.resource_changed_tick::<ActiveEditor>(), opened);
    }

    #[test]
    fn lifecycle_declares_scoped_access_and_batches_deferred_spawns() {
        let system = super::process_editor_commands.into_system();
        let mut meta = concerto_ecs::system::meta::SystemMetadata::default();
        let mut access = concerto_ecs::system::access::SystemAccess::default();
        system.fill_access(&mut meta, &mut access);
        assert!(!access.is_exclusive());
        assert!(access.needs_apply());

        let mut w = world();
        w.get_resource_mut::<AssetEditorCommands>()
            .unwrap()
            .0
            .extend([
                AssetEditorCommand::Open(asset("a")),
                AssetEditorCommand::Open(asset("b")),
            ]);
        process_editor_commands(&mut w);
        let entity = w.get_resource::<ActiveEditor>().unwrap().0.unwrap();
        assert_eq!(w.query::<&EditorDocument, ()>().iter(&mut w).count(), 1);
        let doc = w
            .get_component_for_entity::<EditorDocument>(entity)
            .unwrap();
        assert_eq!(doc.pending.as_ref().unwrap().id, asset("b").id);

        w.get_resource_mut::<AssetEditorCommands>()
            .unwrap()
            .0
            .extend([
                AssetEditorCommand::CloseAll,
                AssetEditorCommand::Open(asset("c")),
                AssetEditorCommand::CloseAll,
                AssetEditorCommand::Open(asset("d")),
            ]);
        process_editor_commands(&mut w);
        assert!(!w.entity_is_valid(entity));
        assert_eq!(w.query::<&EditorDocument, ()>().iter(&mut w).count(), 1);
        let current = w.get_resource::<ActiveEditor>().unwrap().0.unwrap();
        let doc = w
            .get_component_for_entity::<EditorDocument>(current)
            .unwrap();
        assert_eq!(doc.pending.as_ref().unwrap().id, asset("d").id);
    }

    #[test]
    fn an_open_for_an_asset_outside_the_current_project_is_dropped() {
        let mut w = world();

        push(&mut w, AssetEditorCommand::Open(asset("elsewhere")));

        assert_eq!(w.query::<&EditorDocument, ()>().iter(&mut w).count(), 0);
        assert_eq!(w.get_resource::<ActiveEditor>().unwrap().0, None);
    }

    #[test]
    fn a_result_for_a_replaced_request_is_not_applied() {
        let mut w = world();
        let e = open(&mut w, "a");

        push(&mut w, AssetEditorCommand::Open(asset("b")));

        assert!(!finish_asset_request(&mut w, e, &asset("a"), Ok(())));
        let doc = w.get_component_for_entity::<EditorDocument>(e).unwrap();
        assert!(doc.current.is_none());
        assert_eq!(doc.pending, Some(asset("b")));
    }

    #[test]
    fn reopening_the_pending_asset_keeps_the_request_and_reopening_the_current_one_cancels_it() {
        let mut w = world();
        let e = open(&mut w, "a");
        assert!(finish_asset_request(&mut w, e, &asset("a"), Ok(())));
        push(&mut w, AssetEditorCommand::Open(asset("b")));

        push(&mut w, AssetEditorCommand::Open(asset("b")));
        assert_eq!(
            w.get_component_for_entity::<EditorDocument>(e)
                .unwrap()
                .pending,
            Some(asset("b"))
        );

        push(&mut w, AssetEditorCommand::Open(asset("a")));
        assert!(!finish_asset_request(&mut w, e, &asset("b"), Ok(())));
        assert_eq!(w.get_resource::<ActiveEditor>().unwrap().0, Some(e));
    }

    #[test]
    fn a_failed_replacement_keeps_the_current_asset_and_no_project_takes_no_open() {
        let mut w = world();
        let e = open(&mut w, "a");
        assert!(finish_asset_request(&mut w, e, &asset("a"), Ok(())));
        push(&mut w, AssetEditorCommand::Open(asset("b")));

        assert!(finish_asset_request(
            &mut w,
            e,
            &asset("b"),
            Err(anyhow::anyhow!("bad"))
        ));
        assert_eq!(
            w.get_component_for_entity::<EditorDocument>(e)
                .unwrap()
                .current
                .as_ref()
                .unwrap()
                .id,
            AssetId::from_path("a")
        );

        w.insert_resource(crate::project::ProjectState::default());
        push(&mut w, AssetEditorCommand::Open(asset("c")));
        assert_eq!(w.get_resource::<ActiveEditor>().unwrap().0, Some(e));
        assert!(
            w.get_component_for_entity::<EditorDocument>(e)
                .unwrap()
                .pending
                .is_none()
        );
    }

    #[test]
    fn owned_tree_removed_on_close() {
        let mut w = world();
        let e = open(&mut w, "a");
        let child = w.spawn((EditorOwned(e),));
        let grandchild = w.spawn((EditorOwned(e),));
        w.entity_mut(child).add_child(grandchild);
        w.get_resource_mut::<AssetEditorCommands>()
            .unwrap()
            .0
            .push_back(AssetEditorCommand::Close(e));
        process_editor_commands(&mut w);
        assert!(!w.entity_is_valid(e));
        assert!(!w.entity_is_valid(child));
        assert!(!w.entity_is_valid(grandchild));
    }
    fn fire_save(w: &mut World) {
        use concerto_ecs::events::event_channel::EventChannel;
        use concerto_window::input::actions::ActionLabel;
        if w.get_resource::<EventChannel<ActionFired>>().is_none() {
            w.insert_resource(EventChannel::<ActionFired>::default());
        }
        let channel = w.get_resource_mut::<EventChannel<ActionFired>>().unwrap();
        channel.update();
        channel.update();
        channel.push_event(ActionFired {
            action: crate::actions::Save.intern(),
        });
        let mut system = request_save.into_system();
        system.initialize(w);
        system.run_and_apply((), w);
    }

    #[test]
    fn the_save_action_asks_only_an_unsaved_document_with_no_replacement_pending() {
        let mut w = world();
        let e = open(&mut w, "a");
        finish(&mut w, e);
        let requested = |w: &World| w.get_component_for_entity::<SaveRequested>(e).is_some();

        fire_save(&mut w);
        assert!(!requested(&w), "a clean document has nothing to save");

        w.insert(Unsaved, e);
        w.get_component_for_entity_mut::<EditorDocument>(e)
            .unwrap()
            .pending = Some(asset("b"));
        fire_save(&mut w);
        assert!(!requested(&w), "a replacement is loading");

        w.get_component_for_entity_mut::<EditorDocument>(e)
            .unwrap()
            .pending = None;
        fire_save(&mut w);
        assert!(requested(&w));
    }

    fn finish(w: &mut World, e: Entity) {
        let pending = w
            .get_component_for_entity::<EditorDocument>(e)
            .unwrap()
            .pending
            .clone()
            .unwrap();
        assert!(finish_asset_request(w, e, &pending, Ok(())));
    }
    fn push(w: &mut World, command: AssetEditorCommand) {
        w.get_resource_mut::<AssetEditorCommands>()
            .unwrap()
            .0
            .push_back(command);
        process_editor_commands(w);
    }
    fn asking_about(w: &World) -> Vec<Entity> {
        w.get_resource::<crate::guard::UnsavedGuard>()
            .unwrap()
            .documents()
            .to_vec()
    }

    #[test]
    fn replacing_or_closing_a_dirty_document_is_held_for_the_guard() {
        let mut w = world();
        let e = open(&mut w, "a");
        finish(&mut w, e);
        w.insert(Unsaved, e);

        open(&mut w, "a");
        assert!(
            asking_about(&w).is_empty(),
            "reselecting the open asset is harmless"
        );

        open(&mut w, "b");
        let doc = w.get_component_for_entity::<EditorDocument>(e).unwrap();
        assert!(doc.pending.is_none());
        assert_eq!(asking_about(&w), [e]);

        w.insert_resource(crate::guard::UnsavedGuard::default());
        push(&mut w, AssetEditorCommand::Close(e));
        assert!(w.entity_is_valid(e));
        assert_eq!(asking_about(&w), [e]);
        assert_eq!(w.get_resource::<ActiveEditor>().unwrap().0, Some(e));

        w.insert_resource(crate::guard::UnsavedGuard::default());
        w.remove_component::<Unsaved>(e);
        push(&mut w, AssetEditorCommand::Close(e));
        assert!(!w.entity_is_valid(e));
        assert!(asking_about(&w).is_empty());
    }

    #[test]
    fn the_close_command_closes_a_scene_document_whose_tab_has_no_close_button() {
        let mut w = world();
        let registry = w.get_resource_mut::<AssetEditorRegistry>().unwrap();
        registry
            .register::<concerto_scene::scene::Scene>(crate::scene::SceneEditor)
            .unwrap();
        assert!(!registry.closable(concerto_scene::scene::Scene::name()));
        assert!(registry.closable(TestAsset::name()));
        assert!(registry.closable("Unknown"));
        let mut scene = asset("scene");
        scene.kind = concerto_scene::scene::Scene::name().into();
        w.get_resource_mut::<AssetEditorCommands>()
            .unwrap()
            .0
            .push_back(AssetEditorCommand::Open(scene));
        process_editor_commands(&mut w);
        let e = w.get_resource::<ActiveEditor>().unwrap().0.unwrap();
        finish(&mut w, e);
        push(&mut w, AssetEditorCommand::Close(e));
        assert!(!w.entity_is_valid(e));
        assert!(w.get_resource::<ActiveEditor>().unwrap().0.is_none());
    }

    #[derive(serde::Serialize, serde::Deserialize)]
    struct ThirdAsset;
    impl Asset for ThirdAsset {
        fn name() -> &'static str {
            "ThirdAsset"
        }
    }

    #[test]
    fn closing_prefers_left_neighbor_then_right_and_rejects_old_completions() {
        let mut w = world();
        let registry = w.get_resource_mut::<AssetEditorRegistry>().unwrap();
        registry.register::<OtherAsset>(OtherFake).unwrap();
        registry.register::<ThirdAsset>(OtherFake).unwrap();
        let a = open(&mut w, "a");
        let mut other = asset("other");
        other.kind = OtherAsset::name().into();
        let mut third = asset("third");
        third.kind = ThirdAsset::name().into();
        w.get_resource_mut::<AssetEditorCommands>()
            .unwrap()
            .0
            .push_back(AssetEditorCommand::Open(other));
        process_editor_commands(&mut w);
        let b = w.get_resource::<ActiveEditor>().unwrap().0.unwrap();
        w.get_resource_mut::<AssetEditorCommands>()
            .unwrap()
            .0
            .push_back(AssetEditorCommand::Open(third));
        process_editor_commands(&mut w);
        let c = w.get_resource::<ActiveEditor>().unwrap().0.unwrap();
        w.get_resource_mut::<AssetEditorCommands>()
            .unwrap()
            .0
            .extend([
                AssetEditorCommand::Activate(b),
                AssetEditorCommand::Close(b),
            ]);
        process_editor_commands(&mut w);
        assert_eq!(w.get_resource::<ActiveEditor>().unwrap().0, Some(a));
        assert!(!finish_asset_request(&mut w, b, &asset("b"), Ok(())));
        w.get_resource_mut::<AssetEditorCommands>()
            .unwrap()
            .0
            .push_back(AssetEditorCommand::Close(a));
        process_editor_commands(&mut w);
        assert_eq!(w.get_resource::<ActiveEditor>().unwrap().0, Some(c));
        w.get_resource_mut::<AssetEditorCommands>()
            .unwrap()
            .0
            .push_back(AssetEditorCommand::Close(c));
        process_editor_commands(&mut w);
        assert!(w.get_resource::<ActiveEditor>().unwrap().0.is_none());
    }

    #[test]
    fn unsupported_assets_do_not_open_tabs_and_duplicate_registration_is_rejected() {
        let mut w = world();
        let mut unknown = asset("unknown");
        unknown.kind = "Unknown".into();
        let mut unsupported = asset("other");
        unsupported.kind = OtherAsset::name().into();
        for asset in [unknown, unsupported] {
            w.get_resource_mut::<crate::project::ProjectState>()
                .unwrap()
                .project
                .as_mut()
                .unwrap()
                .registry
                .insert(asset.id, asset.address.clone());
            w.get_resource_mut::<AssetEditorCommands>()
                .unwrap()
                .0
                .push_back(AssetEditorCommand::Open(asset));
        }
        process_editor_commands(&mut w);
        assert!(w.get_resource::<ActiveEditor>().unwrap().0.is_none());
        assert_eq!(w.query::<&EditorDocument, ()>().iter(&mut w).count(), 0);
        assert!(
            w.get_resource_mut::<AssetEditorRegistry>()
                .unwrap()
                .register::<TestAsset>(Fake)
                .is_err()
        );
    }
}
