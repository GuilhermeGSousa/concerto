//! Writing property values and adding or removing components on scene entities.
use std::{
    any::{Any, TypeId},
    sync::Arc,
};

use concerto_app::{App, edit_time::AtEditTime};
use concerto_ecs::{
    Entity, Resource, World,
    events::Event,
    reflect::{AppTypeRegistry, ReflectComponent},
    signal::{On, Signal},
    system::input::ComponentMetadata,
};
use concerto_foundation::assets::{
    asset_server::AssetServer, handle::LiveLeaves, registry::AssetRegistry,
};
use concerto_reflect::{
    PathSegment, PropertyPath, Reflect, ReflectDefault, ReflectJson, ReflectRef, ReflectedValue,
    TypeRegistry, ValueWarning, apply_value_with, json_to_value_with, with_property,
    with_property_mut,
};
use serde_json::Value;

pub use crate::property::{EditContext, EditError, PropertyRow};
use crate::{
    property::{ComponentTarget, ErasedEditor, PropertyAddress, PropertyTarget},
    tools::{
        entity_ops::in_open_scene,
        links::{LinkedEdit, refusal},
        scene::{SceneRoot, SceneStatus, report_edit, write},
    },
};

/// One value written into a component, already checked against the widget registry.
pub struct CommitProperty {
    /// The row the edit came from, which addresses the component and the path.
    pub row: PropertyRow,
    /// The editor's edit, erased; the editor downcasts it.
    pub edit: Arc<dyn Any + Send + Sync>,
    /// The editor registered for the property, which applies the edit.
    pub editor: Arc<dyn ErasedEditor>,
}

impl Signal for CommitProperty {}

/// What one edit of a property did to the row it addressed, or why it was refused.
#[derive(Event, Clone)]
pub struct PropertyCommitted {
    /// The property the edit addressed.
    pub address: PropertyAddress,
    /// `Ok` when the value was written, or why it was refused.
    pub result: Result<(), PropertyRefusal>,
}

/// Why a property write was refused, each carrying the status text Chatter shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PropertyRefusal {
    /// The JSON value does not decode to the property's type.
    Undecodable(String),
    /// No JSON can write the property: under a list, a type with no JSON form and no registered `Default`.
    NotEditable(String),
    /// Anything else: the scene root, a missing target, or an adapter's validation.
    Refused(String),
}

impl PropertyRefusal {
    /// The status text.
    pub fn status(&self) -> &str {
        match self {
            Self::Undecodable(status) | Self::NotEditable(status) | Self::Refused(status) => status,
        }
    }

    /// The same refusal with its text reworded.
    pub(crate) fn map(self, reword: impl FnOnce(String) -> String) -> Self {
        match self {
            Self::Undecodable(status) => Self::Undecodable(reword(status)),
            Self::NotEditable(status) => Self::NotEditable(reword(status)),
            Self::Refused(status) => Self::Refused(reword(status)),
        }
    }
}

impl std::fmt::Display for PropertyRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.status())
    }
}

fn refused(reason: impl ToString) -> PropertyRefusal {
    PropertyRefusal::Refused(reason.to_string())
}

impl CommitProperty {
    /// Writes the value into the component the row addresses.
    ///
    /// # Errors
    ///
    /// Returns [`EditError::MissingTarget`] when the row addresses no component or its entity is gone,
    /// [`EditError::UnregisteredComponent`] when the component type is not registered,
    /// [`EditError::MissingAssets`] when `world` has no asset server or asset registry, and the editor's
    /// own error when the property is not found or the value is refused.
    pub fn apply(&self, world: &mut World) -> Result<(), EditError> {
        let target = self
            .row
            .address
            .target
            .component()
            .ok_or(EditError::MissingTarget)?;
        let component = reflected(world, target.component)?;
        with_edit_context(world, |world, context| {
            apply_through(
                world,
                component,
                target.entity,
                &self.row.address.path,
                self.editor.as_ref(),
                self.edit.as_ref(),
                context,
            )
        })
    }
}

/// Runs `edit` with the world's asset server and registry, or answers `MissingAssets` without either.
pub(crate) fn with_edit_context(
    world: &mut World,
    edit: impl FnOnce(&mut World, &EditContext) -> Result<(), EditError>,
) -> Result<(), EditError> {
    let mut result = Err(EditError::MissingAssets);
    world.resource_scope::<AssetServer>(|world, assets| {
        world.resource_scope::<AssetRegistry>(|world, registry| {
            result = edit(world, &EditContext { assets, registry });
        });
    });
    result
}

