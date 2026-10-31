//! Writing property values and adding or removing components on scene entities.
use std::{
    any::{Any, TypeId},
    sync::Arc,
};

use concerto_app::{App, edit_time::AtEditTime};
use concerto_ecs::{
    Component, Entity, Resource, World,
    component::Tick,
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

use crate::{
    inspector::{ComponentTarget, ErasedEditor, PropertyAddress},
    tools::{
        entity_ops::in_open_scene,
        scene::{SceneRoot, SceneStatus, report_edit, write},
    },
};

/// A rejected edit leaves the target unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditError {
    Rejected,
    TypeMismatch,
    NotFound,
    StaleEditor,
    MissingTarget,
    UnregisteredComponent,
    MissingAssets,
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Rejected => "the value was refused",
            Self::TypeMismatch => "the value has the wrong type",
            Self::NotFound => "the property was not found",
            Self::StaleEditor => "its editor changed since it was read",
            Self::MissingTarget => "its entity, component or document no longer exists",
            Self::UnregisteredComponent => "the component type is not registered",
            Self::MissingAssets => "its App has no asset server or asset registry",
        })
    }
}
impl std::error::Error for EditError {}

/// What an edit can read from the world it changes.
pub struct EditContext<'w> {
    pub assets: &'w AssetServer,
    pub registry: &'w AssetRegistry,
}

/// A row an editor built.
#[derive(Component, Clone)]
pub struct PropertyRow {
    pub address: PropertyAddress,
    pub(crate) editor_type: TypeId,
    pub(crate) registry_tick: Tick,
}

impl PropertyRow {
    /// The registry's change tick at collection, which later edits are checked against.
    pub fn registry_tick(&self) -> Tick {
        self.registry_tick
    }
}

/// One value written into a component, already checked against the widget registry.
pub struct CommitProperty {
    pub row: PropertyRow,
    pub edit: Arc<dyn Any + Send + Sync>,
    pub editor: Arc<dyn ErasedEditor>,
}

impl Signal for CommitProperty {}

/// What one edit of a property did to the row it addressed, or why it was refused.
#[derive(Event, Clone)]
pub struct PropertyCommitted {
    pub address: PropertyAddress,
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
    pub entity: Entity,
    pub component: TypeId,
    pub path: PropertyPath,
    pub value: Value,
}

impl Signal for SetValue {}

impl SetValue {
    /// The address of this write, whose value has type `type_id`.
    pub(crate) fn address(&self, type_id: TypeId) -> PropertyAddress {
        PropertyAddress {
            target: Arc::new(ComponentTarget {
                entity: self.entity,
                component: self.component,
            }),
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
    Add { entity: Entity, component: TypeId },
    Remove { entity: Entity, component: TypeId },
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
    pub edit: ComponentEdit,
    pub result: Result<(), String>,
}

/// Apply one structural edit to the live world.
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
        .and_then(|target| in_open_scene(world, target.entity).map_err(|error| error.to_string()))
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
    use crate::tools::scene::{
        SceneEdited,
        testing::{tools_app, written},
    };
    use crate::{
        inspector::{InspectorRegistry, PropertyEditor},
        tools::scene::SceneStatus,
    };
    use concerto_color::Color;
    use concerto_ecs::{command::EntityCommandQueue, component::scene::SceneEntityRef};
    use concerto_foundation::{
        assets::{
            Asset, AssetId, LoadableAsset, asset_server::AssetServer, asset_store::AssetStore,
            handle::AssetHandle,
        },
        transform::Transform,
    };
    use concerto_reflect::{PathSegment, ReflectDefault};
    use concerto_ui::theme::UITheme;
    use serde_json::{Value, json};

    #[derive(Component, concerto_reflect::Reflect, Default)]
    #[reflect(Component, Default)]
    struct Gain {
        amount: f32,
    }

    #[derive(Component, concerto_reflect::Reflect)]
    #[reflect(Component)]
    struct Lens {
        focal: f32,
    }

    #[derive(Component)]
    struct Plumbing;

    fn fixture() -> (App, Entity) {
        let mut app = tools_app();
        app.register_type::<Transform>()
            .register_type::<Gain>()
            .register_type::<Lens>()
            .insert_resource(AssetServer::new())
            .insert_resource(AssetRegistry::default());
        let entity = app.main_mut().world_mut().spawn(Transform::IDENTITY);
        (app, entity)
    }

    fn add<T: 'static>(entity: Entity) -> ComponentEdit {
        ComponentEdit::Add {
            entity,
            component: TypeId::of::<T>(),
        }
    }

