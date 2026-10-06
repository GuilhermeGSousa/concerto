use std::{
    any::{Any, TypeId},
    collections::HashMap,
    marker::PhantomData,
    sync::Arc,
};

use concerto_app::App;
use concerto_ecs::{
    Component, Entity, ResMut, Resource, World, command::CommandQueue, component::Tick,
};
use concerto_ecs::{
    Res,
    entity::EntityStructuralVersion,
    system::{
        access::SystemAccess,
        input::{ComponentMetadata, ReadOnlySystemInput, SystemInput},
        meta::SystemMetadata,
    },
    world::UnsafeWorldCell,
};
use concerto_editable::{Editable, PropertyPath, PropertyVisitor, with_property_mut};
use concerto_ui::theme::UITheme;

use super::rows::{
    EditError, Property, PropertyCommit, PropertyCommits, PropertyEditor, PropertyRowValue,
};

#[derive(Clone, Copy)]
pub(crate) struct EditableComponent {
    pub name: &'static str,
    pub collect: fn(&InspectorRegistry, &ComponentMetadata, Entity, Tick) -> Option<Vec<Property>>,
    pub apply:
        fn(&mut World, Entity, &PropertyPath, &dyn ErasedEditor, &dyn Any) -> Result<(), EditError>,
    pub insert: Option<fn(&mut World, Entity)>,
    pub remove: fn(&mut World, Entity),
}

pub(crate) trait ErasedEditor: Send + Sync {
    fn snapshot(&self, value: &dyn Editable) -> Result<PropertyRowValue, EditError>;
    fn build(
        &self,
        cmd: &mut CommandQueue,
        row: Entity,
        snapshot: &PropertyRowValue,
        theme: &UITheme,
    ) -> Result<(), EditError>;
    fn apply(&self, value: &mut dyn Editable, edit: &dyn Any) -> Result<(), EditError>;
}

struct Adapter<T, E> {
    editor: E,
    marker: PhantomData<fn() -> T>,
}

impl<T: Editable, E: PropertyEditor<T>> ErasedEditor for Adapter<T, E> {
    fn snapshot(&self, value: &dyn Editable) -> Result<PropertyRowValue, EditError> {
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
    fn apply(&self, value: &mut dyn Editable, edit: &dyn Any) -> Result<(), EditError> {
        let value = (value as &mut dyn Any)
            .downcast_mut::<T>()
            .ok_or(EditError::TypeMismatch)?;
        let edit = edit
            .downcast_ref::<E::Edit>()
            .ok_or(EditError::TypeMismatch)?;
        self.editor.apply(value, edit)
    }
}

pub(crate) struct RegisteredEditor {
    pub editor_type: TypeId,
    pub adapter: Arc<dyn ErasedEditor>,
}

/// Components and typed editors available to the inspector.
#[derive(Resource)]
pub struct InspectorRegistry {
    components: HashMap<TypeId, EditableComponent>,
    editors: HashMap<TypeId, RegisteredEditor>,
}

impl Default for InspectorRegistry {
    fn default() -> Self {
        let mut registry = Self {
            components: HashMap::new(),
            editors: HashMap::new(),
        };
        super::numeric::register_defaults(&mut registry);
        registry
    }
}

impl InspectorRegistry {
    pub fn register_component<T: Component + Editable + Default>(&mut self) {
        self.register::<T>(Some(|world, entity| world.insert(T::default(), entity)));
    }

    /// Registers a component the inspector edits and removes but never offers to add.
    pub fn register_inspectable<T: Component + Editable>(&mut self) {
        self.register::<T>(None);
    }

    fn register<T: Component + Editable>(&mut self, insert: Option<fn(&mut World, Entity)>) {
        let path = std::any::type_name::<T>();
        self.components.insert(
            TypeId::of::<T>(),
            EditableComponent {
                name: path.rsplit("::").next().unwrap_or(path),
                collect: collect_typed::<T>,
                apply: apply_typed::<T>,
                insert,
                remove: |world, entity| world.remove_component::<T>(entity),
            },
        );
    }

