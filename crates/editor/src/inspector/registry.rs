use std::{
    any::{Any, TypeId},
    collections::HashMap,
    marker::PhantomData,
    ops::Deref,
    sync::Arc,
};

use concerto_app::App;
use concerto_ecs::{
    Entity, ResMut, Resource, World,
    command::CommandQueue,
    component::{SystemTicks, Tick},
};
use concerto_ecs::{
    Res,
    entity::EntityStructuralVersion,
    reflect::{AppTypeRegistry, ReflectComponent},
    system::{
        access::SystemAccess,
        input::{ComponentMetadata, ReadOnlySystemInput, SystemInput, ValidationError},
        meta::SystemMetadata,
    },
    world::UnsafeWorldCell,
};
use concerto_foundation::assets::handle::ReflectAssetHandle;
use concerto_reflect::{
    PathSegment, PropertyPath, PropertyVisitor, Reflect, ReflectDefault, ReflectEnumInfo,
    ReflectList, ReflectOption, ReflectRef, TypeRegistry, with_property,
};
use concerto_ui::{node::UINode, theme::UITheme};

use super::InspectorData;
use super::list::{ListEdit, ListEditor};
use super::variants::{OptionToggle, VariantPicker};
use crate::asset_picker::AssetPicker;
use crate::hosted::{EditWorld, EditWorldSignals};
use crate::property::{
    ComponentTarget, EditContext, EditError, ErasedEditor, Property, PropertyAddress,
    PropertyCommit, PropertyCommits, PropertyEditor, PropertyKind, PropertyRow, PropertyRowValue,
    TargetWorld, standard_row_node,
};
use crate::tools::edits::CommitProperty;

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
    fn row_node(&self, snapshot: &PropertyRowValue, theme: &UITheme) -> Result<UINode, EditError> {
        let snapshot = snapshot.snapshot::<T, E>().ok_or(EditError::TypeMismatch)?;
        Ok(self.editor.row_node(snapshot, theme))
    }
    fn build(
        &self,
        cmd: &mut CommandQueue,
        row: Entity,
        snapshot: &PropertyRowValue,
        theme: &UITheme,
    ) -> Result<(), EditError> {
        let snapshot = snapshot.snapshot::<T, E>().ok_or(EditError::TypeMismatch)?;
        self.editor.build(&mut cmd.entity(row), snapshot, theme);
        Ok(())
    }
    fn apply(
        &self,
        value: &mut dyn Reflect,
        edit: &dyn Any,
        context: &EditContext,
    ) -> Result<(), EditError> {
        let value = (value as &mut dyn Any)
            .downcast_mut::<T>()
            .ok_or(EditError::TypeMismatch)?;
        let edit = edit
            .downcast_ref::<E::Edit>()
            .ok_or(EditError::TypeMismatch)?;
        self.editor.apply(value, edit, context)
    }
    fn edit_to(
        &self,
        value: &dyn Reflect,
    ) -> Result<Option<Arc<dyn Any + Send + Sync>>, EditError> {
        let value = (value as &dyn Any)
            .downcast_ref::<T>()
            .ok_or(EditError::TypeMismatch)?;
        Ok(self
            .editor
            .edit_to(value)
            .map(|edit| Arc::new(edit) as Arc<dyn Any + Send + Sync>))
    }
    fn follows_snapshot(&self) -> bool {
        self.editor.follows_snapshot()
    }
}

#[derive(Clone)]
pub(crate) struct RegisteredEditor {
    pub editor_type: TypeId,
    pub adapter: Arc<dyn ErasedEditor>,
    pub walks_children: bool,
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
        super::color::register_defaults(&mut registry);
        super::name_text::register_defaults(&mut registry);
        super::text_field::register_defaults(&mut registry);
        crate::entity_picker::register_defaults(&mut registry);
        crate::joint_picker::register_defaults(&mut registry);
        registry
    }
}

