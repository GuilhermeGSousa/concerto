//! Writing property values and adding or removing components on scene entities.
use std::{
    any::{Any, TypeId},
    sync::Arc,
};

use concerto_app::{
    App,
    edit_time::AtEditTime,
    reflect::{ReflectComponent, ReflectComponents},
};
use concerto_ecs::{
    Component, Entity, Resource, World,
    component::Tick,
    signal::{On, Signal},
};
use concerto_reflect::{PropertyPath, with_property_mut};

use crate::{inspector::ErasedEditor, tools::scene::report_edit};

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

impl CommitProperty {
    /// Writes the value into the component the row addresses.
    pub fn apply(&self, world: &mut World) -> Result<(), EditError> {
        let component = world
            .get_resource::<ReflectComponents>()
            .and_then(|table| table.get(self.row.component))
            .ok_or(EditError::UnregisteredComponent)?;
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

/// Apply one structural edit to the live world.
pub fn apply_component_edit(world: &mut World, edit: ComponentEdit) -> Result<(), EditError> {
    let (ComponentEdit::Add { entity, component } | ComponentEdit::Remove { entity, component }) =
        edit;
    let registered = world
        .get_resource::<ReflectComponents>()
        .and_then(|table| table.get(component))
        .ok_or(EditError::UnregisteredComponent)?;
    if !world.entity_is_valid(entity) {
        return Err(EditError::MissingTarget);
    }
    let present = world.component_ids(entity).contains(&component);
    match edit {
        ComponentEdit::Add { .. } if present => Err(EditError::Rejected),
        ComponentEdit::Remove { .. } if !present => Err(EditError::NotFound),
        ComponentEdit::Add { .. } => {
            let insert = registered.insert.ok_or(EditError::Rejected)?;
            insert(world, entity);
            Ok(())
        }
        ComponentEdit::Remove { .. } => {
            (registered.remove)(world, entity);
            Ok(())
        }
    }
}

pub(crate) fn commit_property(on: On<CommitProperty>, world: &mut World) {
    match on.signal().apply(world) {
        Ok(()) => report_edit(world),
        Err(error) => log::warn!("Property commit dropped: {error}"),
    }
}

pub(crate) fn edit_component(on: On<EditComponent>, world: &mut World) {
    match apply_component_edit(world, on.signal().0) {
        Ok(()) => report_edit(world),
        Err(error) => log::warn!("Component edit {:?} dropped: {error}", on.signal().0),
    }
}

pub(crate) fn build(app: &mut App) {
    let world = app.main_mut().world_mut();
    if world.get_resource::<ReflectComponents>().is_none() {
        world.insert_resource(ReflectComponents::default());
    }
    app.add_listener(commit_property.at_edit_time())
        .add_listener(edit_component.at_edit_time());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::scene::{
        SceneEdited,
        testing::{tools_app, written},
    };
    use concerto_foundation::transform::Transform;

    #[derive(Component, concerto_reflect::Reflect, Default)]
    struct Gain {
        amount: f32,
    }

    #[derive(Component, concerto_reflect::Reflect)]
    struct Lens {
        focal: f32,
    }

    #[derive(Component)]
    struct Plumbing;

    fn fixture() -> (App, Entity) {
        let mut app = tools_app();
        app.register_reflect::<Transform>()
            .register_reflect::<Gain>()
            .register_reflect_readonly::<Lens>();
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
    fn the_tools_bring_an_empty_table_when_no_plugin_registered_a_component() {
        let mut app = tools_app();
        assert!(
            app.get_resource::<ReflectComponents>()
                .is_some_and(|table| table.iter().next().is_none())
        );
        let world = app.main_mut().world_mut();
        let entity = world.spawn(());

        assert_eq!(
            apply_component_edit(world, add::<Gain>(entity)),
            Err(EditError::UnregisteredComponent)
        );
    }

    #[test]
    fn a_table_a_plugin_already_filled_is_kept() {
        let mut app = App::new();
        app.register_plugin(concerto_app::main_schedule::MainSchedulePlugin)
            .register_plugin(concerto_app::plugins::TimePlugin)
            .register_reflect::<Gain>();

        crate::tools::build_core(&mut app);

        let table = app.get_resource::<ReflectComponents>().unwrap();
        assert!(table.get(TypeId::of::<Gain>()).is_some());
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
    fn a_readonly_component_is_removed_but_never_added() {
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
}
