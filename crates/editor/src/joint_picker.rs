use concerto_ecs::{
    Component, Entity, Query, Res, ResMut, With,
    command::{CommandQueue, EntityCommandQueue},
    signal::{
        On,
        listener::{IntoListener, Listener},
    },
};
use concerto_mesh::JointName;
use concerto_name::Name;
use concerto_ui::elements::prelude::*;
use concerto_ui::{
    anchor::UIAnchoredPanel,
    interaction::{UIClick, UIDisabled},
    text::UIText,
    theme::UITheme,
};
use concerto_window::input::MouseButton;

use crate::{
    asset_editor::ActiveEditor,
    asset_picker::{picker_button, picker_caret, picker_label},
    inspector::{
        EditContext, EditError, InspectorRegistry, PropertyCommits, PropertyEditor, PropertyRow,
        PropertyRowValue,
    },
    name_menu::{NameMenu, close_menu, open_name_menu},
};

const NO_NAME: &str = "None";

/// A button naming the joint, which opens a menu of the skeleton's bones.
#[derive(Default)]
pub struct JointNameEditor;

/// The bones of the skeleton a document's value names, or `None` while it names none that can be read.
#[derive(Component, Debug, Default, PartialEq)]
pub struct DocumentBones(pub Option<Vec<Name>>);

#[derive(Component)]
pub(crate) struct JointNameLabel {
    row: Entity,
    button: Entity,
}

pub(crate) fn register_defaults(registry: &mut InspectorRegistry) {
    registry.register_property_editor::<JointName, JointNameEditor>();
}

impl PropertyEditor<JointName> for JointNameEditor {
    type Snapshot = JointName;
    type Edit = JointName;

    fn snapshot(&self, value: &JointName) -> JointName {
        *value
    }

    fn build(&self, row: &mut EntityCommandQueue, _: &JointName, theme: &UITheme) {
        let entity = row.entity();
        row.add_child_with(
            picker_button(theme).on_click(toggle_joint_menu(entity)),
            |mut button| {
                let label = JointNameLabel {
                    row: entity,
                    button: button.entity(),
                };
                button
                    .add_child((picker_label(theme), label))
                    .add_child(picker_caret(theme));
            },
        );
    }

    fn apply(
        &self,
        value: &mut JointName,
        edit: &JointName,
        _: &EditContext,
    ) -> Result<(), EditError> {
        *value = *edit;
        Ok(())
    }

    fn edit_to(&self, value: &JointName) -> Option<JointName> {
        Some(*value)
    }

    fn follows_snapshot(&self) -> bool {
        true
    }
}

fn joint_of(values: &Query<&PropertyRowValue>, row: Entity) -> Option<JointName> {
    values
        .get_entity(row)?
        .snapshot::<JointName, JointNameEditor>()
        .copied()
}

fn front_bones<'a>(
    active: &ActiveEditor,
    documents: &'a Query<&DocumentBones>,
) -> Option<&'a [Name]> {
    documents.get_entity(active.0?)?.0.as_deref()
}

fn toggle_joint_menu(row: Entity) -> Listener<UIClick> {
    (move |on: On<UIClick>,
           active: Res<ActiveEditor>,
           documents: Query<&DocumentBones>,
           values: Query<&PropertyRowValue>,
           menus: Query<(Entity, &mut UIAnchoredPanel), With<NameMenu>>,
           theme: Res<UITheme>,
           mut cmd: CommandQueue| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        let Some(bones) = front_bones(&active, &documents) else {
            return;
        };
        let current = joint_of(&values, row);
        open_name_menu(
            on.entity(),
            bones,
            current.as_deref(),
            |name| pick_joint(row, name.into()),
            &menus,
            &theme,
            &mut cmd,
        );
    })
    .into_listener()
}

fn pick_joint(row: Entity, name: JointName) -> Listener<UIClick> {
    (move |on: On<UIClick>,
           menus: Query<&mut UIAnchoredPanel, With<NameMenu>>,
           rows: Query<&PropertyRow>,
           values: Query<&PropertyRowValue>,
           mut commits: ResMut<PropertyCommits>| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        close_menu(&menus);
        let Some(target) = rows.get_entity(row) else {
            return;
        };
        if joint_of(&values, row).as_ref() == Some(&name) {
            return;
        }
        if let Err(error) = commits.push::<JointName, JointNameEditor>(target, name) {
            log::warn!("Joint pick dropped: {error}");
        }
    })
    .into_listener()
}

pub(crate) fn refresh_joint_rows(
    active: Res<ActiveEditor>,
    documents: Query<&DocumentBones>,
    values: Query<&PropertyRowValue>,
    labels: Query<(&JointNameLabel, &mut UIText)>,
    disabled: Query<&UIDisabled>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    let bones = front_bones(&active, &documents);
    for (label, mut text) in labels.iter() {
        let Some(name) = joint_of(&values, label.row) else {
            continue;
        };
        let shown = if name.is_empty() { NO_NAME } else { &name };
        if text.text != shown {
            text.text = shown.to_owned();
        }
        let color = match bones {
            None => theme.text_muted,
            Some(bones) if bones.contains(&name.0) => theme.text,
            Some(_) => theme.warning,
        };
        if text.color != color {
            text.color = color;
        }
        match (bones.is_none(), disabled.get_entity(label.button).is_some()) {
            (true, false) => cmd.insert(UIDisabled, label.button),
            (false, true) => cmd.remove::<UIDisabled>(label.button),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests;
