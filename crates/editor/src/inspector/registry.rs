use std::{
    any::{Any, TypeId},
    collections::HashMap,
    marker::PhantomData,
    sync::Arc,
};

use concerto_app::{
    App,
    reflect::{ReflectComponent, ReflectComponents},
};
use concerto_ecs::{Entity, ResMut, Resource, World, command::CommandQueue, component::Tick};
use concerto_ecs::{
    Res,
    entity::EntityStructuralVersion,
    system::{
        access::SystemAccess,
        input::{ComponentMetadata, ReadOnlySystemInput, SystemInput, ValidationError},
        meta::SystemMetadata,
    },
    world::UnsafeWorldCell,
};
use concerto_reflect::{PropertyPath, PropertyVisitor, Reflect};
use concerto_ui::theme::UITheme;

use super::rows::{
    EditError, Property, PropertyCommit, PropertyCommits, PropertyEditor, PropertyRowValue,
};
use crate::hosted::EditWorldSignals;
use crate::tools::edits::CommitProperty;

/// A registered property editor with its value and edit types erased.
pub trait ErasedEditor: Send + Sync {
    fn snapshot(&self, value: &dyn Reflect) -> Result<PropertyRowValue, EditError>;
    fn build(
        &self,
        cmd: &mut CommandQueue,
        row: Entity,
        snapshot: &PropertyRowValue,
        theme: &UITheme,
    ) -> Result<(), EditError>;
    fn apply(&self, value: &mut dyn Reflect, edit: &dyn Any) -> Result<(), EditError>;
    fn follows_snapshot(&self) -> bool;
}

struct Adapter<T, E> {
    editor: E,
    marker: PhantomData<fn() -> T>,
}

impl<T: Reflect, E: PropertyEditor<T>> ErasedEditor for Adapter<T, E> {
    fn snapshot(&self, value: &dyn Reflect) -> Result<PropertyRowValue, EditError> {
        let value = (value as &dyn Any)
            .downcast_ref::<T>()
            .ok_or(EditError::TypeMismatch)?;
        Ok(PropertyRowValue::new::<T, E>(self.editor.snapshot(value)))
    }
    fn build(
        &self,
        cmd: &mut CommandQueue,
        row: Entity,
        snapshot: &PropertyRowValue,
        theme: &UITheme,
    ) -> Result<(), EditError> {
        let snapshot = snapshot.snapshot::<T, E>().ok_or(EditError::TypeMismatch)?;
        self.editor.build(cmd, row, snapshot, theme);
        Ok(())
    }
    fn apply(&self, value: &mut dyn Reflect, edit: &dyn Any) -> Result<(), EditError> {
        let value = (value as &mut dyn Any)
            .downcast_mut::<T>()
            .ok_or(EditError::TypeMismatch)?;
        let edit = edit
            .downcast_ref::<E::Edit>()
            .ok_or(EditError::TypeMismatch)?;
        self.editor.apply(value, edit)
    }
    fn follows_snapshot(&self) -> bool {
        self.editor.follows_snapshot()
    }
}

pub(crate) struct RegisteredEditor {
    pub editor_type: TypeId,
    pub adapter: Arc<dyn ErasedEditor>,
}

/// Typed property editors available to the inspector.
#[derive(Resource)]
pub struct InspectorRegistry {
    editors: HashMap<TypeId, RegisteredEditor>,
}

impl Default for InspectorRegistry {
    fn default() -> Self {
        let mut registry = Self {
            editors: HashMap::new(),
        };
        super::numeric::register_defaults(&mut registry);
        super::boolean::register_defaults(&mut registry);
        registry
    }
}

impl InspectorRegistry {
    pub fn register_property_editor<T: Reflect, E: PropertyEditor<T>>(&mut self, editor: E) {
        self.editors.insert(
            TypeId::of::<T>(),
            RegisteredEditor {
                editor_type: TypeId::of::<E>(),
                adapter: Arc::new(Adapter::<T, E> {
                    editor,
                    marker: PhantomData,
                }),
            },
        );
    }