impl InspectorRegistry {
    pub fn register_property_editor<T: Reflect, E: PropertyEditor<T>>(&mut self) {
        self.editors.insert(
            TypeId::of::<T>(),
            RegisteredEditor {
                editor_type: TypeId::of::<E>(),
                adapter: Arc::new(Adapter::<T, E> {
                    editor: E::default(),
                    marker: PhantomData,
                }),
                walks_children: false,
            },
        );
    }

    /// The editor registered for `id`, or else the one its type data in `types` calls for: the asset picker, an option toggle, a list editor or a variant picker.
    pub(crate) fn editor(
        &self,
        id: TypeId,
        types: Option<&TypeRegistry>,
    ) -> Option<RegisteredEditor> {
        if let Some(editor) = self.editors.get(&id) {
            return Some(editor.clone());
        }
        let types = types?;
        if let Some(handle) = types.get_type_data::<ReflectAssetHandle>(id) {
            return Some(RegisteredEditor {
                editor_type: TypeId::of::<AssetPicker>(),
                adapter: Arc::new(AssetPicker::new(*handle)),
                walks_children: false,
            });
        }
        let (editor_type, adapter): (TypeId, Arc<dyn ErasedEditor>) =
            if types.get_type_data::<ReflectOption>(id).is_some() {
                (TypeId::of::<OptionToggle>(), Arc::new(OptionToggle))
            } else if types.get_type_data::<ReflectList>(id).is_some() {
                (TypeId::of::<ListEditor>(), Arc::new(ListEditor))
            } else {
                let info = types.get_type_data::<ReflectEnumInfo>(id)?;
                (
                    TypeId::of::<VariantPicker>(),
                    Arc::new(VariantPicker::new(info.variants)),
                )
            };
        Some(RegisteredEditor {
            editor_type,
            adapter,
            walks_children: true,
        })
    }

    pub(crate) fn follows_snapshot(&self, types: &TypeRegistry, row: &PropertyRow) -> bool {
        self.editor(row.address.type_id, Some(types))
            .is_some_and(|editor| {
                editor.editor_type == row.editor_type && editor.adapter.follows_snapshot()
            })
    }

    /// The layout node for `property`'s row, the standard one when no editor claims it.
    pub(crate) fn row_node(
        &self,
        types: &TypeRegistry,
        property: &Property,
        theme: &UITheme,
    ) -> UINode {
        property
            .value()
            .zip(self.editor(property.type_id, Some(types)))
            .and_then(|(value, editor)| editor.adapter.row_node(value, theme).ok())
            .unwrap_or_else(|| standard_row_node(theme))
    }

    /// Collect fresh owned snapshots, preferring an editor for each node over its children unless that editor walks them.
    pub fn collect(&self, root: &dyn Reflect, types: &TypeRegistry) -> Vec<Property> {
        let mut collector = Collect {
            registry: self,
            types,
            path: Vec::new(),
            properties: Vec::new(),
        };
        collector.node(root);
        collector.properties
    }
}

struct Collect<'a> {
    registry: &'a InspectorRegistry,
    types: &'a TypeRegistry,
    path: Vec<PathSegment>,
    properties: Vec<Property>,
}

impl Collect<'_> {
    fn property(&self, type_id: TypeId, kind: PropertyKind) -> Property {
        Property {
            path: PropertyPath::new(self.path.iter().copied()),
            type_id,
            kind,
        }
    }

    fn node(&mut self, value: &dyn Reflect) {
        let type_id = (value as &dyn Any).type_id();
        if let Some(editor) = self.registry.editor(type_id, Some(self.types)) {
            match editor.adapter.snapshot(value) {
                Ok(snapshot) => self
                    .properties
                    .push(self.property(type_id, PropertyKind::Value(snapshot))),
                Err(error) => log::warn!("Unable to snapshot {:?}: {error}", self.path),
            }
            if editor.walks_children {
                value.visit(self);
            }
            return;
        }
        let count = self.properties.len();
        value.visit(self);
        let empty =
            matches!(value.reflect_ref(), ReflectRef::Struct(fields) if fields.field_len() == 0);
        if count == self.properties.len() && !empty {
            self.properties
                .push(self.property(type_id, PropertyKind::Unsupported));
        }
    }
}

