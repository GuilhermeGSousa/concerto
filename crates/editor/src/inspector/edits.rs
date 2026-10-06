use std::any::TypeId;

use concerto_ecs::{Entity, Resource, World};

use super::{registry::InspectorRegistry, rows::EditError};

/// Adds an addable component at its default value, or removes a registered one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComponentEdit {
    Add { entity: Entity, component: TypeId },
    Remove { entity: Entity, component: TypeId },
}

/// Structural edits awaiting the inspector's exclusive applying system.
#[derive(Resource, Default)]
pub struct ComponentEdits(pub Vec<ComponentEdit>);

/// Apply one structural edit to the live world.
pub fn apply_component_edit(world: &mut World, edit: ComponentEdit) -> Result<(), EditError> {
    let (ComponentEdit::Add { entity, component } | ComponentEdit::Remove { entity, component }) =
        edit;
    let registered = world
        .get_resource::<InspectorRegistry>()
        .and_then(|registry| registry.component(component))
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

/// Drain queued structural edits, dropping the ones that no longer apply.
pub fn apply_component_edits(world: &mut World) {
    let Some(edits) = world.get_resource_mut::<ComponentEdits>() else {
        return;
    };
    for edit in std::mem::take(&mut edits.0) {
        let (ComponentEdit::Add { entity, .. } | ComponentEdit::Remove { entity, .. }) = edit;
        match apply_component_edit(world, edit) {
            Ok(()) => crate::asset_editor::mark_entity_edited(world, entity),
            Err(error) => log::warn!("Component edit {edit:?} dropped: {error}"),
        }
    }
}
