//! Lifecycle and registration for asset editors.

use anyhow::{Result, bail};
use concerto_app::App;
use concerto_ecs::events::event_reader::EventReader;
use concerto_ecs::{
    Component, Entity, Query, Resource, World,
    command::{CommandQueue, EntityCommandQueue},
    entity::hierarchy::ChildOf,
    resource::{Res, ResMut},
};
use concerto_foundation::assets::Asset;
use concerto_window::input::actions::ActionFired;
use std::collections::{HashMap, HashSet, VecDeque};

use crate::project::AssetEntry;

/// Installs an editor's components when its document is created.
pub trait AssetEditor: Send + Sync + 'static {
    fn build(&self, _editor: &mut EntityCommandQueue) {}
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
    pub project_generation: u64,
    pub request_generation: u64,
    pub order: u64,
    pub status: String,
    /// Bumped by every edit; the document is dirty while it differs from `saved_revision`.
    pub revision: u64,
    pub saved_revision: u64,
}

impl EditorDocument {
    pub fn mark_edited(&mut self) {
        self.revision += 1;
    }

    /// Records that `revision` is what is on disk, so edits made since it was captured stay unsaved.
    pub fn mark_saved(&mut self, revision: u64) {
        self.saved_revision = revision;
    }

    pub fn is_dirty(&self) -> bool {
        self.revision != self.saved_revision
    }
}

/// Marks the document owning `entity`, found through its nearest [`EditorOwned`] ancestor, as edited.
pub fn mark_entity_edited(world: &mut World, entity: Entity) {
    if let Some(document) = owning_document(world, entity)
        .and_then(|document| world.get_component_for_entity_mut::<EditorDocument>(document))
    {
        document.mark_edited();
    }
}

/// [`mark_entity_edited`] for a system that holds queries instead of the world.
pub fn mark_edited_with(
    entity: Entity,
    owners: &Query<&EditorOwned>,
    parents: &Query<&ChildOf>,
    documents: &Query<&mut EditorDocument>,
) {
    let mut current = entity;
    let document = loop {
        if let Some(owner) = owners.get_entity(current) {
            break owner.0;
        }
        let Some(parent) = parents.get_entity(current) else {
            return;
        };
        current = parent.parent();
    };
    if let Some(mut document) = documents.get_entity(document) {
        document.mark_edited();
    }
}

/// The document named by the nearest [`EditorOwned`] on `entity` or its ancestors.
pub fn owning_document(world: &World, entity: Entity) -> Option<Entity> {
    let mut current = entity;
    loop {
        if let Some(owner) = world.get_component_for_entity::<EditorOwned>(current) {
            return Some(owner.0);
        }
        current = world.get_component_for_entity::<ChildOf>(current)?.parent();
    }
}

/// Asks the editor owning this document to write it to disk; that editor removes the marker.
#[derive(Component)]
pub struct SaveRequested;

pub(crate) fn request_save(
    mut fired: EventReader<ActionFired>,
    active: Res<ActiveEditor>,
    documents: Query<&EditorDocument>,
    mut commands: CommandQueue,
) {
    let save = fired
        .read()
        .fold(false, |save, action| save | action.is(crate::actions::Save));
    let Some(entity) = active.0.filter(|_| save) else {
        return;
    };
    if documents
        .get_entity(entity)
        .is_some_and(|document| document.is_dirty() && document.pending.is_none())
    {
        commands.insert(SaveRequested, entity);
    }
}

#[derive(Resource, Default)]
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
    Open {
        asset: AssetEntry,
        project_generation: u64,
    },
    Activate(Entity),
    Close(Entity),
    CloseAll,
}
#[derive(Resource, Default)]
pub struct AssetEditorCommands(pub VecDeque<AssetEditorCommand>);

/// Complete a request through a scoped document borrow.
pub fn finish_asset_request(
    doc: &mut EditorDocument,
    generation: u64,
    project_generation: u64,
    result: Result<()>,
) -> bool {
    if !asset_request_is_current(doc, generation, project_generation) {
        return false;
    }
    match result {
        Ok(()) => {
            doc.current = doc.pending.take();
            if let Some(asset) = &doc.current {
                doc.title = asset.display_name.clone();
            }
            doc.status.clear();
            doc.revision = 0;
            doc.saved_revision = 0;
        }
        Err(error) => {
            doc.status = format!("{error:#}");
            doc.pending = None;
        }
    }
    true
}

/// Check before applying asynchronous results to the live presentation.
pub fn asset_request_is_current(
    doc: &EditorDocument,
    generation: u64,
    project_generation: u64,
) -> bool {
    doc.request_generation == generation
        && doc.pending.is_some()
        && doc.project_generation == project_generation
}