impl PropertyVisitor for Collect<'_> {
    fn field(&mut self, name: &'static str, value: &dyn Reflect) {
        self.path.push(PathSegment::Field(name));
        self.node(value);
        self.path.pop();
    }

    fn element(&mut self, index: usize, value: &dyn Reflect) {
        self.path.push(PathSegment::Index(index));
        let first = self.properties.len();
        self.node(value);
        let has_own_row = self
            .properties
            .get(first)
            .is_some_and(|property| property.path.depth() == self.path.len());
        if !has_own_row {
            let heading = self.property((value as &dyn Any).type_id(), PropertyKind::Heading);
            self.properties.insert(first, heading);
        }
        self.path.pop();
    }
}

pub(crate) struct InspectionSource<'w> {
    components: ComponentMetadata<'w>,
    table: Res<'w, AppTypeRegistry>,
}

impl InspectionSource<'_> {
    pub fn structural_version(&self, entity: Entity) -> Option<EntityStructuralVersion> {
        self.components.structural_version(entity)
    }
    pub fn entity_is_valid(&self, entity: Entity) -> bool {
        self.components.entity_is_valid(entity)
    }
    pub fn is_component_changed(&self, entity: Entity, id: TypeId) -> bool {
        self.components.is_component_changed(entity, id)
    }
    /// Whether `entity` has component `id`.
    pub fn carries(&self, entity: Entity, id: TypeId) -> bool {
        self.components.component_ids(entity).contains(&id)
    }
    pub fn is_registered(&self, id: TypeId) -> bool {
        self.table.get_type_data::<ReflectComponent>(id).is_some()
    }
    /// Addable components `entity` does not carry yet, with how many are addable in total.
    pub fn addable_components(&self, entity: Entity) -> (Vec<(TypeId, &'static str)>, usize) {
        let present = self.components.component_ids(entity);
        let table = self.table.read();
        let addable = || {
            table
                .iter()
                .filter(|registration| {
                    registration.data::<ReflectComponent>().is_some()
                        && registration.data::<ReflectDefault>().is_some()
                })
                .map(|registration| ((*registration).type_id(), registration.short_name()))
        };
        let missing = addable().filter(|(id, _)| !present.contains(id)).collect();
        (missing, addable().count())
    }
    pub fn visible_components(&self, entity: Entity) -> Vec<(TypeId, &'static str)> {
        let table = self.table.read();
        self.components
            .component_ids(entity)
            .iter()
            .filter_map(|&id| Some((id, table.get(id)?.short_name())))
            .collect()
    }
    /// The edit `App`'s type registry.
    pub fn types(&self) -> impl Deref<Target = TypeRegistry> + '_ {
        self.table.read()
    }
    /// The component `id` on `entity`, or `None` when it is absent or has no reflection data.
    pub fn read(&self, entity: Entity, id: TypeId) -> Option<&dyn Reflect> {
        (self.table.get_type_data::<ReflectComponent>(id)?.read)(&self.components, entity)
    }
    pub fn collect_component(
        &self,
        registry: &InspectorRegistry,
        entity: Entity,
        id: TypeId,
    ) -> Option<Vec<Property>> {
        Some(registry.collect(self.read(entity, id)?, &self.table.read()))
    }
}

impl SystemInput for InspectionSource<'_> {
    type State = ();
    type Data<'world, 'state> = InspectionSource<'world>;
    fn init_state(_: &mut World) {}
    unsafe fn get_data<'world, 'state>(
        state: &'state mut (),
        world: UnsafeWorldCell<'world>,
        ticks: SystemTicks,
    ) -> Result<Self::Data<'world, 'state>, ValidationError> {
        // SAFETY: this input declares exactly the access of its two parts.
        Ok(InspectionSource {
            components: unsafe { ComponentMetadata::get_data(state, world, ticks) }?,
            table: unsafe { Res::new(world, ticks) }?,
        })
    }
    fn fill_access(meta: &mut SystemMetadata, access: &mut SystemAccess) {
        ComponentMetadata::fill_access(meta, access);
        Res::<AppTypeRegistry>::fill_access(meta, access);
    }
}
impl ReadOnlySystemInput for InspectionSource<'_> {}

