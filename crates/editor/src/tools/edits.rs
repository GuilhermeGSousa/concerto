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
};
use concerto_reflect::{PropertyPath, with_property_mut};

use crate::{
    inspector::ErasedEditor,
    tools::scene::{SceneStatus, report_edit, write},
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
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for EditError {}

/// Captured at rebuild, so a queued edit always addresses its original target.
#[derive(Component, Clone, Debug)]
pub struct PropertyRow {
    pub entity: Entity,
    pub component: TypeId,
    pub path: PropertyPath,
    pub type_id: TypeId,
    pub(crate) editor_type: Option<TypeId>,
    pub(crate) registry_tick: Tick,
}

impl PropertyRow {
    /// Whether an adapter claimed this row when it was collected.
    pub fn has_editor(&self) -> bool {
        self.editor_type.is_some()
    }

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

/// What one `CommitProperty` did to the row it addressed, or the status text saying why it was refused.
#[derive(Event, Clone)]
pub struct PropertyCommitted {
    pub row: PropertyRow,
    pub result: Result<(), String>,
}

impl CommitProperty {
    /// Writes the value into the component the row addresses.
    pub fn apply(&self, world: &mut World) -> Result<(), EditError> {
        let component = reflected(world, self.row.component)?;
        apply_through(
            world,
            component,
            self.row.entity,
            &self.row.path,
            self.editor.as_ref(),
            self.edit.as_ref(),
        )
    }
}

fn apply_through(
    world: &mut World,
    component: ReflectComponent,
    entity: Entity,
    path: &PropertyPath,
    editor: &dyn ErasedEditor,
    edit: &dyn Any,
) -> Result<(), EditError> {
    let mut result = Err(EditError::NotFound);
    let found = (component.edit)(world, entity, &mut |value| {
        if with_property_mut(value, path, &mut |value| {
            result = editor.apply(value, edit);
        })
        .is_err()
        {
            result = Err(EditError::NotFound);
        }
    });
    if found {
        result
    } else {
        Err(EditError::MissingTarget)
    }
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
        .copied()
        .ok_or(EditError::UnregisteredComponent)
}

pub(crate) fn commit_property(on: On<CommitProperty>, world: &mut World) {
    let result = on
        .signal()
        .apply(world)
        .map_err(|error| format!("Property commit dropped: {error}"));
    report(world, &result);
    write(
        world,
        PropertyCommitted {
            row: on.signal().row.clone(),
            result,
        },
    );
}

pub(crate) fn edit_component(on: On<EditComponent>, world: &mut World) {
    let edit = on.signal().0;
    let result = apply_component_edit(world, edit)
        .map_err(|error| format!("Component edit {edit:?} dropped: {error}"));
    report(world, &result);
    write(world, ComponentEdited { edit, result });
}

fn report(world: &mut World, result: &Result<(), String>) {
    match result {
        Ok(()) => report_edit(world),
        Err(status) => write(world, SceneStatus(status.clone())),
    }
}

pub(crate) fn build(app: &mut App) {
    app.register_event::<PropertyCommitted>()
        .register_event::<ComponentEdited>()
        .add_listener(commit_property.at_edit_time())
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
    use concerto_ecs::command::CommandQueue;
    use concerto_foundation::transform::Transform;
    use concerto_reflect::ReflectDefault;
    use concerto_ui::theme::UITheme;

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
            .register_type::<Lens>();
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

    struct NonNegative;

    impl PropertyEditor<f32> for NonNegative {
        type Snapshot = f32;
        type Edit = f32;

        fn snapshot(&self, value: &f32) -> f32 {
            *value
        }

        fn build(&self, _: &mut CommandQueue, _: Entity, _: &f32, _: &UITheme) {}

        fn apply(&self, value: &mut f32, edit: &f32) -> Result<(), EditError> {
            if *edit < 0.0 {
                return Err(EditError::Rejected);
            }
            *value = *edit;
            Ok(())
        }
    }

    fn gain_to(entity: Entity, amount: f32) -> CommitProperty {
        let mut registry = InspectorRegistry::default();
        registry.register_property_editor::<f32, _>(NonNegative);
        let editor = registry.editor(TypeId::of::<f32>()).unwrap();
        CommitProperty {
            row: PropertyRow {
                entity,
                component: TypeId::of::<Gain>(),
                path: PropertyPath::new(["amount"]),
                type_id: TypeId::of::<f32>(),
                editor_type: Some(editor.editor_type),
                registry_tick: Tick::default(),
            },
            edit: Arc::new(amount),
            editor: Arc::clone(&editor.adapter),
        }
    }

    fn statuses(app: &App) -> Vec<String> {
        written::<SceneStatus>(app)
            .into_iter()
            .map(|status| status.0)
            .collect()
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
        assert_eq!(outcomes[0].row.entity, entity);
        assert_eq!(outcomes[0].result, Ok(()));
        assert!(statuses(&app).is_empty());
    }

    #[test]
    fn a_refused_property_commit_writes_its_outcome_and_a_status_with_the_same_text() {
        let (mut app, entity) = fixture();
        app.main_mut().world_mut().insert(Gain::default(), entity);

        app.main_mut().world_mut().trigger(gain_to(entity, -1.0));

        assert_eq!(gain(&mut app, entity), 0.0);
        assert_eq!(statuses(&app), ["Property commit dropped: Rejected"]);
        let outcomes = written::<PropertyCommitted>(&app);
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].row.entity, entity);
        assert_eq!(outcomes[0].result, Err(statuses(&app).remove(0)));
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
        assert_eq!(statuses.len(), 1);
        assert!(
            statuses[0].starts_with("Component edit Add"),
            "{}",
            statuses[0]
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
}
