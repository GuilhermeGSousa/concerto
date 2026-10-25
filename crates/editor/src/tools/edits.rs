//! Writing property values and adding or removing components on scene entities.
use std::{
    any::{Any, TypeId},
    sync::Arc,
};

use concerto_app::{App, edit_time::AtEditTime};
use concerto_ecs::{
    Component, Entity, Resource, World,
    component::Tick,
    reflect::{AppTypeRegistry, ReflectComponent},
    signal::{On, Signal},
};
use concerto_foundation::assets::{asset_server::AssetServer, registry::AssetRegistry};
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
    MissingAssets,
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for EditError {}

/// What an edit can read from the world it changes.
pub struct EditContext<'w> {
    pub assets: &'w AssetServer,
    pub registry: &'w AssetRegistry,
}

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
        let component = reflected(world, self.row.component)?;
        let mut result = Err(EditError::MissingAssets);
        world.resource_scope::<AssetServer>(|world, assets| {
            world.resource_scope::<AssetRegistry>(|world, registry| {
                result = apply_through(
                    world,
                    component,
                    self.row.entity,
                    &self.row.path,
                    self.editor.as_ref(),
                    self.edit.as_ref(),
                    &EditContext { assets, registry },
                );
            });
        });
        result
    }
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
        if with_property_mut(value, path, &mut |value| {
            result = editor.apply(value, edit, context);
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
    use concerto_reflect::ReflectDefault;

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
}