/// Applies `edit` through `editor` to the property at `path` of `value`.
pub(crate) fn apply_at(
    value: &mut dyn Reflect,
    path: &PropertyPath,
    editor: &dyn ErasedEditor,
    edit: &dyn Any,
    context: &EditContext,
) -> Result<(), EditError> {
    let mut result = Err(EditError::NotFound);
    if with_property_mut(value, path, &mut |value| {
        result = editor.apply(value, edit, context);
    })
    .is_err()
    {
        result = Err(EditError::NotFound);
    }
    result
}

fn apply_through(
    world: &mut World,
    component: ReflectComponent,
    entity: Entity,
    path: &PropertyPath,
    editor: &dyn ErasedEditor,
    edit: &dyn Any,
    context: &EditContext,
) -> Result<(), EditError> {
    let mut result = Err(EditError::NotFound);
    let found = (component.edit)(world, entity, &mut |value| {
        result = apply_at(value, path, editor, edit, context);
    });
    if found {
        result
    } else {
        Err(EditError::MissingTarget)
    }
}

/// Writes a JSON value, shaped as `inspect` reports it, into the property at `path` of a component; an empty path names the whole component.
pub struct SetValue {
    /// The entity in the edit target's world.
    pub entity: Entity,
    /// The type of the component on `entity`.
    pub component: TypeId,
    /// The property inside the component, empty for the whole component.
    pub path: PropertyPath,
    /// The new value, as JSON in the shape `inspect` reports.
    pub value: Value,
}

impl Signal for SetValue {}

impl SetValue {
    /// The component this writes to.
    pub(crate) fn target(&self) -> Arc<dyn PropertyTarget> {
        Arc::new(ComponentTarget {
            entity: self.entity,
            component: self.component,
        })
    }

    /// The address of this write, whose value has type `type_id`.
    pub(crate) fn address(&self, type_id: TypeId) -> PropertyAddress {
        PropertyAddress {
            target: self.target(),
            path: self.path.clone(),
            type_id,
        }
    }

    fn write(&self, world: &mut World, field: &mut TypeId) -> Result<(), PropertyRefusal> {
        let component = reflected(world, self.component).map_err(refused)?;
        if !world.entity_is_valid(self.entity) {
            return Err(refused(EditError::MissingTarget));
        }
        if world
            .get_component_for_entity::<SceneRoot>(self.entity)
            .is_some()
        {
            return Err(refused("the scene root cannot be changed"));
        }
        in_open_scene(world, self.entity).map_err(refused)?;
        if let Some(reason) = refusal(
            world,
            self.entity,
            LinkedEdit::SetOwnComponent(self.component),
        ) {
            return Err(refused(reason));
        }
        let mut hook = LiveLeaves::new(world);
        let stored = {
            let live = (component.read)(&ComponentMetadata::new(world), self.entity)
                .ok_or_else(|| refused(EditError::MissingTarget))?;
            with_property(live, &self.path, &mut |value| {
                *field = (value as &dyn Any).type_id();
            })
            .map_err(|_| refused(EditError::NotFound))?;
            let Some(registry) = world.get_resource::<AppTypeRegistry>() else {
                return Err(PropertyRefusal::Undecodable(
                    EditError::UnregisteredComponent.to_string(),
                ));
            };
            decode_property(
                &self.value,
                live,
                self.component,
                &self.path,
                *field,
                &registry.read(),
                &mut hook,
            )?
        };
        let mut warnings = Vec::new();
        (component.edit)(world, self.entity, &mut |value| {
            let _ = with_property_mut(value, &self.path, &mut |value| {
                warnings = apply_value_with(value, &stored, &mut hook);
            });
        });
        applied(&warnings)
    }
}

/// `Ok` unless applying a decoded value left `warnings`.
pub(crate) fn applied(warnings: &[ValueWarning]) -> Result<(), PropertyRefusal> {
    if warnings.is_empty() {
        Ok(())
    } else {
        Err(PropertyRefusal::Undecodable(
            warnings
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; "),
        ))
    }
}