    pub fn register_property_editor<T: Editable, E: PropertyEditor<T>>(&mut self, editor: E) {
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

    pub(crate) fn component(&self, id: TypeId) -> Option<EditableComponent> {
        self.components.get(&id).copied()
    }
    pub(crate) fn editor(&self, id: TypeId) -> Option<&RegisteredEditor> {
        self.editors.get(&id)
    }

    /// Collect fresh owned snapshots, preferring an editor for each node over its children.
    pub fn collect(&self, root: &dyn Editable) -> Vec<Property> {
        let mut collector = Collect {
            registry: self,
            path: Vec::new(),
            properties: Vec::new(),
        };
        collector.node(root);
        collector.properties
    }

    /// Take fresh snapshots of a registered component, or `None` if it is absent or unregistered.
    pub fn collect_component(
        &self,
        world: &World,
        entity: Entity,
        component: TypeId,
    ) -> Option<Vec<Property>> {
        (self.component(component)?.collect)(
            self,
            &ComponentMetadata::new(world),
            entity,
            world
                .resource_changed_tick::<InspectorRegistry>()
                .unwrap_or_default(),
        )
    }
}

struct Collect<'a> {
    registry: &'a InspectorRegistry,
    path: Vec<&'static str>,
    properties: Vec<Property>,
}

impl Collect<'_> {
    fn node(&mut self, value: &dyn Editable) {
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
    fn field(&mut self, name: &'static str, value: &dyn Editable) {
        self.path.push(name);
        self.node(value);
        self.path.pop();
    }
}

fn collect_typed<T: Component + Editable>(
    registry: &InspectorRegistry,
    components: &ComponentMetadata,
    entity: Entity,
    registry_tick: Tick,
) -> Option<Vec<Property>> {
    components.get::<T>(entity).map(|value| {
        let mut properties = registry.collect(value);
        for property in &mut properties {
            property.registry_tick = registry_tick;
        }
        properties
    })
}

pub(super) struct InspectionSource<'w> {
    components: ComponentMetadata<'w>,
    registry: Res<'w, InspectorRegistry>,
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
    pub fn registry_tick(&self) -> Tick {
        self.registry.changed_tick()
    }
    pub fn has_component_changed_since(&self, entity: Entity, id: TypeId, tick: Tick) -> bool {
        self.components
            .has_component_changed_since(entity, id, tick)
    }
    pub fn is_registered(&self, id: TypeId) -> bool {
        self.registry.component(id).is_some()
    }
    /// Addable components `entity` does not carry yet, with how many are addable in total.
    pub fn addable_components(&self, entity: Entity) -> (Vec<(TypeId, EditableComponent)>, usize) {
        let present = self.components.component_ids(entity);
        let addable = || {
            self.registry
                .components
                .iter()
                .filter(|(_, component)| component.insert.is_some())
        };
        let missing = addable()
            .filter(|(id, _)| !present.contains(id))
            .map(|(&id, &component)| (id, component))
            .collect();
        (missing, addable().count())
    }
    pub fn visible_components(&self, entity: Entity) -> Vec<(TypeId, &'static str)> {
        self.components
            .component_ids(entity)
            .iter()
            .filter_map(|&id| {
                let name = self
                    .registry
                    .component(id)
                    .map(|c| c.name)
                    .or_else(|| self.components.type_info(id).map(|info| info.short()))?;
                Some((id, name))
            })
            .collect()
    }
    pub fn collect_component(&self, entity: Entity, id: TypeId) -> Option<Vec<Property>> {
        (self.registry.component(id)?.collect)(
            &self.registry,
            &self.components,
            entity,
            self.registry_tick(),
        )
    }
}

impl SystemInput for InspectionSource<'_> {
    type State = ();
    type Data<'world, 'state> = InspectionSource<'world>;
    fn init_state(_: &mut World) {}
    fn get_data<'world, 'state>(
        state: &'state mut (),
        world: UnsafeWorldCell<'world>,
    ) -> Self::Data<'world, 'state> {
        InspectionSource {
            components: ComponentMetadata::get_data(state, world),
            registry: Res::new(world),
        }
    }
    fn fill_access(meta: &mut SystemMetadata, access: &mut SystemAccess) {
        ComponentMetadata::fill_access(meta, access);
        Res::<InspectorRegistry>::fill_access(meta, access);
    }
}
impl ReadOnlySystemInput for InspectionSource<'_> {}

