use std::{any::Any, sync::Arc};

use concerto_ecs::{
    Component, Entity, Query, Res, ResMut,
    command::CommandQueue,
    signal::{
        On,
        lifecycle::Remove,
        listener::{IntoListener, Listener},
    },
};
use concerto_reflect::{Reflect, ReflectMut, ReflectRef};
use concerto_ui::elements::prelude::*;
use concerto_ui::{
    anchor::{UIAnchorTarget, UIAnchoredPanel},
    checkbox::{UICheckbox, UICheckboxChanged},
    interaction::UIClick,
    node::UIRect,
    text::UIText,
    theme::UITheme,
    transform::UIValue,
};
use concerto_window::input::MouseButton;

use super::{
    EditContext, EditError, ErasedEditor, MENU_FILL, PropertyCommit, PropertyCommits, PropertyRow,
    PropertyRowValue,
};
use crate::fonts::{glyph, icon};
use crate::hosted::EditTarget;

const MENU_WIDTH: f32 = 160.0;
const ITEM_HEIGHT: f32 = 26.0;
const BOX_SIZE: f32 = 16.0;

/// Picks the variant of a reflected enum from its variant names; the active variant's fields follow as rows of their own.
pub(crate) struct VariantPicker {
    variants: &'static [&'static str],
}

impl VariantPicker {
    pub(crate) fn new(variants: &'static [&'static str]) -> Self {
        Self { variants }
    }
}

/// Turns a reflected `Option` on and off; the value's fields follow as rows of their own.
pub(crate) struct OptionToggle;

/// The text inside a [`VariantPicker`] row's button, naming the row's active variant.
#[derive(Component)]
pub(crate) struct VariantPickerLabel {
    row: Entity,
}

/// The menu every variant picker shares, pointed at the row whose button opened it last.
#[derive(Component)]
pub(crate) struct VariantMenu {
    row: Option<Entity>,
}

/// The checkbox of an [`OptionToggle`] row.
#[derive(Component)]
pub(crate) struct OptionField {
    row: Entity,
}

fn active_variant(value: &dyn Reflect) -> Result<&'static str, EditError> {
    let ReflectRef::Enum(value) = value.reflect_ref() else {
        return Err(EditError::TypeMismatch);
    };
    Ok(value.variant_name())
}

fn switch_variant(value: &mut dyn Reflect, edit: &dyn Any) -> Result<(), EditError> {
    let name = *edit
        .downcast_ref::<&'static str>()
        .ok_or(EditError::TypeMismatch)?;
    let ReflectMut::Enum(value) = value.reflect_mut() else {
        return Err(EditError::TypeMismatch);
    };
    if value.variant_name() == name || value.set_variant(name) {
        Ok(())
    } else {
        Err(EditError::Rejected)
    }
}

impl ErasedEditor for VariantPicker {
    fn snapshot(&self, value: &dyn Reflect) -> Result<PropertyRowValue, EditError> {
        Ok(PropertyRowValue::erased::<Self, &'static str>(
            (value as &dyn Any).type_id(),
            active_variant(value)?,
        ))
    }

    fn build(
        &self,
        cmd: &mut CommandQueue,
        row: Entity,
        snapshot: &PropertyRowValue,
        theme: &UITheme,
    ) -> Result<(), EditError> {
        let name = *snapshot
            .downcast::<&'static str>()
            .ok_or(EditError::TypeMismatch)?;
        cmd.entity(row).add_child_with(
            theme
                .pressable()
                .bordered()
                .radius_sm()
                .row()
                .gap(6.0)
                .height(UIValue::Px(theme.control_height))
                .padding(UIRect::axes(0.0, 8.0))
                .grow()
                .shrink(1.0)
                .min_width(UIValue::Px(0.0))
                .on_click(toggle_variant_menu(row, self.variants)),
            |mut button| {
                button
                    .add_child((
                        theme
                            .label(name)
                            .font_size(theme.font_size_sm)
                            .single_line()
                            .grow()
                            .shrink(1.0)
                            .min_width(UIValue::Px(0.0)),
                        VariantPickerLabel { row },
                    ))
                    .add_child(icon(theme, glyph::CARET_DOWN, theme.font_size_sm).muted());
            },
        );
        Ok(())
    }

    fn apply(
        &self,
        value: &mut dyn Reflect,
        edit: &dyn Any,
        _: &EditContext,
    ) -> Result<(), EditError> {
        switch_variant(value, edit)
    }

    fn edit_to(&self, _: &dyn Reflect) -> Result<Option<Arc<dyn Any + Send + Sync>>, EditError> {
        Ok(None)
    }

    fn follows_snapshot(&self) -> bool {
        true
    }
}

impl ErasedEditor for OptionToggle {
    fn snapshot(&self, value: &dyn Reflect) -> Result<PropertyRowValue, EditError> {
        Ok(PropertyRowValue::erased::<Self, bool>(
            (value as &dyn Any).type_id(),
            active_variant(value)? == "Some",
        ))
    }

    fn build(
        &self,
        cmd: &mut CommandQueue,
        row: Entity,
        snapshot: &PropertyRowValue,
        theme: &UITheme,
    ) -> Result<(), EditError> {
        let checked = *snapshot.downcast::<bool>().ok_or(EditError::TypeMismatch)?;
        let margin = ((theme.control_height - BOX_SIZE) / 2.0).max(0.0);
        cmd.entity(row).add_child((
            theme
                .checkbox("", checked)
                .size(UIValue::Px(BOX_SIZE), UIValue::Px(BOX_SIZE))
                .padding(0.0)
                .margin(UIRect::axes(margin, 0.0))
                .radius_sm()
                .fixed()
                .on_change(toggle_option(row)),
            OptionField { row },
        ));
        Ok(())
    }