/// Decodes `json` for the property at `path` of `live`, a value of type `root`, where the property has type `field`.
pub(crate) fn decode_property(
    json: &Value,
    live: &dyn Reflect,
    root: TypeId,
    path: &PropertyPath,
    field: TypeId,
    types: &TypeRegistry,
    hook: &mut LiveLeaves,
) -> Result<ReflectedValue, PropertyRefusal> {
    if types.get_type_data::<ReflectJson>(field).is_some()
        || types.get_type_data::<ReflectDefault>(field).is_some()
    {
        return json_to_value_with(json, field, types, hook).map_err(PropertyRefusal::Undecodable);
    }
    let segments = path.segments();
    if segments
        .iter()
        .any(|segment| matches!(segment, PathSegment::Index(_)))
    {
        let name = types
            .get(field)
            .map_or("the type", |registration| registration.type_path());
        return Err(PropertyRefusal::NotEditable(format!(
            "{name} has no JSON form and no registered Default, so JSON cannot build it inside a list"
        )));
    }
    let variants: Vec<Option<&'static str>> = (0..segments.len())
        .map(|depth| {
            let mut variant = None;
            let _ = with_property(
                live,
                &PropertyPath::new(segments[..depth].iter().copied()),
                &mut |owner| {
                    if let ReflectRef::Enum(owner) = owner.reflect_ref() {
                        variant = Some(owner.variant_name());
                    }
                },
            );
            variant
        })
        .collect();
    let wrapped =
        segments
            .iter()
            .zip(&variants)
            .rev()
            .fold(json.clone(), |inner, (segment, variant)| {
                let field = Value::Object([(segment.to_string(), inner)].into_iter().collect());
                match variant {
                    Some(variant) => {
                        Value::Object([(variant.to_string(), field)].into_iter().collect())
                    }
                    None => field,
                }
            });
    let stored =
        json_to_value_with(&wrapped, root, types, hook).map_err(PropertyRefusal::Undecodable)?;
    segments
        .iter()
        .try_fold(stored, |stored, segment| {
            let (ReflectedValue::Struct(fields) | ReflectedValue::Variant { fields, .. }) = stored
            else {
                return None;
            };
            fields
                .into_iter()
                .find(|(name, _)| *segment == name.as_str())
                .map(|(_, value)| value)
        })
        .ok_or_else(|| PropertyRefusal::Undecodable(EditError::Rejected.to_string()))
}

/// Adds an addable component at its default value, or removes a registered one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComponentEdit {
    /// Adds `component`, at its registered default, to `entity`.
    Add {
        /// The entity in the edit target's world.
        entity: Entity,
        /// The type of the component to add.
        component: TypeId,
    },
    /// Removes `component` from `entity`.
    Remove {
        /// The entity in the edit target's world.
        entity: Entity,
        /// The type of the component to remove.
        component: TypeId,
    },
}

/// Structural edits waiting to be sent to the scene.
#[derive(Resource, Default)]
pub struct ComponentEdits(pub Vec<ComponentEdit>);

/// One component added to or removed from a scene entity.
pub struct EditComponent(pub ComponentEdit);

impl Signal for EditComponent {}

/// What one `EditComponent` did, or the status text saying why it was refused.
#[derive(Event, Clone)]
pub struct ComponentEdited {
    /// The edit that was asked for.
    pub edit: ComponentEdit,
    /// `Ok` when it was applied, or the status text saying why it was refused.
    pub result: Result<(), String>,
}

/// Applies one structural edit to the live world.
///
/// # Errors
///
/// Returns [`EditError::UnregisteredComponent`] when the component type is not registered,
/// [`EditError::MissingTarget`] when the entity does not exist, [`EditError::Rejected`] when adding a
/// component the entity already has or one with no default, and [`EditError::NotFound`] when removing
/// one it lacks.
pub fn apply_component_edit(world: &mut World, edit: ComponentEdit) -> Result<(), EditError> {
    let (ComponentEdit::Add { entity, component } | ComponentEdit::Remove { entity, component }) =
        edit;
    let registered = reflected(world, component)?;
    if !world.entity_is_valid(entity) {
        return Err(EditError::MissingTarget);
    }
    let present = world.component_ids(entity).contains(&component);
    match edit {
        ComponentEdit::Add { .. } if present => Err(EditError::Rejected),
        ComponentEdit::Remove { .. } if !present => Err(EditError::NotFound),
        ComponentEdit::Add { .. } => {
            if registered.insert_default(component, world, entity) {
                Ok(())
            } else {
                Err(EditError::Rejected)
            }
        }
        ComponentEdit::Remove { .. } => {
            (registered.remove)(world, entity);
            Ok(())
        }
    }
}

fn reflected(world: &World, component: TypeId) -> Result<ReflectComponent, EditError> {
    world
        .get_resource::<AppTypeRegistry>()
        .and_then(|registry| registry.get_type_data::<ReflectComponent>(component))
        .ok_or(EditError::UnregisteredComponent)
}