/// Process tab commands using declared ECS access.
pub fn process_editor_commands(
    mut requests: ResMut<AssetEditorCommands>,
    registry: Res<AssetEditorRegistry>,
    project: Res<crate::project::ProjectState>,
    mut active: ResMut<ActiveEditor>,
    mut guard: ResMut<crate::guard::UnsavedGuard>,
    documents: Query<(Entity, &EditorDocument)>,
    owned: Query<(Entity, &EditorOwned)>,
    mut commands: CommandQueue,
) {
    if requests.0.is_empty() {
        return;
    }
    let mut next_active = active.0;
    let mut staged: Vec<_> = documents.iter().map(|(e, d)| (e, d.clone())).collect();
    let mut created = HashSet::new();
    let mut changed = HashSet::new();
    let mut closed = HashSet::new();
    for request in requests.0.drain(..) {
        match request {
            AssetEditorCommand::Open {
                asset,
                project_generation,
            } => {
                if project.generation != project_generation {
                    continue;
                }
                let Some(asset_type) = registry.editor_kind(&asset.kind) else {
                    continue;
                };
                if let Some((entity, doc)) =
                    staged.iter_mut().find(|(_, d)| d.asset_type == asset_type)
                {
                    if doc.is_dirty() && !doc.current.as_ref().is_some_and(|a| a.id == asset.id) {
                        guard.hold(
                            crate::guard::GuardedIntent::Editor(AssetEditorCommand::Open {
                                asset,
                                project_generation,
                            }),
                            vec![*entity],
                        );
                        continue;
                    }
                    if !doc.pending.as_ref().is_some_and(|a| a.id == asset.id) {
                        doc.request_generation += 1;
                        doc.project_generation = project_generation;
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
                            project_generation,
                            request_generation: 1,
                            order,
                            revision: 0,
                            saved_revision: 0,
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
                    if staged[index].1.is_dirty() {
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
        generation: u64,
        result: Result<()>,
    ) -> bool {
        let project = world
            .get_resource::<crate::project::ProjectState>()
            .unwrap()
            .generation;
        world
            .get_component_for_entity_mut::<EditorDocument>(editor)
            .is_some_and(|mut doc| {
                super::finish_asset_request(&mut doc, generation, project, result)
            })
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
        let mut project = crate::project::ProjectState::default();
        project.generation = 1;
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
            .push_back(AssetEditorCommand::Open {
                asset: asset(path),
                project_generation: 1,
            });
        process_editor_commands(w);
        w.get_resource::<ActiveEditor>().unwrap().0.unwrap()
    }
    #[test]
    fn selecting_active_document_does_not_mark_active_resource_changed() {
        let mut w = world();
        let e = open(&mut w, "a");
        w.tick();
        w.get_resource_mut::<AssetEditorCommands>()
            .unwrap()
            .0
            .push_back(AssetEditorCommand::Activate(e));
        process_editor_commands(&mut w);
        let mut check = (|active: Res<ActiveEditor>| {
            use concerto_ecs::query::change_detection::DetectChanges;
            assert!(!active.has_changed());
        })
        .into_system();
        check.initialize(&mut w);
        check.run_and_apply((), &mut w);
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
                AssetEditorCommand::Open {
                    asset: asset("a"),
                    project_generation: 1,
                },
                AssetEditorCommand::Open {
                    asset: asset("b"),
                    project_generation: 1,
                },
            ]);
        process_editor_commands(&mut w);
        let entity = w.get_resource::<ActiveEditor>().unwrap().0.unwrap();
        assert_eq!(w.query::<&EditorDocument, ()>().iter(&mut w).count(), 1);
        let doc = w
            .get_component_for_entity::<EditorDocument>(entity)
            .unwrap();
        assert_eq!(doc.pending.as_ref().unwrap().id, asset("b").id);
        assert_eq!(doc.request_generation, 2);

        w.get_resource_mut::<AssetEditorCommands>()
            .unwrap()
            .0
            .extend([
                AssetEditorCommand::CloseAll,
                AssetEditorCommand::Open {
                    asset: asset("c"),
                    project_generation: 1,
                },
                AssetEditorCommand::CloseAll,
                AssetEditorCommand::Open {
                    asset: asset("d"),
                    project_generation: 1,
                },
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
    fn completion_rejects_changed_project_without_mutating_document() {
        let mut w = world();
        let e = open(&mut w, "a");
        w.get_resource_mut::<crate::project::ProjectState>()
            .unwrap()
            .generation = 2;
        assert!(!finish_asset_request(&mut w, e, 1, Ok(())));
        let doc = w.get_component_for_entity::<EditorDocument>(e).unwrap();
        assert!(doc.current.is_none());
        assert!(doc.pending.is_some());
    }

    #[test]
    fn one_document_and_stale_generation() {
        let mut w = world();
        let e = open(&mut w, "a");
        let g = w
            .get_component_for_entity::<EditorDocument>(e)
            .unwrap()
            .request_generation;
        assert!(finish_asset_request(&mut w, e, g, Ok(())));
        w.get_resource_mut::<AssetEditorCommands>()
            .unwrap()
            .0
            .push_back(AssetEditorCommand::Open {
                asset: asset("b"),
                project_generation: 1,
            });
        process_editor_commands(&mut w);
        let g = w
            .get_component_for_entity::<EditorDocument>(e)
            .unwrap()
            .request_generation;
        w.get_resource_mut::<AssetEditorCommands>()
            .unwrap()
            .0
            .push_back(AssetEditorCommand::Open {
                asset: asset("b"),
                project_generation: 1,
            });
        process_editor_commands(&mut w);
        assert_eq!(
            w.get_component_for_entity::<EditorDocument>(e)
                .unwrap()
                .request_generation,
            g
        );
        assert!(
            w.get_component_for_entity::<EditorDocument>(e)
                .unwrap()
                .pending
                .is_some()
        );
        w.get_resource_mut::<AssetEditorCommands>()
            .unwrap()
            .0
            .push_back(AssetEditorCommand::Open {
                asset: asset("a"),
                project_generation: 1,
            });
        process_editor_commands(&mut w);
        assert!(!finish_asset_request(&mut w, e, g, Ok(())));
        assert_eq!(w.get_resource::<ActiveEditor>().unwrap().0, Some(e));
    }
    #[test]
    fn failed_replacement_keeps_current_and_stale_project_is_ignored() {
        let mut w = world();
        let e = open(&mut w, "a");
        let g = w
            .get_component_for_entity::<EditorDocument>(e)
            .unwrap()
            .request_generation;
        assert!(finish_asset_request(&mut w, e, g, Ok(())));
        w.get_resource_mut::<AssetEditorCommands>()
            .unwrap()
            .0
            .push_back(AssetEditorCommand::Open {
                asset: asset("b"),
                project_generation: 1,
            });
        process_editor_commands(&mut w);
        let g = w
            .get_component_for_entity::<EditorDocument>(e)
            .unwrap()
            .request_generation;
        assert!(finish_asset_request(
            &mut w,
            e,
            g,
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
        w.get_resource_mut::<crate::project::ProjectState>()
            .unwrap()
            .generation = 2;
        w.get_resource_mut::<AssetEditorCommands>()
            .unwrap()
            .0
            .push_back(AssetEditorCommand::Open {
                asset: asset("c"),
                project_generation: 1,
            });
        process_editor_commands(&mut w);
        assert_eq!(w.get_resource::<ActiveEditor>().unwrap().0, Some(e));
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
    #[test]
    fn edits_mark_the_owning_document_until_the_captured_revision_is_saved() {
        let mut w = world();
        let e = open(&mut w, "a");
        let root = w.spawn((EditorOwned(e),));
        let child = w.spawn(());
        let grandchild = w.spawn(());
        w.entity_mut(root).add_child(child);
        w.entity_mut(child).add_child(grandchild);
        let unowned = w.spawn(());
        let dirty = |w: &World| {
            w.get_component_for_entity::<EditorDocument>(e)
                .unwrap()
                .is_dirty()
        };

        mark_entity_edited(&mut w, unowned);
        assert!(!dirty(&w));

        mark_entity_edited(&mut w, grandchild);
        assert!(dirty(&w));

        let captured = w
            .get_component_for_entity::<EditorDocument>(e)
            .unwrap()
            .revision;
        mark_entity_edited(&mut w, root);
        w.get_component_for_entity_mut::<EditorDocument>(e)
            .unwrap()
            .mark_saved(captured);
        assert!(dirty(&w), "an edit after the capture is still unsaved");

        let latest = w
            .get_component_for_entity::<EditorDocument>(e)
            .unwrap()
            .revision;
        w.get_component_for_entity_mut::<EditorDocument>(e)
            .unwrap()
            .mark_saved(latest);
        assert!(!dirty(&w));
    }

    fn finish(w: &mut World, e: Entity) {
        let g = w
            .get_component_for_entity::<EditorDocument>(e)
            .unwrap()
            .request_generation;
        assert!(finish_asset_request(w, e, g, Ok(())));
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
        w.get_component_for_entity_mut::<EditorDocument>(e)
            .unwrap()
            .mark_edited();

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
        let doc = w.get_component_for_entity_mut::<EditorDocument>(e).unwrap();
        let revision = doc.revision;
        doc.mark_saved(revision);
        push(&mut w, AssetEditorCommand::Close(e));
        assert!(!w.entity_is_valid(e));
        assert!(asking_about(&w).is_empty());
    }

    #[test]
    fn a_successful_load_clears_dirty_state() {
        let mut w = world();
        let e = open(&mut w, "a");
        w.get_component_for_entity_mut::<EditorDocument>(e)
            .unwrap()
            .mark_edited();
        assert!(finish_asset_request(&mut w, e, 1, Ok(())));
        assert!(
            !w.get_component_for_entity::<EditorDocument>(e)
                .unwrap()
                .is_dirty()
        );
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
            .push_back(AssetEditorCommand::Open {
                asset: other,
                project_generation: 1,
            });
        process_editor_commands(&mut w);
        let b = w.get_resource::<ActiveEditor>().unwrap().0.unwrap();
        w.get_resource_mut::<AssetEditorCommands>()
            .unwrap()
            .0
            .push_back(AssetEditorCommand::Open {
                asset: third,
                project_generation: 1,
            });
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
        assert!(!finish_asset_request(&mut w, b, 1, Ok(())));
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
            w.get_resource_mut::<AssetEditorCommands>()
                .unwrap()
                .0
                .push_back(AssetEditorCommand::Open {
                    asset,
                    project_generation: 1,
                });
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
