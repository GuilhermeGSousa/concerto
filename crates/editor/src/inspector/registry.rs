use std::{
    any::{Any, TypeId},
    collections::HashMap,
    marker::PhantomData,
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
    PropertyPath, PropertyVisitor, Reflect, ReflectDefault, ReflectEnumInfo, ReflectOption,
    TypeRegistry,
};
use concerto_ui::theme::UITheme;

use super::rows::{
    EditContext, EditError, Property, PropertyCommit, PropertyCommits, PropertyEditor,
    PropertyRowValue,
};
use super::variants::{OptionToggle, VariantPicker};
use crate::asset_picker::AssetPicker;
use crate::hosted::{EditWorld, EditWorldSignals};
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
    fn apply(
        &self,
        value: &mut dyn Reflect,
        edit: &dyn Any,
        context: &EditContext,
    ) -> Result<(), EditError>;
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

    /// The editor registered for `id`, or else the one its type data in `types` calls for: the asset picker, an option toggle or a variant picker.
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

    #[cfg(test)]
    pub(crate) fn collect_component(
        &self,
        registry_tick: Tick,
        scene: &World,
        entity: Entity,
        component: TypeId,
    ) -> Option<Vec<Property>> {
        let types = scene.get_resource::<AppTypeRegistry>()?;
        let entry = types.get_type_data::<ReflectComponent>(component)?;
        let value = (entry.read)(&ComponentMetadata::new(scene), entity)?;
        Some(self.collect_at(registry_tick, value, types))
    }

    fn collect_at(
        &self,
        registry_tick: Tick,
        root: &dyn Reflect,
        types: &TypeRegistry,
    ) -> Vec<Property> {
        let mut properties = self.collect(root, types);
        for property in &mut properties {
            property.registry_tick = registry_tick;
        }
        properties
    }
}

struct Collect<'a> {
    registry: &'a InspectorRegistry,
    types: &'a TypeRegistry,
    path: Vec<&'static str>,
    properties: Vec<Property>,
}