pub trait EditableApp {
    fn register_property_editor<T: Reflect, E: PropertyEditor<T>>(&mut self) -> &mut Self;
}

impl EditableApp for App {
    fn register_property_editor<T: Reflect, E: PropertyEditor<T>>(&mut self) -> &mut Self {
        registry(self).register_property_editor::<T, E>();
        self
    }
}

fn registry(app: &mut App) -> ResMut<'_, InspectorRegistry> {
    let world = app.main_mut().world_mut();
    let ticks = world.out_of_system_ticks();
    // SAFETY: the exclusive borrow of the world covers any access.
    unsafe { ResMut::new(world.as_unsafe_world_cell_mut(), ticks) }
        .expect("InspectorPlugin must be registered first")
}

pub(crate) fn current_editor(
    registry: &InspectorRegistry,
    types: Option<&TypeRegistry>,
    changed: Tick,
    row: &PropertyRow,
) -> Result<RegisteredEditor, EditError> {
    registry
        .editor(row.address.type_id, types)
        .filter(|editor| row.editor_type == editor.editor_type && row.registry_tick == changed)
        .ok_or(EditError::StaleEditor)
}

pub(crate) fn checked(
    registry: &InspectorRegistry,
    types: Option<&TypeRegistry>,
    changed: Tick,
    commit: PropertyCommit,
) -> Result<CommitProperty, EditError> {
    let row = commit.row;
    let editor = current_editor(registry, types, changed, &row)?;
    Ok(CommitProperty {
        editor: editor.adapter,
        row,
        edit: commit.edit,
    })
}

pub(super) fn send_property_commits(
    mut commits: ResMut<PropertyCommits>,
    registry: Res<InspectorRegistry>,
    data: Res<InspectorData>,
    source: EditWorld<InspectionSource>,
    mut scene: EditWorldSignals,
) {
    if !commits.holds_for(TargetWorld::EditTarget) {
        return;
    }
    let types = source.types();
    for commit in commits.take_for(TargetWorld::EditTarget) {
        match checked(&registry, Some(&types), registry.changed_tick(), commit) {
            Ok(commit) => {
                for commit in fanned_out(commit, &data, &source) {
                    scene.trigger(commit);
                }
            }
            Err(error) => log::warn!("Property commit dropped: {error}"),
        }
    }
}

fn fanned_out(
    commit: CommitProperty,
    data: &InspectorData,
    source: &InspectionSource,
) -> Vec<CommitProperty> {
    let Some(target) = commit.row.address.target.component() else {
        return vec![commit];
    };
    if data.scene_entities.len() < 2 || data.scene_entity != Some(target.entity) {
        return vec![commit];
    }
    let holders: Vec<Entity> = data
        .scene_entities
        .iter()
        .copied()
        .filter(|&entity| source.carries(entity, target.component))
        .collect();
    if commit.edit.is::<ListEdit>() && !same_on_all(&commit, &holders, source) {
        return vec![commit];
    }
    holders
        .into_iter()
        .map(|entity| CommitProperty {
            row: PropertyRow {
                address: PropertyAddress {
                    target: Arc::new(ComponentTarget {
                        entity,
                        component: target.component,
                    }),
                    ..commit.row.address.clone()
                },
                ..commit.row.clone()
            },
            edit: commit.edit.clone(),
            editor: commit.editor.clone(),
        })
        .collect()
}

fn same_on_all(commit: &CommitProperty, holders: &[Entity], source: &InspectionSource) -> bool {
    let address = &commit.row.address;
    let Some(component) = address.target.component() else {
        return false;
    };
    let snapshot = |entity: Entity| {
        let mut shot = None;
        let value = source.read(entity, component.component)?;
        let _ = with_property(value, &address.path, &mut |field| {
            shot = commit.editor.snapshot(field).ok();
        });
        shot
    };
    let active = snapshot(component.entity);
    active.is_some() && holders.iter().all(|&entity| snapshot(entity) == active)
}