pub(crate) fn commit_property(on: On<CommitProperty>, world: &mut World) {
    let address = &on.signal().row.address;
    let target = address.target.component();
    let property = match target {
        Some(target) => property_name(world, target.component, &address.path),
        None => address.target.name(),
    };
    let result = target
        .ok_or(EditError::MissingTarget)
        .map_err(|error| error.to_string())
        .and_then(|target| {
            in_open_scene(world, target.entity).map_err(|error| error.to_string())?;
            match refusal(
                world,
                target.entity,
                LinkedEdit::SetOwnComponent(target.component),
            ) {
                Some(reason) => Err(reason),
                None => Ok(()),
            }
        })
        .and_then(|()| on.signal().apply(world).map_err(|error| error.to_string()))
        .map_err(|reason| refused(format!("Set {property} failed: {reason}")));
    report(world, &result);
    write(
        world,
        PropertyCommitted {
            address: address.clone(),
            result,
        },
    );
}

pub(crate) fn set_value(on: On<SetValue>, world: &mut World) {
    let set = on.signal();
    let property = property_name(world, set.component, &set.path);
    let mut field = TypeId::of::<()>();
    let result = set
        .write(world, &mut field)
        .map_err(|refusal| refusal.map(|reason| format!("Set {property} failed: {reason}")));
    report(world, &result);
    write(
        world,
        PropertyCommitted {
            address: set.address(field),
            result,
        },
    );
}

fn property_name(world: &World, component: TypeId, path: &PropertyPath) -> String {
    dotted_name(component_name(world, component), path)
}

/// `owner` and the segments of `path`, as a status line names a property.
pub(crate) fn dotted_name(owner: &str, path: &PropertyPath) -> String {
    std::iter::once(owner.to_string())
        .chain(path.segments().iter().map(ToString::to_string))
        .collect::<Vec<_>>()
        .join(".")
}

pub(crate) fn edit_component(on: On<EditComponent>, world: &mut World) {
    let edit = on.signal().0;
    let (verb, entity, component) = match edit {
        ComponentEdit::Add { entity, component } => ("Add", entity, component),
        ComponentEdit::Remove { entity, component } => ("Remove", entity, component),
    };
    let name = component_name(world, component);
    let result = in_open_scene(world, entity)
        .map_err(|error| error.to_string())
        .and_then(|()| {
            apply_component_edit(world, edit).map_err(|error| match (edit, error) {
                (ComponentEdit::Add { .. }, EditError::Rejected) => {
                    "it is already there, or has no default value".to_owned()
                }
                (ComponentEdit::Remove { .. }, EditError::NotFound) => {
                    "the entity does not have it".to_owned()
                }
                (_, error) => error.to_string(),
            })
        })
        .map_err(|reason| format!("{verb} {name} failed: {reason}"));
    report(world, &result);
    write(world, ComponentEdited { edit, result });
}

fn component_name(world: &World, component: TypeId) -> &'static str {
    world
        .get_resource::<AppTypeRegistry>()
        .and_then(|registry| registry.read().get(component))
        .map_or("the component", |registration| registration.short_name())
}

fn report(world: &mut World, result: &Result<(), impl ToString>) {
    match result {
        Ok(()) => report_edit(world),
        Err(status) => write(world, SceneStatus(status.to_string())),
    }
}