fn apply_typed<T: Component + Editable>(
    world: &mut World,
    entity: Entity,
    path: &PropertyPath,
    editor: &dyn ErasedEditor,
    edit: &dyn Any,
) -> Result<(), EditError> {
    let value = world
        .get_component_for_entity_mut::<T>(entity)
        .ok_or(EditError::MissingTarget)?;
    let mut result = Err(EditError::NotFound);
    with_property_mut(value, path, &mut |value| {
        result = editor.apply(value, edit);
    })
    .map_err(|_| EditError::NotFound)?;
    result
}

pub trait EditableApp {
    fn register_editable<T: Component + Editable + Default>(&mut self) -> &mut Self;
    /// Registers a component the inspector edits and removes but never offers to add.
    fn register_inspectable<T: Component + Editable>(&mut self) -> &mut Self;
    fn register_property_editor<T: Editable, E: PropertyEditor<T>>(
        &mut self,
        editor: E,
    ) -> &mut Self;
}

impl EditableApp for App {
    fn register_editable<T: Component + Editable + Default>(&mut self) -> &mut Self {
        registry(self).register_component::<T>();
        self
    }
    fn register_inspectable<T: Component + Editable>(&mut self) -> &mut Self {
        registry(self).register_inspectable::<T>();
        self
    }
    fn register_property_editor<T: Editable, E: PropertyEditor<T>>(
        &mut self,
        editor: E,
    ) -> &mut Self {
        registry(self).register_property_editor::<T, E>(editor);
        self
    }
}

fn registry(app: &mut App) -> ResMut<'_, InspectorRegistry> {
    let world = app.main_mut().world_mut();
    assert!(
        world.get_resource::<InspectorRegistry>().is_some(),
        "InspectorPlugin must be registered first"
    );
    ResMut::new(world.as_unsafe_world_cell_mut())
}

/// Apply one captured edit to the live world.
pub fn apply_property_commit(world: &mut World, commit: PropertyCommit) -> Result<(), EditError> {
    let row = &commit.row;
    let registry = world
        .get_resource::<InspectorRegistry>()
        .ok_or(EditError::UnregisteredComponent)?;
    let component = registry
        .component(row.component)
        .ok_or(EditError::UnregisteredComponent)?;
    let editor = registry.editor(row.type_id).ok_or(EditError::StaleEditor)?;
    if row.editor_type != Some(editor.editor_type)
        || Some(row.registry_tick) != world.resource_changed_tick::<InspectorRegistry>()
    {
        return Err(EditError::StaleEditor);
    }
    let adapter = Arc::clone(&editor.adapter);
    (component.apply)(
        world,
        row.entity,
        &row.path,
        adapter.as_ref(),
        commit.edit.as_ref(),
    )
}

