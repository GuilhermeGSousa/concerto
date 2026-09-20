//! Lifecycle and registration for asset editors.
//!
//! Asset editors are deliberately kept separate from property editors.  An
//! editor is a single, type-owned document; opening another asset of the same
//! type replaces the document's contents instead of creating another world.

use anyhow::{bail, Result};
use app::App;
use ecs::{
    command::{CommandQueue, EntityCommandQueue},
    resource::{Res, ResMut},
    Component, Entity, Query, Resource,
};
use essential::assets::Asset;
use std::collections::{HashMap, HashSet, VecDeque};

use crate::project::AssetEntry;

/// Installs an editor's components when its document is created.
///
/// Asset loading and UI interaction belong in ordinary ECS systems. Query the
/// document's `pending` request and your own components there, and complete it
/// with [`finish_asset_request`]. Query [`ActiveEditor`] for tab activation.
/// Mark detached UI/preview roots with [`EditorOwned`] for automatic cleanup.
/// Contextual [`EditorHosts`] are supplied by the workspace before UI systems run.
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

/// Complete a request through a scoped document borrow. Pass the current
/// `ProjectState::generation`, not the generation captured by a worker.
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

/// Process tab commands using declared ECS access. Asset-specific work belongs
/// to the editor's own systems, which observe the resulting pending request.
pub fn process_editor_commands(
    mut requests: ResMut<AssetEditorCommands>,
    registry: Res<AssetEditorRegistry>,
    project: Res<crate::project::ProjectState>,
    mut active: ResMut<ActiveEditor>,
    documents: Query<(Entity, &EditorDocument)>,
    owned: Query<(Entity, &EditorOwned)>,
    mut commands: CommandQueue,
) {
    if requests.0.is_empty() {
        return;
    }
    // Batch-local staging makes deferred spawns visible to subsequent commands
    // in this batch (including open/open and open/close-all). ECS remains the
    // persistent owner; only changed documents are written back.
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
    use ecs::{IntoSystem, System, World};
    fn process_editor_commands(world: &mut World) {
        let mut system = super::process_editor_commands.into_system();
        system.initialize(world);
        system.run_and_apply(world);
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
    use essential::assets::AssetId;

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
            provenance: essential::assets::content::ImportProvenance {
                source: path.into(),
                sub_asset: String::new(),
            },
        }
    }
    fn world() -> World {
        let mut w = World::new();
        w.insert_resource(AssetEditorCommands::default());
        w.insert_resource(ActiveEditor::default());
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
            use ecs::query::change_detection::DetectChanges;
            assert!(!active.has_changed());
        })
        .into_system();
        check.initialize(&mut w);
        check.run_and_apply(&mut w);
    }

    #[test]
    fn lifecycle_declares_scoped_access_and_batches_deferred_spawns() {
        let system = super::process_editor_commands.into_system();
        let mut meta = ecs::system::meta::SystemMetadata::default();
        let mut access = ecs::system::access::SystemAccess::default();
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
        assert!(w
            .get_component_for_entity::<EditorDocument>(e)
            .unwrap()
            .pending
            .is_some());
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
        assert!(w
            .get_resource_mut::<AssetEditorRegistry>()
            .unwrap()
            .register::<TestAsset>(Fake)
            .is_err());
    }
}