pub(crate) fn build(app: &mut App) {
    app.register_event::<PropertyCommitted>()
        .register_event::<ComponentEdited>()
        .add_listener(commit_property.at_edit_time())
        .add_listener(set_value.at_edit_time())
        .add_listener(edit_component.at_edit_time());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inspector::{InspectorRegistry, ListEdit, ListEditor, PropertyEditor};
    use crate::tools::scene::{
        SceneDirty, SceneEdited,
        testing::{tools_app, written},
    };
    use concerto_ecs::{
        Component,
        command::EntityCommandQueue,
        component::{Tick, scene::SceneEntityRef},
    };
    use concerto_foundation::assets::{
        Asset, AssetId, LoadableAsset, asset_store::AssetStore, handle::AssetHandle,
    };
    use concerto_foundation::transform::Transform;
    use concerto_ui::theme::UITheme;
    use serde_json::{Value, json};

    #[derive(Component, concerto_reflect::Reflect, Default)]
    #[reflect(Component, Default)]
    struct Gain {
        amount: f32,
    }

    #[derive(Component, concerto_reflect::Reflect, Default)]
    #[reflect(Component, Default)]
    struct Rig {
        stops: Vec<f32>,
        mode: Mode,
    }

    #[derive(concerto_reflect::Reflect, Default, Debug, PartialEq)]
    enum Mode {
        #[default]
        Fixed,
        Orbit {
            distance: f32,
        },
    }

    #[derive(Component, concerto_reflect::Reflect, Default)]
    #[reflect(Component, Default)]
    struct Follow {
        target: SceneEntityRef,
    }

    #[derive(serde::Serialize, serde::Deserialize)]
    struct Note;

    impl Asset for Note {
        fn name() -> &'static str {
            "Note"
        }
    }

    impl LoadableAsset for Note {}

    #[derive(Component, concerto_reflect::Reflect, Default)]
    #[reflect(Component, Default)]
    struct Voice {
        note: AssetHandle<Note>,
    }

    struct Fixture {
        app: App,
        entity: Entity,
    }

    fn fixture() -> Fixture {
        let mut app = tools_app();
        app.register_type::<Transform>()
            .register_type::<Gain>()
            .register_type::<Rig>()
            .register_type::<Follow>()
            .register_type::<Voice>()
            .register_type::<AssetHandle<Note>>()
            .insert_resource(AssetServer::new())
            .insert_resource(AssetRegistry::default());
        let entity = app.main_mut().world_mut().spawn(Transform::IDENTITY);
        Fixture { app, entity }
    }

    fn read<C: Component, R>(app: &mut App, entity: Entity, read: impl FnOnce(&C) -> R) -> R {
        read(
            app.main_mut()
                .world_mut()
                .get_component_for_entity::<C>(entity)
                .unwrap(),
        )
    }

    fn results(app: &App) -> Vec<bool> {
        written::<PropertyCommitted>(app)
            .into_iter()
            .map(|outcome| outcome.result.is_ok())
            .collect()
    }

    fn dirty(app: &App) -> bool {
        app.get_resource::<SceneDirty>().unwrap().0
    }

    #[derive(Default)]
    struct NonNegative;

    impl PropertyEditor<f32> for NonNegative {
        type Snapshot = f32;
        type Edit = f32;

        fn snapshot(&self, value: &f32) -> f32 {
            *value
        }

        fn build(&self, _: &mut EntityCommandQueue, _: &f32, _: &UITheme) {}

        fn apply(&self, value: &mut f32, edit: &f32, _: &EditContext) -> Result<(), EditError> {
            if *edit < 0.0 {
                return Err(EditError::Rejected);
            }
            *value = *edit;
            Ok(())
        }
    }

    fn row<C: 'static, T: 'static>(
        entity: Entity,
        path: &[&'static str],
        editor: TypeId,
    ) -> PropertyRow {
        PropertyRow {
            address: PropertyAddress {
                target: Arc::new(ComponentTarget {
                    entity,
                    component: TypeId::of::<C>(),
                }),
                path: PropertyPath::new(path.iter().copied()),
                type_id: TypeId::of::<T>(),
            },
            editor_type: editor,
            registry_tick: Tick::default(),
        }
    }

    fn gain_to(entity: Entity, amount: f32) -> CommitProperty {
        let mut registry = InspectorRegistry::default();
        registry.register_property_editor::<f32, NonNegative>();
        let editor = registry.editor(TypeId::of::<f32>(), None).unwrap();
        CommitProperty {
            row: row::<Gain, f32>(entity, &["amount"], editor.editor_type),
            edit: Arc::new(amount),
            editor: Arc::clone(&editor.adapter),
        }
    }

    fn list_edit(entity: Entity, edit: ListEdit) -> CommitProperty {
        CommitProperty {
            row: row::<Rig, Vec<f32>>(entity, &["stops"], TypeId::of::<ListEditor>()),
            edit: Arc::new(edit),
            editor: Arc::new(ListEditor),
        }
    }

    #[test]
    fn committed_properties_land_in_the_scene_and_refused_ones_change_nothing() {
        let Fixture { mut app, entity } = fixture();
        app.main_mut().world_mut().insert(Gain::default(), entity);

        app.main_mut().world_mut().trigger(gain_to(entity, -1.0));
        assert_eq!(read(&mut app, entity, |gain: &Gain| gain.amount), 0.0);
        assert!(!dirty(&app));

        app.main_mut().world_mut().trigger(gain_to(entity, 2.0));
        assert_eq!(read(&mut app, entity, |gain: &Gain| gain.amount), 2.0);
        assert!(dirty(&app));
        assert_eq!(results(&app), [false, true]);
        assert_eq!(written::<SceneEdited>(&app).len(), 1);
    }

    #[test]
    fn a_list_commit_pushes_a_default_element_and_removes_one_by_index() {
        let Fixture { mut app, entity } = fixture();
        app.main_mut().world_mut().insert(
            Rig {
                stops: vec![1.4],
                mode: Mode::Fixed,
            },
            entity,
        );

        for edit in [ListEdit::Push, ListEdit::Remove(0), ListEdit::Remove(5)] {
            app.main_mut().world_mut().trigger(list_edit(entity, edit));
        }

        assert_eq!(read(&mut app, entity, |rig: &Rig| rig.stops.clone()), [0.0]);
        assert_eq!(results(&app), [true, true, false]);
        assert_eq!(written::<SceneEdited>(&app).len(), 2);
    }

    #[test]
    fn a_set_value_writes_a_field_switches_a_variant_and_resolves_entity_and_asset_references() {
        let Fixture { mut app, entity } = fixture();
        let mut server = AssetServer::new();
        server.register_asset(&AssetStore::<Note>::new());
        app.insert_resource(server.clone());
        let target = app.main_mut().world_mut().spawn(());
        app.main_mut().world_mut().insert(
            (
                Gain::default(),
                Rig::default(),
                Follow::default(),
                Voice::default(),
            ),
            entity,
        );
        let note = AssetId::new();
        let set = |app: &mut App, component: TypeId, path: &[&'static str], value: Value| {
            app.main_mut().world_mut().trigger(SetValue {
                entity,
                component,
                path: PropertyPath::new(path.iter().copied()),
                value,
            });
        };

        set(&mut app, TypeId::of::<Gain>(), &["amount"], json!(2.5));
        set(&mut app, TypeId::of::<Gain>(), &["amount"], json!("loud"));
        set(
            &mut app,
            TypeId::of::<Rig>(),
            &["mode"],
            json!({ "Orbit": { "distance": 4 } }),
        );
        set(
            &mut app,
            TypeId::of::<Rig>(),
            &["mode", "distance"],
            json!(6),
        );
        set(
            &mut app,
            TypeId::of::<Follow>(),
            &["target"],
            json!(target.to_id_string()),
        );
        set(
            &mut app,
            TypeId::of::<Voice>(),
            &["note"],
            json!(note.simple_hex()),
        );

        assert_eq!(results(&app), [true, false, true, true, true, true]);
        assert_eq!(read(&mut app, entity, |gain: &Gain| gain.amount), 2.5);
        assert!(read(&mut app, entity, |rig: &Rig| rig.mode
            == Mode::Orbit { distance: 6.0 }));
        assert_eq!(
            read(&mut app, entity, |follow: &Follow| follow.target),
            SceneEntityRef::Entity(target)
        );
        assert!(read(&mut app, entity, |voice: &Voice| matches!(
            voice.note,
            AssetHandle::Strong(..)
        ) && voice.note.id()
            == note));
        assert!(server.is_loading());
        assert_eq!(written::<SceneEdited>(&app).len(), 5);
    }

    #[test]
    fn an_edit_component_adds_and_removes_a_component_and_a_refused_one_changes_nothing() {
        let Fixture { mut app, entity } = fixture();
        let edit = |app: &mut App, edit: fn(Entity, TypeId) -> ComponentEdit| {
            app.main_mut()
                .world_mut()
                .trigger(EditComponent(edit(entity, TypeId::of::<Gain>())));
        };
        let add = |entity, component| ComponentEdit::Add { entity, component };
        let remove = |entity, component| ComponentEdit::Remove { entity, component };

        edit(&mut app, remove);
        assert!(!dirty(&app));
        edit(&mut app, add);
        assert!(
            app.main_mut()
                .world_mut()
                .get_component_for_entity::<Gain>(entity)
                .is_some()
        );
        assert!(dirty(&app));
        edit(&mut app, remove);
        assert!(
            app.main_mut()
                .world_mut()
                .get_component_for_entity::<Gain>(entity)
                .is_none()
        );

        let outcomes: Vec<bool> = written::<ComponentEdited>(&app)
            .into_iter()
            .map(|outcome| outcome.result.is_ok())
            .collect();
        assert_eq!(outcomes, [false, true, true]);
        assert_eq!(written::<SceneEdited>(&app).len(), 2);
    }
}
