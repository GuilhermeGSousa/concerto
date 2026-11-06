use concerto_ecs::command::EntityCommandQueue;
use concerto_ecs::{Component, Entity, Query, Res, ResMut, events::event_reader::EventReader};
use concerto_name::Name;
use concerto_ui::elements::prelude::*;
use concerto_ui::{
    focus::{FocusedWidget, UIFocusLost},
    node::UIRect,
    text_input::{UITextInput, UITextInputCancelled, UITextInputSubmitted},
    theme::UITheme,
    transform::UIValue,
};

use crate::inspector::rows::{
    EditContext, EditError, PropertyCommits, PropertyEditor, PropertyRow, PropertyRowValue,
};

/// A text field for a `Name`, committing the typed text on submit.
#[derive(Default)]
pub(crate) struct NameText;

/// The text field of a [`NameText`] row and the name it last showed.
#[derive(Component)]
pub(crate) struct NameTextField {
    row: Entity,
    displayed: Name,
}

pub(crate) fn register_defaults(registry: &mut super::registry::InspectorRegistry) {
    registry.register_property_editor::<Name, NameText>();
}

impl PropertyEditor<Name> for NameText {
    type Snapshot = Name;
    type Edit = Name;
    fn snapshot(&self, value: &Name) -> Name {
        *value
    }
    fn apply(&self, value: &mut Name, edit: &Name, _: &EditContext) -> Result<(), EditError> {
        *value = *edit;
        Ok(())
    }
    fn edit_to(&self, value: &Name) -> Option<Name> {
        Some(*value)
    }
    fn follows_snapshot(&self) -> bool {
        true
    }
    fn build(&self, row: &mut EntityCommandQueue, value: &Name, theme: &UITheme) {
        row.add_child((
            theme
                .text_field("")
                .small()
                .grow()
                .shrink(1.0)
                .width(UIValue::Px(0.0))
                .min_width(UIValue::Px(0.0))
                .padding(UIRect::axes(
                    super::numeric::field_leading(theme),
                    theme.spacing_xs,
                ))
                .clipped(),
            NameTextField {
                row: row.entity(),
                displayed: *value,
            },
        ));
    }
}

fn revert(field: &NameTextField, input: &mut UITextInput) {
    input.value = field.displayed.as_str().into();
    input.cursor = input.value.len();
    input.selection_anchor = None;
}

pub(crate) fn commit_name_fields(
    mut submitted: EventReader<UITextInputSubmitted>,
    mut lost: EventReader<UIFocusLost>,
    rows: Query<&PropertyRow>,
    fields: Query<(&mut NameTextField, &mut UITextInput)>,
    mut commits: ResMut<PropertyCommits>,
) {
    for event in submitted.read() {
        let Some((mut field, mut input)) = fields.get_entity(event.entity) else {
            continue;
        };
        let Some(row) = rows.get_entity(field.row) else {
            continue;
        };
        if input.value == field.displayed.as_str() {
            revert(&field, &mut input);
            continue;
        }
        let name = Name::new(&input.value);
        if let Err(error) = commits.push::<Name, NameText>(row, name) {
            log::warn!("Name edit dropped: {error}");
            revert(&field, &mut input);
            continue;
        }
        field.displayed = name;
    }
    for event in lost.read() {
        let Some((field, mut input)) = fields.get_entity(event.0) else {
            continue;
        };
        revert(&field, &mut input);
    }
}

pub(crate) fn cancel_name_fields(
    mut cancelled: EventReader<UITextInputCancelled>,
    fields: Query<(&NameTextField, &mut UITextInput)>,
    mut focused: ResMut<FocusedWidget>,
) {
    for event in cancelled.read() {
        let Some((field, mut input)) = fields.get_entity(event.entity) else {
            continue;
        };
        revert(field, &mut input);
        if **focused == Some(event.entity) {
            **focused = None;
        }
    }
}

/// Writes each row's name into its field, except the focused one.
pub(crate) fn refresh_name_fields(
    focused: Res<FocusedWidget>,
    rows: Query<&PropertyRowValue>,
    fields: Query<(Entity, &mut NameTextField, &mut UITextInput)>,
) {
    for (entity, mut field, mut input) in fields.iter() {
        if **focused == Some(entity) {
            continue;
        }
        let Some(name) = rows
            .get_entity(field.row)
            .and_then(|value| value.downcast::<Name>().copied())
        else {
            continue;
        };
        if input.value != name.as_str() {
            input.value = name.as_str().into();
            input.cursor = input.value.len();
            input.selection_anchor = None;
        }
        if field.displayed != name {
            field.displayed = name;
        }
    }
}