    fn remove<T: 'static>(entity: Entity) -> ComponentEdit {
        ComponentEdit::Remove {
            entity,
            component: TypeId::of::<T>(),
        }
    }

    #[test]
    fn a_component_that_was_never_registered_cannot_be_edited() {
        let mut app = tools_app();
        let world = app.main_mut().world_mut();
        let entity = world.spawn(());

        assert_eq!(
            apply_component_edit(world, add::<Gain>(entity)),
            Err(EditError::UnregisteredComponent)
        );
    }

    #[test]
    fn an_applied_component_edit_writes_scene_edited_and_a_dropped_one_does_not() {
        let (mut app, entity) = fixture();

        app.main_mut()
            .world_mut()
            .trigger(EditComponent(remove::<Gain>(entity)));
        assert!(
            written::<SceneEdited>(&app).is_empty(),
            "a dropped edit changes nothing"
        );

        app.main_mut()
            .world_mut()
            .trigger(EditComponent(add::<Gain>(entity)));
        let world = app.main_mut().world_mut();
        assert!(world.get_component_for_entity::<Gain>(entity).is_some());
        assert_eq!(written::<SceneEdited>(&app).len(), 1);

        app.main_mut()
            .world_mut()
            .trigger(EditComponent(remove::<Gain>(entity)));
        let world = app.main_mut().world_mut();
        assert!(world.get_component_for_entity::<Gain>(entity).is_none());
        assert_eq!(written::<SceneEdited>(&app).len(), 2);
    }

    #[test]
    fn stale_structural_edits_are_dropped() {
        let (mut app, entity) = fixture();
        let world = app.main_mut().world_mut();
        assert_eq!(
            apply_component_edit(world, add::<Transform>(entity)),
            Err(EditError::Rejected)
        );
        assert_eq!(
            apply_component_edit(world, remove::<Gain>(entity)),
            Err(EditError::NotFound)
        );
        assert_eq!(
            apply_component_edit(world, add::<Plumbing>(entity)),
            Err(EditError::UnregisteredComponent)
        );
        let gone = world.spawn(());
        world.despawn(gone);
        for edit in [add::<Gain>(gone), remove::<Gain>(gone)] {
            assert_eq!(
                apply_component_edit(world, edit),
                Err(EditError::MissingTarget)
            );
        }

        world.trigger(EditComponent(add::<Gain>(gone)));
        world.trigger(EditComponent(add::<Gain>(entity)));
        assert!(world.get_component_for_entity::<Gain>(entity).is_some());
        assert_eq!(written::<SceneEdited>(&app).len(), 1);
    }

    #[test]
    fn a_component_without_default_is_removed_but_never_added() {
        let (mut app, entity) = fixture();
        let world = app.main_mut().world_mut();
        assert_eq!(
            apply_component_edit(world, add::<Lens>(entity)),
            Err(EditError::Rejected)
        );

        world.insert(Lens { focal: 35.0 }, entity);
        assert_eq!(apply_component_edit(world, remove::<Lens>(entity)), Ok(()));
        assert!(world.get_component_for_entity::<Lens>(entity).is_none());
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

    fn gain_to(entity: Entity, amount: f32) -> CommitProperty {
        let mut registry = InspectorRegistry::default();
        registry.register_property_editor::<f32, NonNegative>();
        let editor = registry.editor(TypeId::of::<f32>(), None).unwrap();
        CommitProperty {
            row: PropertyRow {
                address: PropertyAddress {
                    target: Arc::new(ComponentTarget {
                        entity,
                        component: TypeId::of::<Gain>(),
                    }),
                    path: PropertyPath::new(["amount"]),
                    type_id: TypeId::of::<f32>(),
                },
                editor_type: editor.editor_type,
                registry_tick: Tick::default(),
            },
            edit: Arc::new(amount),
            editor: Arc::clone(&editor.adapter),
        }
    }

    fn entity_of(outcome: &PropertyCommitted) -> Entity {
        outcome.address.target.component().unwrap().entity
    }

    fn statuses(app: &App) -> Vec<String> {
        written::<SceneStatus>(app)
            .into_iter()
            .map(|status| status.0)
            .collect()
    }

    #[test]
    fn a_refused_component_removal_says_what_was_missing() {
        let (mut app, entity) = fixture();

        app.main_mut()
            .world_mut()
            .trigger(EditComponent(remove::<Gain>(entity)));

        assert_eq!(
            statuses(&app),
            ["Remove Gain failed: the entity does not have it"]
        );
    }

    fn gain(app: &mut App, entity: Entity) -> f32 {
        app.main_mut()
            .world_mut()
            .get_component_for_entity::<Gain>(entity)
            .unwrap()
            .amount
    }

    #[test]
    fn a_committed_property_writes_its_outcome() {
        let (mut app, entity) = fixture();
        app.main_mut().world_mut().insert(Gain::default(), entity);

        app.main_mut().world_mut().trigger(gain_to(entity, 2.0));

        assert_eq!(gain(&mut app, entity), 2.0);
        let outcomes = written::<PropertyCommitted>(&app);
        assert_eq!(outcomes.len(), 1);
        assert_eq!(entity_of(&outcomes[0]), entity);
        assert_eq!(outcomes[0].result, Ok(()));
        assert!(statuses(&app).is_empty());
    }

    #[test]
    fn a_refused_property_commit_writes_its_outcome_and_a_status_with_the_same_text() {
        let (mut app, entity) = fixture();
        app.main_mut().world_mut().insert(Gain::default(), entity);

        app.main_mut().world_mut().trigger(gain_to(entity, -1.0));

        assert_eq!(gain(&mut app, entity), 0.0);
        assert_eq!(
            statuses(&app),
            ["Set Gain.amount failed: the value was refused"]
        );
        let outcomes = written::<PropertyCommitted>(&app);
        assert_eq!(outcomes.len(), 1);
        assert_eq!(entity_of(&outcomes[0]), entity);
        assert_eq!(
            outcomes[0].result,
            Err(PropertyRefusal::Refused(statuses(&app).remove(0)))
        );
        assert!(written::<SceneEdited>(&app).is_empty());
    }

    #[test]
    fn each_component_edit_writes_its_outcome_and_a_refused_one_a_status_with_the_same_text() {
        let (mut app, entity) = fixture();

        for _ in 0..2 {
            app.main_mut()
                .world_mut()
                .trigger(EditComponent(add::<Gain>(entity)));
        }

        let statuses = statuses(&app);
        assert_eq!(
            statuses,
            ["Add Gain failed: it is already there, or has no default value"]
        );
        let outcomes: Vec<_> = written::<ComponentEdited>(&app)
            .into_iter()
            .map(|outcome| (outcome.edit, outcome.result))
            .collect();
        assert_eq!(
            outcomes,
            vec![
                (add::<Gain>(entity), Ok(())),
                (add::<Gain>(entity), Err(statuses[0].clone())),
            ]
        );
    }

    #[derive(Component, concerto_reflect::Reflect, Default)]
    #[reflect(Component, Default)]
    struct Rig {
        optics: Optics,
        mode: Mode,
    }

    #[derive(concerto_reflect::Reflect, Default, Debug, PartialEq)]
    struct Optics {
        focal: f32,
        stops: Vec<f32>,
    }

    #[derive(concerto_reflect::Reflect, Default, Debug, PartialEq)]
    enum Mode {
        #[default]
        Fixed,
        Orbit {
            distance: f32,
            heights: Vec<f32>,
        },
    }

    #[derive(Component, concerto_reflect::Reflect, Default)]
    #[reflect(Component, Default)]
    struct Follow {
        target: SceneEntityRef,
    }

    #[derive(Component, concerto_reflect::Reflect, Default)]
    #[reflect(Component, Default)]
    struct Tint {
        color: Color,
    }

    #[derive(Component, concerto_reflect::Reflect, Default)]
    #[reflect(Component, Default)]
    struct Track {
        modes: Vec<Mode>,
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

    fn values() -> (App, Entity) {
        let (mut app, entity) = fixture();
        app.register_type::<Rig>()
            .register_type::<Follow>()
            .register_type::<Tint>()
            .register_type::<Voice>()
            .register_type::<AssetHandle<Note>>();
        app.main_mut().world_mut().insert(
            (
                Gain::default(),
                Lens { focal: 35.0 },
                Rig::default(),
                Follow::default(),
                Tint::default(),
                Voice::default(),
            ),
            entity,
        );
        (app, entity)
    }

    fn set<C: 'static>(app: &mut App, entity: Entity, path: &[&'static str], value: Value) {
        set_at::<C>(app, entity, PropertyPath::new(path.iter().copied()), value);
    }

    fn set_at<C: 'static>(app: &mut App, entity: Entity, path: PropertyPath, value: Value) {
        app.main_mut().world_mut().trigger(SetValue {
            entity,
            component: TypeId::of::<C>(),
            path,
            value,
        });
    }

    fn read<C: Component, R>(app: &mut App, entity: Entity, read: impl FnOnce(&C) -> R) -> R {
        read(
            app.main_mut()
                .world_mut()
                .get_component_for_entity::<C>(entity)
                .unwrap(),
        )
    }

    fn results(app: &App) -> Vec<Result<(), PropertyRefusal>> {
        written::<PropertyCommitted>(app)
            .into_iter()
            .map(|outcome| outcome.result)
            .collect()
    }

    #[test]
    fn a_set_value_writes_the_field_and_reports_the_row_it_addressed() {
        let (mut app, entity) = values();

        set::<Gain>(&mut app, entity, &["amount"], json!(2.5));
        set::<Lens>(&mut app, entity, &["focal"], json!(50));

        assert_eq!(gain(&mut app, entity), 2.5);
        assert_eq!(read(&mut app, entity, |lens: &Lens| lens.focal), 50.0);
        let outcomes = written::<PropertyCommitted>(&app);
        assert_eq!(outcomes.len(), 2);
        let address = &outcomes[0].address;
        assert_eq!(
            address.target.component(),
            Some(ComponentTarget {
                entity,
                component: TypeId::of::<Gain>(),
            })
        );
        assert_eq!(address.path, PropertyPath::new(["amount"]));
        assert_eq!(address.type_id, TypeId::of::<f32>());
        assert_eq!(results(&app), [Ok(()), Ok(())]);
        assert_eq!(written::<SceneEdited>(&app).len(), 2);
        assert!(statuses(&app).is_empty());
    }

    #[test]
    fn a_set_value_reaches_nested_fields_through_structs_and_variants() {
        let (mut app, entity) = values();

        set::<Rig>(&mut app, entity, &["optics", "focal"], json!(24));
        set::<Rig>(&mut app, entity, &["optics", "stops"], json!([1.4, 2.8]));
        set::<Rig>(
            &mut app,
            entity,
            &["mode"],
            json!({ "Orbit": { "distance": 4 } }),
        );
        set::<Rig>(&mut app, entity, &["mode", "distance"], json!(6));
        set::<Rig>(&mut app, entity, &["mode", "heights"], json!([1, 2]));

        assert_eq!(results(&app), [Ok(()), Ok(()), Ok(()), Ok(()), Ok(())]);
        let (optics, mode) = read(&mut app, entity, |rig: &Rig| {
            (
                Optics {
                    focal: rig.optics.focal,
                    stops: rig.optics.stops.clone(),
                },
                match &rig.mode {
                    Mode::Fixed => Mode::Fixed,
                    Mode::Orbit { distance, heights } => Mode::Orbit {
                        distance: *distance,
                        heights: heights.clone(),
                    },
                },
            )
        });
        assert_eq!(
            optics,
            Optics {
                focal: 24.0,
                stops: vec![1.4, 2.8],
            }
        );
        assert_eq!(
            mode,
            Mode::Orbit {
                distance: 6.0,
                heights: vec![1.0, 2.0],
            }
        );
    }

    #[test]
    fn a_set_value_writes_one_element_of_a_list() {
        let (mut app, entity) = values();
        set::<Rig>(&mut app, entity, &["optics", "stops"], json!([1.4, 2.8]));
        let element = PropertyPath::new([
            PathSegment::Field("optics"),
            PathSegment::Field("stops"),
            PathSegment::Index(1),
        ]);

        set_at::<Rig>(&mut app, entity, element.clone(), json!(4));

        assert_eq!(results(&app), [Ok(()), Ok(())]);
        assert_eq!(
            read(&mut app, entity, |rig: &Rig| rig.optics.stops.clone()),
            [1.4, 4.0]
        );
        let outcomes = written::<PropertyCommitted>(&app);
        assert_eq!(outcomes[1].address.path, element);
        assert_eq!(outcomes[1].address.type_id, TypeId::of::<f32>());
    }

    #[test]
    fn a_set_value_under_a_list_refuses_a_type_with_no_json_and_no_default() {
        let (mut app, entity) = values();
        app.register_type::<Track>();
        app.main_mut().world_mut().insert(
            Track {
                modes: vec![Mode::Fixed],
            },
            entity,
        );

        set_at::<Track>(
            &mut app,
            entity,
            PropertyPath::new([PathSegment::Field("modes"), PathSegment::Index(0)]),
            json!({ "Orbit": { "distance": 4 } }),
        );

        let refused = format!(
            "Set Track.modes.0 failed: {} has no JSON form and no registered Default, so JSON cannot build it inside a list",
            std::any::type_name::<Mode>()
        );
        assert_eq!(
            results(&app),
            [Err(PropertyRefusal::NotEditable(refused.clone()))]
        );
        assert_eq!(statuses(&app), [refused]);
        assert!(read(&mut app, entity, |track: &Track| matches!(
            track.modes[0],
            Mode::Fixed
        )));
        assert!(written::<SceneEdited>(&app).is_empty());
    }

    #[test]
    fn a_set_value_with_an_empty_path_merges_into_the_whole_component() {
        let (mut app, entity) = values();
        set::<Rig>(
            &mut app,
            entity,
            &["mode"],
            json!({ "Orbit": { "distance": 4 } }),
        );

        set::<Rig>(&mut app, entity, &[], json!({ "optics": { "focal": 85 } }));

        assert_eq!(results(&app), [Ok(()), Ok(())]);
        assert_eq!(read(&mut app, entity, |rig: &Rig| rig.optics.focal), 85.0);
        assert!(read(&mut app, entity, |rig: &Rig| matches!(
            rig.mode,
            Mode::Orbit { distance: 4.0, .. }
        )));
    }

    #[test]
    fn an_invalid_value_is_refused_and_leaves_the_component_unchanged() {
        let (mut app, entity) = values();
        set::<Gain>(&mut app, entity, &["amount"], json!(1.0));

        set::<Gain>(&mut app, entity, &["amount"], json!("loud"));
        set::<Rig>(&mut app, entity, &["mode"], json!("Spinning"));

        assert_eq!(gain(&mut app, entity), 1.0);
        assert!(read(&mut app, entity, |rig: &Rig| matches!(
            rig.mode,
            Mode::Fixed
        )));
        let refused = [
            "Set Gain.amount failed: expected a number, or \"NaN\", \"inf\" or \"-inf\"",
            "Set Rig.mode failed: mode: no variant 'Spinning'; the variants are Fixed, Orbit",
        ];
        assert_eq!(statuses(&app), refused);
        assert_eq!(
            results(&app),
            [
                Ok(()),
                Err(PropertyRefusal::Undecodable(refused[0].to_owned())),
                Err(PropertyRefusal::Undecodable(refused[1].to_owned()))
            ]
        );
        assert_eq!(written::<SceneEdited>(&app).len(), 1);
    }

    #[test]
    fn the_scene_root_is_refused() {
        let (mut app, _) = values();
        let world = app.main_mut().world_mut();
        let root = world.spawn((Gain::default(), SceneRoot));

        set::<Gain>(&mut app, root, &["amount"], json!(3.0));

        assert_eq!(gain(&mut app, root), 0.0);
        assert_eq!(
            results(&app),
            [Err(PropertyRefusal::Refused(
                "Set Gain.amount failed: the scene root cannot be changed".to_owned()
            ))]
        );
    }

    #[test]
    fn every_listener_refuses_an_entity_outside_the_open_scene() {
        let (mut app, outsider) = values();
        let world = app.main_mut().world_mut();
        let root = world.spawn((Transform::IDENTITY, SceneRoot));
        let member = world.spawn((Transform::IDENTITY, Gain::default()));
        world.entity_mut(root).add_child(member);

        set::<Gain>(&mut app, outsider, &["amount"], json!(3.0));
        app.main_mut().world_mut().trigger(gain_to(outsider, 2.0));
        app.main_mut()
            .world_mut()
            .trigger(EditComponent(remove::<Lens>(outsider)));
        set::<Gain>(&mut app, member, &["amount"], json!(1.0));
        app.main_mut().world_mut().trigger(gain_to(member, 2.0));
        app.main_mut()
            .world_mut()
            .trigger(EditComponent(add::<Rig>(member)));

        assert_eq!(gain(&mut app, outsider), 0.0);
        assert_eq!(read(&mut app, outsider, |lens: &Lens| lens.focal), 35.0);
        assert_eq!(gain(&mut app, member), 2.0);
        assert!(
            app.main_mut()
                .world_mut()
                .get_component_for_entity::<Rig>(member)
                .is_some()
        );
        assert_eq!(
            statuses(&app),
            [
                "Set Gain.amount failed: the entity is not part of an open scene",
                "Set Gain.amount failed: the entity is not part of an open scene",
                "Remove Lens failed: the entity is not part of an open scene",
            ]
        );
        assert_eq!(written::<SceneEdited>(&app).len(), 3);
    }

    #[test]
    fn an_unknown_path_a_missing_component_and_a_missing_entity_are_refused() {
        let (mut app, entity) = values();
        let world = app.main_mut().world_mut();
        let bare = world.spawn(Transform::IDENTITY);
        let gone = world.spawn(Gain::default());
        world.despawn(gone);

        set::<Gain>(&mut app, entity, &["volume"], json!(1.0));
        set::<Gain>(&mut app, bare, &["amount"], json!(1.0));
        set::<Gain>(&mut app, gone, &["amount"], json!(1.0));
        set::<Plumbing>(&mut app, entity, &[], json!(null));

        assert_eq!(
            statuses(&app),
            [
                "Set Gain.volume failed: the property was not found",
                "Set Gain.amount failed: its entity, component or document no longer exists",
                "Set Gain.amount failed: its entity, component or document no longer exists",
                "Set the component failed: the component type is not registered",
            ]
        );
        assert_eq!(gain(&mut app, entity), 0.0);
        assert!(written::<SceneEdited>(&app).is_empty());
    }

    #[test]
    fn an_entity_reference_is_written_as_the_live_entity_it_names() {
        let (mut app, entity) = values();
        let target = app.main_mut().world_mut().spawn(());

        set::<Follow>(&mut app, entity, &["target"], json!(target.to_id_string()));

        assert_eq!(results(&app), [Ok(())]);
        assert_eq!(
            read(&mut app, entity, |follow: &Follow| follow.target),
            SceneEntityRef::Entity(target)
        );
    }

    #[test]
    fn an_asset_handle_is_written_strong_and_requested_from_the_asset_server() {
        let (mut app, entity) = values();
        let mut server = AssetServer::new();
        server.register_asset(&AssetStore::<Note>::new());
        app.insert_resource(server.clone());
        let id = AssetId::new();

        set::<Voice>(&mut app, entity, &["note"], json!(id.simple_hex()));

        assert_eq!(results(&app), [Ok(())]);
        let (strong, written) = read(&mut app, entity, |voice: &Voice| {
            (
                matches!(voice.note, AssetHandle::Strong(..)),
                voice.note.id(),
            )
        });
        assert!(strong, "a handle set from JSON keeps its asset alive");
        assert_eq!(written, id);
        assert!(server.is_loading());
    }

    #[test]
    fn a_color_is_written_from_its_components() {
        let (mut app, entity) = values();

        set::<Tint>(&mut app, entity, &["color"], json!([0.5, 0.25, 1.0, 1.0]));

        assert_eq!(results(&app), [Ok(())]);
        assert_eq!(
            read(&mut app, entity, |tint: &Tint| tint.color),
            Color::rgba(0.5, 0.25, 1.0, 1.0)
        );
    }
}