    fn apply(
        &self,
        value: &mut dyn Reflect,
        edit: &dyn Any,
        _: &EditContext,
    ) -> Result<(), EditError> {
        switch_variant(value, edit)
    }

    fn edit_to(&self, _: &dyn Reflect) -> Result<Option<Arc<dyn Any + Send + Sync>>, EditError> {
        Ok(None)
    }

    fn follows_snapshot(&self) -> bool {
        true
    }
}

/// Commits `Some` or `None` to `row` when its checkbox no longer matches the row's value.
fn toggle_option(row: Entity) -> Listener<UICheckboxChanged> {
    (move |on: On<UICheckboxChanged>,
           rows: Query<(&PropertyRow, &PropertyRowValue)>,
           mut commits: ResMut<PropertyCommits>| {
        let Some((target, value)) = rows.get_entity(row) else {
            return;
        };
        let checked = on.signal().checked;
        if value.downcast::<bool>() == Some(&checked) {
            return;
        }
        let name: &'static str = if checked { "Some" } else { "None" };
        match PropertyCommit::erased::<OptionToggle>(target, name) {
            Ok(commit) => commits.0.push(commit),
            Err(error) => log::warn!("Option toggle dropped: {error}"),
        }
    })
    .into_listener()
}

/// Opens the shared menu on `row`'s variants, or closes it if `row`'s button opened it.
fn toggle_variant_menu(row: Entity, variants: &'static [&'static str]) -> Listener<UIClick> {
    (move |on: On<UIClick>,
           values: Query<&PropertyRowValue>,
           menus: Query<(Entity, &mut VariantMenu, &mut UIAnchoredPanel)>,
           theme: Res<UITheme>,
           mut cmd: CommandQueue| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        let button = on.entity();
        let mut menu = match menus.iter().next() {
            Some((entity, mut menu, mut panel)) => {
                if panel.open && panel.owner == Some(button) {
                    panel.open = false;
                    return;
                }
                menu.row = Some(row);
                panel.target = UIAnchorTarget::from_node(button);
                panel.owner = Some(button);
                panel.open = true;
                cmd.entity(entity).despawn_children()
            }
            None => cmd.spawn((
                theme
                    .dropdown(button)
                    .fill(MENU_FILL)
                    .width(UIValue::Px(MENU_WIDTH))
                    .padding(theme.spacing_xs)
                    .open(true),
                VariantMenu { row: Some(row) },
            )),
        };
        let current = values
            .get_entity(row)
            .and_then(|value| value.downcast::<&'static str>().copied());
        for &name in variants {
            menu.add_child_with(
                theme
                    .pressable()
                    .selected(current == Some(name))
                    .row()
                    .fixed()
                    .height(UIValue::Px(ITEM_HEIGHT))
                    .padding(UIRect::axes(0.0, 8.0))
                    .on_click(pick_variant(name)),
                |mut item| {
                    item.add_child(
                        theme
                            .label(name)
                            .font_size(12.5)
                            .single_line()
                            .grow()
                            .shrink(1.0)
                            .min_width(UIValue::Px(0.0)),
                    );
                },
            );
        }
    })
    .into_listener()
}

/// Commits `name` to the row the open menu was opened for unless it is already active, then closes the menu.
fn pick_variant(name: &'static str) -> Listener<UIClick> {
    (move |on: On<UIClick>,
           menus: Query<(&VariantMenu, &mut UIAnchoredPanel)>,
           rows: Query<(&PropertyRow, &PropertyRowValue)>,
           mut commits: ResMut<PropertyCommits>| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        for (menu, mut panel) in menus.iter() {
            if let Some((row, value)) = menu
                .row
                .filter(|_| panel.open)
                .and_then(|row| rows.get_entity(row))
                && value.downcast::<&'static str>() != Some(&name)
            {
                match PropertyCommit::erased::<VariantPicker>(row, name) {
                    Ok(commit) => commits.0.push(commit),
                    Err(error) => log::warn!("Variant pick dropped: {error}"),
                }
            }
            panel.open = false;
        }
    })
    .into_listener()
}

pub(super) fn forget_scene(
    _: On<Remove<EditTarget>>,
    menus: Query<(&mut VariantMenu, &mut UIAnchoredPanel)>,
) {
    for (mut menu, mut panel) in menus.iter() {
        if menu.row.is_some() {
            menu.row = None;
        }
        if panel.open {
            panel.open = false;
        }
    }
}

/// Writes the active variant of each picker row into its button.
pub(crate) fn refresh_variant_labels(
    values: Query<&PropertyRowValue>,
    labels: Query<(&VariantPickerLabel, &mut UIText)>,
) {
    for (label, mut text) in labels.iter() {
        let Some(&name) = values
            .get_entity(label.row)
            .and_then(|value| value.downcast::<&'static str>())
        else {
            continue;
        };
        if text.text != name {
            text.text = name.into();
        }
    }
}

/// Writes whether each option row holds a value into its checkbox.
pub(crate) fn refresh_option_fields(
    values: Query<&PropertyRowValue>,
    fields: Query<(&OptionField, &mut UICheckbox)>,
) {
    for (field, mut checkbox) in fields.iter() {
        let Some(&checked) = values
            .get_entity(field.row)
            .and_then(|value| value.downcast::<bool>())
        else {
            continue;
        };
        if checkbox.checked != checked {
            checkbox.checked = checked;
        }
    }
}