    pub(crate) fn editor(&self, id: TypeId) -> Option<&RegisteredEditor> {
        self.editors.get(&id)
    }

    /// Collect fresh owned snapshots, preferring an editor for each node over its children.
    pub fn collect(&self, root: &dyn Reflect) -> Vec<Property> {
        let mut collector = Collect {
            registry: self,
            path: Vec::new(),
            properties: Vec::new(),
        };
        collector.node(root);
        collector.properties
    }

    #[cfg(test)]
    pub(crate) fn collect_component(
        &self,
        registry_tick: Tick,
        scene: &World,
        entity: Entity,
        component: TypeId,
    ) -> Option<Vec<Property>> {
        let entry = scene.get_resource::<ReflectComponents>()?.get(component)?;
        let value = (entry.read)(&ComponentMetadata::new(scene), entity)?;
        Some(self.collect_at(registry_tick, value))
    }

    fn collect_at(&self, registry_tick: Tick, root: &dyn Reflect) -> Vec<Property> {
        let mut properties = self.collect(root);
        for property in &mut properties {
            property.registry_tick = registry_tick;
        }
        properties
    }
}

struct Collect<'a> {
    registry: &'a InspectorRegistry,
    path: Vec<&'static str>,
    properties: Vec<Property>,
}

impl Collect<'_> {
    fn node(&mut self, value: &dyn Reflect) {
        let type_id = (value as &dyn Any).type_id();
        if let Some(editor) = self.registry.editor(type_id) {
            match editor.adapter.snapshot(value) {
                Ok(snapshot) => self.properties.push(Property {
                    path: PropertyPath::new(self.path.iter().copied()),
                    type_id,
                    value: snapshot,
                    editor_type: Some(editor.editor_type),
                    registry_tick: Tick::default(),
                }),
                Err(error) => log::warn!("Unable to snapshot {:?}: {error}", self.path),
            }
            return;
        }
        let count = self.properties.len();
        value.visit(self);
        if count == self.properties.len() {
            self.properties.push(Property {
                path: PropertyPath::new(self.path.iter().copied()),
                type_id,
                value: PropertyRowValue::default(),
                editor_type: None,
                registry_tick: Tick::default(),
            });
        }
    }
}

impl PropertyVisitor for Collect<'_> {
    fn field(&mut self, name: &'static str, value: &dyn Reflect) {
        self.path.push(name);
        self.node(value);
        self.path.pop();
    }
}

pub(super) struct InspectionSource<'w> {
    components: ComponentMetadata<'w>,
    table: Res<'w, ReflectComponents>,
}

impl InspectionSource<'_> {
    pub fn structural_version(&self, entity: Entity) -> Option<EntityStructuralVersion> {
        self.components.structural_version(entity)
    }
    pub fn entity_is_valid(&self, entity: Entity) -> bool {
        self.components.entity_is_valid(entity)
    }
    pub fn current_tick(&self) -> Tick {
        self.components.current_tick()
    }
    pub fn has_component_changed_since(&self, entity: Entity, id: TypeId, tick: Tick) -> bool {
        self.components
            .has_component_changed_since(entity, id, tick)
    }
    pub fn is_registered(&self, id: TypeId) -> bool {
        self.table.get(id).is_some()
    }
    pub fn follows_snapshot(&self, registry: &InspectorRegistry, property: &Property) -> bool {
        registry.editor(property.type_id).is_some_and(|editor| {
            Some(editor.editor_type) == property.editor_type && editor.adapter.follows_snapshot()
        })
    }
    /// Addable components `entity` does not carry yet, with how many are addable in total.
    pub fn addable_components(&self, entity: Entity) -> (Vec<(TypeId, ReflectComponent)>, usize) {
        let present = self.components.component_ids(entity);
        let addable = || {
            self.table
                .iter()
                .filter(|(_, component)| component.insert.is_some())
        };
        let missing = addable().filter(|(id, _)| !present.contains(id)).collect();
        (missing, addable().count())
    }
    pub fn visible_components(&self, entity: Entity) -> Vec<(TypeId, &'static str)> {
        self.components
            .component_ids(entity)
            .iter()
            .filter_map(|&id| {
                let name = self
                    .table
                    .get(id)
                    .map(|c| c.name)
                    .or_else(|| self.components.type_info(id).map(|info| info.short()))?;
                Some((id, name))
            })
            .collect()
    }
    pub fn collect_component(
        &self,
        registry: &InspectorRegistry,
        registry_tick: Tick,
        entity: Entity,
        id: TypeId,
    ) -> Option<Vec<Property>> {
        let value = (self.table.get(id)?.read)(&self.components, entity)?;
        Some(registry.collect_at(registry_tick, value))
    }
}

