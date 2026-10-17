use concerto_ecs::{Component, Entity, Query, ResMut, command::CommandQueue, signal::On};
use concerto_ui::elements::prelude::*;
use concerto_ui::{
    checkbox::{UICheckbox, UICheckboxChanged},
    node::UIRect,
    theme::UITheme,
    transform::UIValue,
};

use crate::inspector::rows::{
    EditError, PropertyCommits, PropertyEditor, PropertyRow, PropertyRowValue,
};

const BOX_SIZE: f32 = 16.0;

/// A checkbox for a `bool`.
pub(crate) struct BoolCheckbox;

/// The checkbox of a [`BoolCheckbox`] row.
#[derive(Component)]
pub(crate) struct BoolField {
    row: Entity,
}

pub(crate) fn register_defaults(registry: &mut super::registry::InspectorRegistry) {
    registry.register_property_editor::<bool, _>(BoolCheckbox);
}

impl PropertyEditor<bool> for BoolCheckbox {
    type Snapshot = bool;
    type Edit = bool;
    fn snapshot(&self, value: &bool) -> bool {
        *value
    }
    fn apply(&self, value: &mut bool, edit: &bool) -> Result<(), EditError> {
        *value = *edit;
        Ok(())
    }
    fn follows_snapshot(&self) -> bool {
        true
    }
    fn build(&self, cmd: &mut CommandQueue, row: Entity, value: &bool, theme: &UITheme) {
        let margin = ((theme.control_height - BOX_SIZE) / 2.0).max(0.0);
        cmd.entity(row).add_child((
            theme
                .checkbox("", *value)
                .size(UIValue::Px(BOX_SIZE), UIValue::Px(BOX_SIZE))
                .padding(0.0)
                .margin(UIRect::axes(margin, 0.0))
                .radius_sm()
                .fixed()
                .on_change(commit_bool_field),
            BoolField { row },
        ));
    }
}

fn commit_bool_field(
    on: On<UICheckboxChanged>,
    fields: Query<&BoolField>,
    rows: Query<&PropertyRow>,
    mut commits: ResMut<PropertyCommits>,
) {
    let Some(row) = fields
        .get_entity(on.entity())
        .and_then(|field| rows.get_entity(field.row))
    else {
        return;
    };
    if let Err(error) = commits.push::<bool, BoolCheckbox>(row, on.signal().checked) {
        log::warn!("Bool edit dropped: {error}");
    }
}

/// Writes each row's value into its checkbox.
pub(crate) fn refresh_bool_fields(
    rows: Query<&PropertyRowValue>,
    fields: Query<(&BoolField, &mut UICheckbox)>,
) {
    for (field, mut checkbox) in fields.iter() {
        let Some(value) = rows
            .get_entity(field.row)
            .and_then(|value| value.downcast::<bool>().copied())
        else {
            continue;
        };
        if checkbox.checked != value {
            checkbox.checked = value;
        }
    }
}
