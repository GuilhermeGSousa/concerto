//! The selected entity's name, above its component cards.
use super::*;

use crate::tools::entity_ops::EntityEdit;
use concerto_ecs::{component::name::Name, events::event_reader::EventReader};
use concerto_ui::{
    focus::{FocusedWidget, UIFocusGained, UIFocusLost},
    text_input::{UITextInput, UITextInputCancelled, UITextInputSubmitted},
};

/// The inspector's name field and the entity whose name it is showing.
#[derive(Component, Default)]
pub struct NameField {
    scene_target: Option<Entity>,
}

/// Set to give the name field keyboard focus with its text selected.
#[derive(Resource, Default)]
pub struct FocusNameField(pub bool);

pub(super) fn name_field(theme: &UITheme) -> impl IntoBundle<Bundle: 'static> + use<> {
    (theme.text_field("Name").hidden(), NameField::default())
}

pub(super) fn forget_scene(
    target: Res<LastEditTarget>,
    fields: Query<(&mut NameField, &mut UITextInput, &mut UINode)>,
) {
    if !target.has_changed() {
        return;
    }
    for (mut field, mut input, mut node) in fields.iter() {
        if field.scene_target.is_some() {
            field.scene_target = None;
        }
        if !input.value.is_empty() {
            input.value.clear();
            input.cursor = 0;
            input.selection_anchor = None;
        }
        if node.visible {
            node.visible = false;
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn update_name_field(
    mut submitted: EventReader<UITextInputSubmitted>,
    mut lost: EventReader<UIFocusLost>,
    mut cancelled: EventReader<UITextInputCancelled>,
    mut gained: EventReader<UIFocusGained>,
    data: Res<InspectorData>,
    names: Editor<Query<&Name>>,
    fields: Query<(Entity, &mut NameField, &mut UITextInput, &mut UINode)>,
    mut focused: ResMut<FocusedWidget>,
    mut focus_request: ResMut<FocusNameField>,
    mut scene: EditorSignals,
) {
    let Some((entity, mut field, mut input, mut node)) = fields.iter().next() else {
        return;
    };
    let finished = submitted
        .read()
        .fold(false, |hit, event| hit | (event.entity == entity))
        | lost
            .read()
            .fold(false, |hit, event| hit | (event.0 == entity));
    let cancelled = cancelled
        .read()
        .fold(false, |hit, event| hit | (event.entity == entity));
    let mut select = gained
        .read()
        .fold(false, |hit, event| hit | (event.0 == entity));
    let name_of = |target: Entity| {
        names
            .get_entity(target)
            .map(|name| name.as_str().to_owned())
            .unwrap_or_default()
    };

    if cancelled {
        if **focused == Some(entity) {
            **focused = None;
        }
    } else if finished
        && let Some(target) = field.scene_target
        && input.value.trim() != name_of(target)
    {
        scene.trigger(EntityEdit::Rename {
            entity: target,
            name: input.value.clone(),
        });
    }

    if **focused != Some(entity) {
        if field.scene_target != data.scene_entity {
            field.scene_target = data.scene_entity;
        }
        let name = data.scene_entity.map(name_of).unwrap_or_default();
        if input.value != name {
            input.cursor = name.len();
            input.selection_anchor = None;
            input.value = name;
        }
        let visible = data.scene_entity.is_some();
        if node.visible != visible {
            node.visible = visible;
        }
    }

    if focus_request.0 {
        focus_request.0 = false;
        if node.visible {
            **focused = Some(entity);
            select = true;
        }
    }
    if select {
        input.selection_anchor = Some(0);
        input.cursor = input.value.len();
    }
}