impl Collect<'_> {
    fn node(&mut self, value: &dyn Reflect) {
        let type_id = (value as &dyn Any).type_id();
        if let Some(editor) = self.registry.editor(type_id, Some(self.types)) {
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
            if editor.walks_children {
                value.visit(self);
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
    pub fn is_registered(&self, id: TypeId) -> bool {
        self.table.get_type_data::<ReflectComponent>(id).is_some()
    }
    pub fn follows_snapshot(&self, registry: &InspectorRegistry, property: &Property) -> bool {
        registry
            .editor(property.type_id, Some(&self.table))
            .is_some_and(|editor| {
                Some(editor.editor_type) == property.editor_type
                    && editor.adapter.follows_snapshot()
            })
    }
    /// Addable components `entity` does not carry yet, with how many are addable in total.
    pub fn addable_components(&self, entity: Entity) -> (Vec<(TypeId, &'static str)>, usize) {
        let present = self.components.component_ids(entity);
        let addable = || {
            self.table
                .iter()
                .filter(|registration| {
                    registration.data::<ReflectComponent>().is_some()
                        && registration.data::<ReflectDefault>().is_some()
                })
                .map(|registration| (registration.type_id(), registration.short_name()))
        };
        let missing = addable().filter(|(id, _)| !present.contains(id)).collect();
        (missing, addable().count())
    }
    pub fn visible_components(&self, entity: Entity) -> Vec<(TypeId, &'static str)> {
        self.components
            .component_ids(entity)
            .iter()
            .filter_map(|&id| Some((id, self.table.get(id)?.short_name())))
            .collect()
    }
    pub fn collect_component(
        &self,
        registry: &InspectorRegistry,
        registry_tick: Tick,
        entity: Entity,
        id: TypeId,
    ) -> Option<Vec<Property>> {
        let value =
            (self.table.get_type_data::<ReflectComponent>(id)?.read)(&self.components, entity)?;
        Some(registry.collect_at(registry_tick, value, &self.table))
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

fn checked(
    registry: &InspectorRegistry,
    types: Option<&TypeRegistry>,
    changed: Tick,
    commit: PropertyCommit,
) -> Result<CommitProperty, EditError> {
    let row = commit.row;
    let editor = registry
        .editor(row.type_id, types)
        .filter(|editor| {
            row.editor_type == Some(editor.editor_type) && row.registry_tick == changed
        })
        .ok_or(EditError::StaleEditor)?;
    Ok(CommitProperty {
        editor: editor.adapter,
        row,
        edit: commit.edit,
    })
}

pub(super) fn send_property_commits(
    mut commits: ResMut<PropertyCommits>,
    registry: Res<InspectorRegistry>,
    types: EditWorld<Option<Res<AppTypeRegistry>>>,
    mut scene: EditWorldSignals,
) {
    if commits.0.is_empty() {
        return;
    }
    let types = types.as_deref().map(|types| &types.0);
    for commit in std::mem::take(&mut commits.0) {
        match checked(&registry, types, registry.changed_tick(), commit) {
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
    use concerto_foundation::{
        assets::{asset_server::AssetServer, registry::AssetRegistry},
        transform::Transform,
    };
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

    fn scene() -> (World, Entity) {
        let mut world = World::default();
        world.register_component::<Transform>();
        world.register_type::<Transform>();
        world.insert_resource(AssetServer::new());
        world.insert_resource(AssetRegistry::default());
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
        checked(registry, None, changed, commit).err()
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
        let ticks = s.f.world.out_of_system_ticks();
        // SAFETY: the exclusive borrow of the world covers any access.
        unsafe { ResMut::<InspectorRegistry>::new(s.f.world.as_unsafe_world_cell_mut(), ticks) }
            .unwrap()
            .register_property_editor::<Vec3, NumericFields>();
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
        let commit = checked(&registry, None, Tick::default(), malformed)
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
        scene.insert_resource(AppTypeRegistry::default());
        let commit = checked(&registry, None, Tick::default(), edit).unwrap();
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
        checked(registry, None, Tick::default(), commit).unwrap()
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
    fn a_scene_without_its_asset_resources_refuses_every_edit_and_keeps_them() {
        let (mut scene, entity) = scene();
        let registry = InspectorRegistry::default();
        let row = row_in(&registry, &scene, entity);
        scene.remove_resource::<AssetRegistry>();

        assert_eq!(
            numeric(&registry, row, 4.0).apply(&mut scene),
            Err(EditError::MissingAssets)
        );

        assert_eq!(
            scene
                .get_component_for_entity::<Transform>(entity)
                .unwrap()
                .translation,
            Vec3::ZERO
        );
        assert!(scene.get_resource::<AssetServer>().is_some());
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

    #[derive(concerto_reflect::Reflect, Default)]
    enum Shape {
        #[default]
        Dot,
        Circle(f32),
        Square {
            side: f32,
            round: bool,
        },
    }

    #[derive(concerto_reflect::Reflect, Default)]
    enum Maybe {
        #[default]
        None,
        Some(f32),
    }

    #[derive(concerto_reflect::Reflect, Default)]
    struct Fog {
        density: f32,
    }

    #[derive(concerto_reflect::Reflect, Default)]
    struct Holder {
        shape: Shape,
        fog: Option<Fog>,
    }

    fn types() -> TypeRegistry {
        let mut types = TypeRegistry::default();
        types.register::<Holder>();
        types.register::<Maybe>();
        types
    }

    fn chosen<T: 'static>(registry: &InspectorRegistry) -> Option<(TypeId, bool)> {
        registry
            .editor(TypeId::of::<T>(), Some(&types()))
            .map(|editor| (editor.editor_type, editor.walks_children))
    }

    fn collected(
        registry: &InspectorRegistry,
        holder: &Holder,
    ) -> Vec<(Vec<&'static str>, TypeId)> {
        registry
            .collect(holder, &types())
            .into_iter()
            .map(|property| {
                (
                    property.path.segments().to_vec(),
                    property.editor_type.expect("every row has an editor"),
                )
            })
            .collect()
    }

    fn picker() -> TypeId {
        TypeId::of::<VariantPicker>()
    }

    fn toggle() -> TypeId {
        TypeId::of::<OptionToggle>()
    }

    fn of<T: 'static>() -> TypeId {
        InspectorRegistry::default()
            .editor(TypeId::of::<T>(), None)
            .unwrap()
            .editor_type
    }

    #[test]
    fn enums_and_options_choose_a_walking_editor_from_their_type_alone() {
        let registry = InspectorRegistry::default();
        assert_eq!(chosen::<Option<Fog>>(&registry), Some((toggle(), true)));
        assert_eq!(chosen::<Shape>(&registry), Some((picker(), true)));
        assert_eq!(chosen::<Maybe>(&registry), Some((picker(), true)));
        assert_eq!(chosen::<f32>(&registry), Some((of::<f32>(), false)));
    }

    #[test]
    fn a_unit_variant_and_none_yield_only_their_own_rows() {
        let rows = collected(&InspectorRegistry::default(), &Holder::default());
        assert_eq!(rows, [(vec!["shape"], picker()), (vec!["fog"], toggle())]);
    }

    #[test]
    fn the_active_variants_fields_follow_its_picker_row() {
        let registry = InspectorRegistry::default();
        let square = Holder {
            shape: Shape::Square {
                side: 2.0,
                round: true,
            },
            ..Default::default()
        };
        assert_eq!(
            collected(&registry, &square),
            [
                (vec!["shape"], picker()),
                (vec!["shape", "side"], of::<f32>()),
                (vec!["shape", "round"], of::<bool>()),
                (vec!["fog"], toggle()),
            ]
        );
        let circle = Holder {
            shape: Shape::Circle(1.0),
            ..Default::default()
        };
        assert_eq!(
            collected(&registry, &circle)[..2],
            [(vec!["shape"], picker()), (vec!["shape", "0"], of::<f32>())]
        );
    }

    #[test]
    fn some_is_followed_by_the_values_rows() {
        let holder = Holder {
            fog: Some(Fog { density: 0.5 }),
            ..Default::default()
        };
        assert_eq!(
            collected(&InspectorRegistry::default(), &holder),
            [
                (vec!["shape"], picker()),
                (vec!["fog"], toggle()),
                (vec!["fog", "0", "density"], of::<f32>()),
            ]
        );
    }

    #[derive(Default)]
    struct Whole;

    impl<T: Reflect> PropertyEditor<T> for Whole {
        type Snapshot = ();
        type Edit = ();
        fn snapshot(&self, _: &T) {}
        fn build(&self, _: &mut concerto_ecs::command::EntityCommandQueue, _: &(), _: &UITheme) {}
        fn apply(&self, _: &mut T, _: &(), _: &EditContext) -> Result<(), EditError> {
            Ok(())
        }
    }

    #[test]
    fn an_editor_registered_for_the_exact_type_wins_and_hides_the_fields() {
        let mut registry = InspectorRegistry::default();
        registry.register_property_editor::<Shape, Whole>();
        registry.register_property_editor::<Option<Fog>, Whole>();
        let holder = Holder {
            shape: Shape::Circle(1.0),
            fog: Some(Fog::default()),
        };
        let whole = TypeId::of::<Whole>();
        assert_eq!(
            collected(&registry, &holder),
            [(vec!["shape"], whole), (vec!["fog"], whole)]
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
        checked(&registry, None, Tick::default(), edit)
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