impl SystemInput for InspectionSource<'_> {
    type State = ();
    type Data<'world, 'state> = InspectionSource<'world>;
    fn init_state(_: &mut World) {}
    unsafe fn get_data<'world, 'state>(
        state: &'state mut (),
        world: UnsafeWorldCell<'world>,
    ) -> Result<Self::Data<'world, 'state>, ValidationError> {
        // SAFETY: this input declares exactly the access of its two parts.
        Ok(InspectionSource {
            components: unsafe { ComponentMetadata::get_data(state, world) }?,
            table: unsafe { Res::new(world) }?,
        })
    }
    fn fill_access(meta: &mut SystemMetadata, access: &mut SystemAccess) {
        ComponentMetadata::fill_access(meta, access);
        Res::<ReflectComponents>::fill_access(meta, access);
    }
}
impl ReadOnlySystemInput for InspectionSource<'_> {}

pub trait EditableApp {
    fn register_property_editor<T: Reflect, E: PropertyEditor<T>>(
        &mut self,
        editor: E,
    ) -> &mut Self;
}

impl EditableApp for App {
    fn register_property_editor<T: Reflect, E: PropertyEditor<T>>(
        &mut self,
        editor: E,
    ) -> &mut Self {
        registry(self).register_property_editor::<T, E>(editor);
        self
    }
}

fn registry(app: &mut App) -> ResMut<'_, InspectorRegistry> {
    let world = app.main_mut().world_mut();
    // SAFETY: the exclusive borrow of the world covers any access.
    unsafe { ResMut::new(world.as_unsafe_world_cell_mut()) }
        .expect("InspectorPlugin must be registered first")
}

fn checked(
    registry: &InspectorRegistry,
    changed: Tick,
    commit: PropertyCommit,
) -> Result<CommitProperty, EditError> {
    let row = commit.row;
    let editor = registry
        .editor(row.type_id)
        .filter(|editor| {
            row.editor_type == Some(editor.editor_type) && row.registry_tick == changed
        })
        .ok_or(EditError::StaleEditor)?;
    Ok(CommitProperty {
        editor: Arc::clone(&editor.adapter),
        row,
        edit: commit.edit,
    })
}

