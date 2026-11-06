use concerto_ecs::command::EntityCommandQueue;
use concerto_ecs::{Component, Entity, Query, Res, ResMut, events::event_reader::EventReader};
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

/// A text field for a `String`, committing the typed text on submit.
#[derive(Default)]
pub(crate) struct TextField;

/// The text field of a [`TextField`] row and the text it last showed.
#[derive(Component)]
pub(crate) struct TextFieldInput {
    row: Entity,
    displayed: String,
}

pub(crate) fn register_defaults(registry: &mut super::registry::InspectorRegistry) {
    registry.register_property_editor::<String, TextField>();
}

impl PropertyEditor<String> for TextField {
    type Snapshot = String;
    type Edit = String;
    fn snapshot(&self, value: &String) -> String {
        value.clone()
    }
    fn apply(&self, value: &mut String, edit: &String, _: &EditContext) -> Result<(), EditError> {
        value.clone_from(edit);
        Ok(())
    }
    fn edit_to(&self, value: &String) -> Option<String> {
        Some(value.clone())
    }
    fn follows_snapshot(&self) -> bool {
        true
    }
    fn build(&self, row: &mut EntityCommandQueue, value: &String, theme: &UITheme) {
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
            TextFieldInput {
                row: row.entity(),
                displayed: value.clone(),
            },
        ));
    }
}

fn revert(field: &TextFieldInput, input: &mut UITextInput) {
    input.value = field.displayed.clone();
    input.cursor = input.value.len();
    input.selection_anchor = None;
}

pub(crate) fn commit_text_fields(
    mut submitted: EventReader<UITextInputSubmitted>,
    mut lost: EventReader<UIFocusLost>,
    rows: Query<&PropertyRow>,
    fields: Query<(&mut TextFieldInput, &mut UITextInput)>,
    mut commits: ResMut<PropertyCommits>,
) {
    for event in submitted.read() {
        let Some((mut field, mut input)) = fields.get_entity(event.entity) else {
            continue;
        };
        let Some(row) = rows.get_entity(field.row) else {
            continue;
        };
        if input.value == field.displayed {
            revert(&field, &mut input);
            continue;
        }
        let text = input.value.clone();
        if let Err(error) = commits.push::<String, TextField>(row, text.clone()) {
            log::warn!("Text edit dropped: {error}");
            revert(&field, &mut input);
            continue;
        }
        field.displayed = text;
    }
    for event in lost.read() {
        let Some((field, mut input)) = fields.get_entity(event.0) else {
            continue;
        };
        revert(&field, &mut input);
    }
}

pub(crate) fn cancel_text_fields(
    mut cancelled: EventReader<UITextInputCancelled>,
    fields: Query<(&TextFieldInput, &mut UITextInput)>,
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

/// Writes each row's text into its field, except the focused one.
pub(crate) fn refresh_text_fields(
    focused: Res<FocusedWidget>,
    rows: Query<&PropertyRowValue>,
    fields: Query<(Entity, &mut TextFieldInput, &mut UITextInput)>,
) {
    for (entity, mut field, mut input) in fields.iter() {
        if **focused == Some(entity) {
            continue;
        }
        let Some(text) = rows
            .get_entity(field.row)
            .and_then(|value| value.downcast::<String>())
        else {
            continue;
        };
        if input.value != *text {
            input.value = text.clone();
            input.cursor = input.value.len();
            input.selection_anchor = None;
        }
        if field.displayed != *text {
            field.displayed = text.clone();
        }
    }
}