/// Drain queued edits without consulting current selection or UI entity lifetime.
pub fn apply_property_commits(world: &mut World) {
    let Some(commits) = world.get_resource_mut::<PropertyCommits>() else {
        return;
    };
    for commit in std::mem::take(&mut commits.0) {
        let entity = commit.row.entity;
        match apply_property_commit(world, commit) {
            Ok(()) => crate::asset_editor::mark_entity_edited(world, entity),
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
    use concerto_foundation::transform::Transform;
    use glam::Vec3;

    fn world() -> (World, Entity) {
        let mut registry = InspectorRegistry::default();
        registry.register_component::<Transform>();
        let mut world = World::default();
        world.insert_resource(registry);
        world.insert_resource(PropertyCommits::default());
        let entity = world.spawn(Transform::IDENTITY);
        world.tick();
        (world, entity)
    }

    fn row(world: &World, entity: Entity) -> PropertyRow {
        world
            .get_resource::<InspectorRegistry>()
            .unwrap()
            .collect_component(world, entity, TypeId::of::<Transform>())
            .unwrap()[0]
            .row(entity, TypeId::of::<Transform>())
    }

    #[test]
    fn queued_slot_edits_compose_against_the_live_vector() {
        let (mut world, entity) = world();
        let row = row(&world, entity);
        let queue = world.get_resource_mut::<PropertyCommits>().unwrap();
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
        world
            .get_component_for_entity_mut::<Transform>(entity)
            .unwrap()
            .translation
            .z = 6.0;
        apply_property_commits(&mut world);
        assert_eq!(
            world
                .get_component_for_entity::<Transform>(entity)
                .unwrap()
                .translation,
            Vec3::new(4.0, 5.0, 6.0)
        );
        assert!(
            world
                .get_resource::<PropertyCommits>()
                .unwrap()
                .0
                .is_empty()
        );
    }

    #[test]
    fn an_applied_commit_marks_the_owning_document_and_a_dropped_one_does_not() {
        use crate::asset_editor::{EditorDocument, EditorOwned};
        let (mut world, entity) = world();
        let document = world.spawn(EditorDocument {
            asset_type: "Scene",
            title: String::new(),
            current: None,
            pending: None,
            project_generation: 0,
            request_generation: 0,
            order: 0,
            status: String::new(),
            revision: 0,
            saved_revision: 0,
        });
        world.insert(EditorOwned(document), entity);
        let row = row(&world, entity);
        let mut stale = row.clone();
        stale.editor_type = Some(TypeId::of::<u8>());
        let dirty = |world: &World| {
            world
                .get_component_for_entity::<EditorDocument>(document)
                .unwrap()
                .is_dirty()
        };

        world
            .get_resource_mut::<PropertyCommits>()
            .unwrap()
            .0
            .push(PropertyCommit {
                row: stale,
                edit: Box::new(NumericEdit {
                    slot: 0,
                    number: 1.0,
                }),
            });
        apply_property_commits(&mut world);
        assert!(!dirty(&world));

        world
            .get_resource_mut::<PropertyCommits>()
            .unwrap()
            .push::<Vec3, NumericFields>(
                &row,
                NumericEdit {
                    slot: 0,
                    number: 4.0,
                },
            )
            .unwrap();
        apply_property_commits(&mut world);
        assert!(dirty(&world));
    }

    #[test]
    fn mismatched_payloads_are_errors_and_never_reach_the_adapter() {
        let (mut world, entity) = world();
        let row = row(&world, entity);
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
            edit: Box::new("wrong payload"),
        };
        assert_eq!(
            apply_property_commit(&mut world, malformed),
            Err(EditError::TypeMismatch)
        );
        assert_eq!(
            world
                .get_component_for_entity::<Transform>(entity)
                .unwrap()
                .translation,
            Vec3::ZERO
        );
    }

    #[test]
    fn replacing_the_registry_also_invalidates_old_edits() {
        let (mut world, entity) = world();
        let edit = PropertyCommit::new::<Vec3, NumericFields>(
            &row(&world, entity),
            NumericEdit {
                slot: 0,
                number: 9.0,
            },
        )
        .unwrap();
        let mut registry = InspectorRegistry::default();
        registry.register_component::<Transform>();
        world.insert_resource(registry);
        assert_eq!(
            apply_property_commit(&mut world, edit),
            Err(EditError::StaleEditor)
        );
    }

    #[test]
    fn unregistered_components_return_an_error() {
        let (mut world, entity) = world();
        let edit = PropertyCommit::new::<Vec3, NumericFields>(
            &row(&world, entity),
            NumericEdit {
                slot: 0,
                number: 9.0,
            },
        )
        .unwrap();
        world.insert_resource(InspectorRegistry::default());
        assert_eq!(
            apply_property_commit(&mut world, edit),
            Err(EditError::UnregisteredComponent)
        );
    }

    #[test]
    fn commits_are_visible_to_transform_propagation_in_the_same_tick() {
        use concerto_ecs::{IntoSystem, System};
        use concerto_foundation::transform::{GlobalTransform, systems::update_simple_entities};
        let mut registry = InspectorRegistry::default();
        registry.register_component::<Transform>();
        let mut world = World::default();
        world.register_component::<Transform>();
        world.insert_resource(registry);
        let entity = world.spawn(Transform::IDENTITY);
        world.tick();
        let edit = PropertyCommit::new::<Vec3, NumericFields>(
            &row(&world, entity),
            NumericEdit {
                slot: 0,
                number: 3.0,
            },
        )
        .unwrap();
        apply_property_commit(&mut world, edit).unwrap();
        let mut propagation = update_simple_entities.into_system();
        propagation.initialize(&mut world);
        propagation.run_and_apply((), &mut world);
        assert_eq!(
            world
                .get_component_for_entity::<GlobalTransform>(entity)
                .unwrap()
                .translation(),
            Vec3::new(3.0, 0.0, 0.0)
        );
    }
}