pub(super) fn send_property_commits(
    mut commits: ResMut<PropertyCommits>,
    registry: Res<InspectorRegistry>,
    mut scene: EditWorldSignals,
) {
    if commits.0.is_empty() {
        return;
    }
    for commit in std::mem::take(&mut commits.0) {
        match checked(&registry, registry.changed_tick(), commit) {
            Ok(commit) => scene.trigger(commit),
            Err(error) => log::warn!("Property commit dropped: {error}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        PropertyRow,
        numeric::{NumericEdit, NumericFields},
    };
    use super::*;
    use crate::{hosted::testing::SceneFixture, tools::scene::SceneEdited};
    use concerto_ecs::{IntoSystem, System, events::event_channel::EventChannel};
    use concerto_foundation::transform::Transform;
    use glam::Vec3;

    struct Sending {
        f: SceneFixture,
        document: Entity,
        entity: Entity,
    }

    fn sending() -> Sending {
        let mut f = SceneFixture::new();
        f.world.insert_resource(InspectorRegistry::default());
        let document = f.open();
        let entity = f.scene_entity(document, "first");
        f.world.tick();
        Sending {
            f,
            document,
            entity,
        }
    }

    impl Sending {
        fn send(&mut self) {
            let mut system = send_property_commits.into_system();
            system.initialize(&mut self.f.world);
            system.run_and_apply((), &mut self.f.world);
        }
        fn edits(&self) -> usize {
            self.f
                .scene_world(self.document)
                .get_resource::<EventChannel<SceneEdited>>()
                .unwrap()
                .len()
        }
        fn row(&self) -> PropertyRow {
            let registry = self.f.world.get_resource::<InspectorRegistry>().unwrap();
            let changed = self
                .f
                .world
                .resource_changed_tick::<InspectorRegistry>()
                .unwrap();
            registry
                .collect_component(
                    changed,
                    self.f.scene_world(self.document),
                    self.entity,
                    TypeId::of::<Transform>(),
                )
                .unwrap()[0]
                .row(self.entity, TypeId::of::<Transform>())
        }
        fn translation(&self) -> Vec3 {
            self.f
                .scene_world(self.document)
                .get_component_for_entity::<Transform>(self.entity)
                .unwrap()
                .translation
        }
        fn commits(&mut self) -> &mut PropertyCommits {
            self.f.world.get_resource_mut::<PropertyCommits>().unwrap()
        }
    }

    fn table() -> ReflectComponents {
        let mut table = ReflectComponents::default();
        table.register::<Transform>();
        table
    }

    fn scene() -> (World, Entity) {
        let mut world = World::default();
        world.register_component::<Transform>();
        world.insert_resource(table());
        let entity = world.spawn(Transform::IDENTITY);
        world.tick();
        (world, entity)
    }

    fn row_in(registry: &InspectorRegistry, scene: &World, entity: Entity) -> PropertyRow {
        registry
            .collect_component(Tick::default(), scene, entity, TypeId::of::<Transform>())
            .unwrap()[0]
            .row(entity, TypeId::of::<Transform>())
    }

    fn refused(
        registry: &InspectorRegistry,
        changed: Tick,
        commit: PropertyCommit,
    ) -> Option<EditError> {
        checked(registry, changed, commit).err()
    }

    #[test]
    fn queued_slot_edits_compose_against_the_live_vector() {
        let mut s = sending();
        let row = s.row();
        let queue = s.commits();
        queue
            .push::<Vec3, NumericFields>(
                &row,
                NumericEdit {
                    slot: 0,
                    number: 4.0,
                },
            )
            .unwrap();
        queue
            .push::<Vec3, NumericFields>(
                &row,
                NumericEdit {
                    slot: 1,
                    number: 5.0,
                },
            )
            .unwrap();
        let entity = s.entity;
        s.f.scene_world_mut(s.document)
            .get_component_for_entity_mut::<Transform>(entity)
            .unwrap()
            .translation
            .z = 6.0;
        s.send();
        assert_eq!(s.translation(), Vec3::new(4.0, 5.0, 6.0));
        assert!(s.commits().0.is_empty());
    }

    #[test]
    fn an_applied_commit_writes_scene_edited_and_a_stale_one_is_never_sent() {
        let mut s = sending();
        let row = s.row();
        let mut stale = row.clone();
        stale.editor_type = Some(TypeId::of::<u8>());

        s.commits().0.push(PropertyCommit {
            row: stale,
            edit: Arc::new(NumericEdit {
                slot: 0,
                number: 1.0,
            }),
        });
        s.send();
        assert_eq!(s.edits(), 0);
        assert_eq!(s.translation(), Vec3::ZERO);
        assert!(!s.f.unsaved(s.document));

        s.commits()
            .push::<Vec3, NumericFields>(
                &row,
                NumericEdit {
                    slot: 0,
                    number: 4.0,
                },
            )
            .unwrap();
        s.send();
        assert_eq!(s.edits(), 1);
        assert_eq!(s.translation().x, 4.0);
        assert!(s.f.unsaved(s.document));
    }

    #[test]
    fn a_commit_whose_row_was_collected_before_the_registry_changed_is_never_sent() {
        let mut s = sending();
        let row = s.row();
        s.f.world.tick();
        // SAFETY: the exclusive borrow of the world covers any access.
        unsafe { ResMut::<InspectorRegistry>::new(s.f.world.as_unsafe_world_cell_mut()) }
            .unwrap()
            .register_property_editor::<Vec3, _>(NumericFields);
        s.commits()
            .push::<Vec3, NumericFields>(
                &row,
                NumericEdit {
                    slot: 0,
                    number: 4.0,
                },
            )
            .unwrap();

        s.send();

        assert_eq!(s.edits(), 0);
        assert_eq!(s.translation(), Vec3::ZERO);
        assert!(s.commits().0.is_empty());
    }

    #[test]
    fn an_empty_queue_is_not_marked_changed() {
        let mut s = sending();
        let changed = s.f.world.resource_changed_tick::<PropertyCommits>();
        s.f.world.tick();

        s.send();

        assert_eq!(
            s.f.world.resource_changed_tick::<PropertyCommits>(),
            changed
        );
    }

    #[test]
    fn mismatched_payloads_are_errors_and_never_reach_the_adapter() {
        let (mut scene, entity) = scene();
        let registry = InspectorRegistry::default();
        let row = row_in(&registry, &scene, entity);
        assert!(matches!(
            PropertyCommit::new::<f32, NumericFields>(
                &row,
                NumericEdit {
                    slot: 0,
                    number: 9.0
                }
            ),
            Err(EditError::TypeMismatch)
        ));
        let malformed = PropertyCommit {
            row,
            edit: Arc::new("wrong payload"),
        };
        let commit = checked(&registry, Tick::default(), malformed)
            .expect("the row still names the registered editor");
        assert_eq!(commit.apply(&mut scene), Err(EditError::TypeMismatch));
        assert_eq!(
            scene
                .get_component_for_entity::<Transform>(entity)
                .unwrap()
                .translation,
            Vec3::ZERO
        );
    }

    #[test]
    fn replacing_the_registry_also_invalidates_old_edits() {
        let (scene, entity) = scene();
        let mut editors = World::default();
        editors.insert_resource(InspectorRegistry::default());
        editors.tick();
        let changed = |editors: &World| {
            editors
                .resource_changed_tick::<InspectorRegistry>()
                .unwrap()
        };
        let old = editors
            .get_resource::<InspectorRegistry>()
            .unwrap()
            .collect_component(changed(&editors), &scene, entity, TypeId::of::<Transform>())
            .unwrap()[0]
            .row(entity, TypeId::of::<Transform>());
        let edit = || {
            PropertyCommit::new::<Vec3, NumericFields>(
                &old,
                NumericEdit {
                    slot: 0,
                    number: 9.0,
                },
            )
            .unwrap()
        };
        let registry = editors.get_resource::<InspectorRegistry>().unwrap();
        assert_eq!(refused(registry, changed(&editors), edit()), None);

        editors.tick();
        editors.insert_resource(InspectorRegistry::default());

        let registry = editors.get_resource::<InspectorRegistry>().unwrap();
        assert_eq!(
            refused(registry, changed(&editors), edit()),
            Some(EditError::StaleEditor)
        );
    }

    #[test]
    fn unregistered_components_return_an_error() {
        let (mut scene, entity) = scene();
        let registry = InspectorRegistry::default();
        let edit = PropertyCommit::new::<Vec3, NumericFields>(
            &row_in(&registry, &scene, entity),
            NumericEdit {
                slot: 0,
                number: 9.0,
            },
        )
        .unwrap();
        scene.insert_resource(ReflectComponents::default());
        let commit = checked(&registry, Tick::default(), edit).unwrap();
        assert_eq!(
            commit.apply(&mut scene),
            Err(EditError::UnregisteredComponent)
        );
    }

    fn numeric(
        registry: &InspectorRegistry,
        row: PropertyRow,
        number: f64,
    ) -> crate::tools::edits::CommitProperty {
        let commit = PropertyCommit {
            row,
            edit: Arc::new(NumericEdit { slot: 0, number }),
        };
        checked(registry, Tick::default(), commit).unwrap()
    }

    #[test]
    fn a_commit_changes_only_the_entity_its_row_captured() {
        let (mut scene, a) = scene();
        let b = scene.spawn(Transform::IDENTITY);
        scene.tick();
        let registry = InspectorRegistry::default();
        let row = row_in(&registry, &scene, a);
        assert!(
            !scene.was_component_changed(a, TypeId::of::<Transform>()),
            "collecting a row does not mark the component changed"
        );

        numeric(&registry, row, 4.0).apply(&mut scene).unwrap();

        let x = |scene: &World, entity| {
            scene
                .get_component_for_entity::<Transform>(entity)
                .unwrap()
                .translation
                .x
        };
        assert_eq!((x(&scene, a), x(&scene, b)), (4.0, 0.0));
        assert!(scene.was_component_changed(a, TypeId::of::<Transform>()));
        assert!(!scene.was_component_changed(b, TypeId::of::<Transform>()));
    }

    #[test]
    fn a_commit_is_validated_before_it_changes_anything() {
        let (mut scene, entity) = scene();
        let registry = InspectorRegistry::default();
        let row = row_in(&registry, &scene, entity);

        assert_eq!(
            numeric(&registry, row, f64::NAN).apply(&mut scene),
            Err(EditError::Rejected)
        );

        assert_eq!(
            scene
                .get_component_for_entity::<Transform>(entity)
                .unwrap()
                .translation,
            Vec3::ZERO
        );
    }

    #[test]
    fn missing_targets_and_incompatible_paths_return_errors() {
        let (mut scene, a) = scene();
        let b = scene.spawn(Transform::IDENTITY);
        let registry = InspectorRegistry::default();
        let row = row_in(&registry, &scene, a);

        let mut wrong_type = row.clone();
        wrong_type.path = PropertyPath::new(["rotation"]);
        assert_eq!(
            numeric(&registry, wrong_type, 1.0).apply(&mut scene),
            Err(EditError::TypeMismatch)
        );
        let mut missing = row.clone();
        missing.path = PropertyPath::new(["absent"]);
        assert_eq!(
            numeric(&registry, missing, 1.0).apply(&mut scene),
            Err(EditError::NotFound)
        );
        scene.remove_component::<Transform>(a);
        assert_eq!(
            numeric(&registry, row, 1.0).apply(&mut scene),
            Err(EditError::MissingTarget)
        );
        let despawned = row_in(&registry, &scene, b);
        scene.despawn(b);
        assert_eq!(
            numeric(&registry, despawned, 1.0).apply(&mut scene),
            Err(EditError::MissingTarget)
        );
    }

    #[test]
    fn commits_are_visible_to_transform_propagation_in_the_same_tick() {
        use concerto_foundation::transform::{GlobalTransform, systems::update_simple_entities};
        let (mut scene, entity) = scene();
        let registry = InspectorRegistry::default();
        let edit = PropertyCommit::new::<Vec3, NumericFields>(
            &row_in(&registry, &scene, entity),
            NumericEdit {
                slot: 0,
                number: 3.0,
            },
        )
        .unwrap();
        checked(&registry, Tick::default(), edit)
            .unwrap()
            .apply(&mut scene)
            .unwrap();
        let mut propagation = update_simple_entities.into_system();
        propagation.initialize(&mut scene);
        propagation.run_and_apply((), &mut scene);
        assert_eq!(
            scene
                .get_component_for_entity::<GlobalTransform>(entity)
                .unwrap()
                .translation(),
            Vec3::new(3.0, 0.0, 0.0)
        );
    }
}
