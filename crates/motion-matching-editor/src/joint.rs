use concerto_ecs::{
    Component, Entity, Query, Res, ResMut, With,
    command::{CommandQueue, EntityCommandQueue},
    signal::{
        On,
        listener::{IntoListener, Listener},
    },
};
use concerto_editor::{
    asset_picker::{picker_button, picker_caret, picker_label},
    hosted::EditWorld,
    inspector::{
        EditContext, EditError, PropertyCommits, PropertyEditor, PropertyRow, PropertyRowValue,
    },
};
use concerto_mesh::JointName;
use concerto_ui::elements::prelude::*;
use concerto_ui::{
    anchor::UIAnchoredPanel,
    interaction::{UIClick, UIDisabled},
    text::UIText,
    theme::UITheme,
};
use concerto_window::input::MouseButton;

use crate::{
    menu::{NameMenu, close_menu, open_name_menu},
    preview::SkeletonBones,
};

const NO_NAME: &str = "None";

/// A button naming the joint, which opens a menu of the skeleton's bones.
#[derive(Default)]
pub struct JointNameEditor;

#[derive(Component)]
pub(crate) struct JointNameLabel {
    row: Entity,
    button: Entity,
}

impl PropertyEditor<JointName> for JointNameEditor {
    type Snapshot = JointName;
    type Edit = JointName;

    fn snapshot(&self, value: &JointName) -> JointName {
        value.clone()
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
        *value = edit.clone();
        Ok(())
    }

    fn edit_to(&self, value: &JointName) -> Option<JointName> {
        Some(value.clone())
    }

    fn follows_snapshot(&self) -> bool {
        true
    }
}

fn joint_of(values: &Query<&PropertyRowValue>, row: Entity) -> Option<JointName> {
    values
        .get_entity(row)?
        .snapshot::<JointName, JointNameEditor>()
        .cloned()
}

fn toggle_joint_menu(row: Entity) -> Listener<UIClick> {
    (move |on: On<UIClick>,
           bones: EditWorld<Option<Res<SkeletonBones>>>,
           values: Query<&PropertyRowValue>,
           menus: Query<(Entity, &mut UIAnchoredPanel), With<NameMenu>>,
           theme: Res<UITheme>,
           mut cmd: CommandQueue| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        let Some(bones) = bones.as_deref() else {
            return;
        };
        let current = joint_of(&values, row);
        open_name_menu(
            on.entity(),
            &bones.0,
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
           mut commits: ResMut<PropertyCommits>| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        close_menu(&menus);
        let Some(target) = rows.get_entity(row) else {
            return;
        };
        if let Err(error) = commits.push::<JointName, JointNameEditor>(target, name.clone()) {
            log::warn!("Joint pick dropped: {error}");
        }
    })
    .into_listener()
}

pub(crate) fn refresh_joint_rows(
    bones: EditWorld<Option<Res<SkeletonBones>>>,
    values: Query<&PropertyRowValue>,
    labels: Query<(&JointNameLabel, &mut UIText)>,
    disabled: Query<&UIDisabled>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    let bones = bones.as_deref();
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
            Some(bones) if bones.0.iter().any(|bone| *bone == *name) => theme.text,
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
pub(crate) mod tests;
