use concerto_color::Color;
use concerto_ecs::command::{CommandQueue, EntityCommandQueue};
use concerto_ecs::query::filter::With;
use concerto_ecs::signal::{
    On,
    lifecycle::Remove,
    listener::{IntoListener, Listener},
};
use concerto_ecs::{Component, Entity, Query, Res, ResMut};
use concerto_ui::elements::prelude::*;
use concerto_ui::{
    anchor::{UIAnchorAlign, UIAnchorSide, UIAnchorTarget, UIAnchoredPanel},
    color_picker::{UIColorChanged, restore_on_escape},
    interaction::{Interactable, UIClick},
    material::UIMaterial,
    theme::UITheme,
    transform::UIValue,
};
use concerto_window::input::MouseButton;

use crate::hosted::EditTarget;
use crate::inspector::numeric::{NumericEdit, NumericFields, NumericSnapshot, NumericValue};
use crate::inspector::rows::{EditError, PropertyCommits, PropertyRow, PropertyRowValue};

/// The space a [`Color`] is stored in, which is the space its fields edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ColorSpace {
    Linear,
    Srgb,
    Hsl,
}

impl ColorSpace {
    pub(crate) fn of(color: &Color) -> Self {
        match color {
            Color::LinearRgba(_) => Self::Linear,
            Color::Srgba(_) => Self::Srgb,
            Color::Hsla(_) => Self::Hsl,
        }
    }
}

pub(crate) fn register_defaults(registry: &mut super::registry::InspectorRegistry) {
    registry.register_property_editor::<Color, NumericFields>();
}

pub(crate) fn channel(color: &Color, slot: usize, number: f64) -> Result<f32, EditError> {
    let number = number as f32;
    if slot >= 4 || !number.is_finite() {
        return Err(EditError::Rejected);
    }
    let unit = (0.0..=1.0).contains(&number);
    let accepted = match (color, slot) {
        (_, 3) => unit,
        (Color::Hsla(_), 0) => return Ok(hue(number)),
        (Color::LinearRgba(_), _) => number >= 0.0,
        _ => unit,
    };
    if !accepted {
        return Err(EditError::Rejected);
    }
    Ok(positive_zero(number))
}

fn hue(degrees: f32) -> f32 {
    let wrapped = degrees.rem_euclid(360.0);
    if wrapped >= 360.0 {
        0.0
    } else {
        positive_zero(wrapped)
    }
}

fn positive_zero(number: f32) -> f32 {
    if number == 0.0 { 0.0 } else { number }
}

fn channels(color: &Color) -> [f32; 4] {
    match color {
        Color::LinearRgba(c) => [c.r, c.g, c.b, c.a],
        Color::Srgba(c) => [c.r, c.g, c.b, c.a],
        Color::Hsla(c) => [c.h, c.s, c.l, c.a],
    }
}

fn channels_mut(color: &mut Color) -> [&mut f32; 4] {
    match color {
        Color::LinearRgba(c) => [&mut c.r, &mut c.g, &mut c.b, &mut c.a],
        Color::Srgba(c) => [&mut c.r, &mut c.g, &mut c.b, &mut c.a],
        Color::Hsla(c) => [&mut c.h, &mut c.s, &mut c.l, &mut c.a],
    }
}

pub(crate) fn shown(color: &Color, slot: usize) -> Option<f64> {
    channels(color).get(slot).copied().map(f64::from)
}

impl NumericValue for Color {
    fn numbers(&self) -> NumericSnapshot {
        NumericSnapshot::Color(*self)
    }
    fn edit(&mut self, edit: &NumericEdit) -> Result<(), EditError> {
        let mut next = *self;
        match *edit {
            NumericEdit::Slot {
                slot,
                number,
                space,
            } => {
                if space != Some(ColorSpace::of(self)) {
                    return Err(EditError::Rejected);
                }
                let number = channel(self, slot, number)?;
                *channels_mut(&mut next)
                    .into_iter()
                    .nth(slot)
                    .ok_or(EditError::Rejected)? = number;
            }
            NumericEdit::Whole(NumericSnapshot::Color(color)) => {
                next = color;
                for (slot, number) in channels_mut(&mut next).into_iter().enumerate() {
                    *number = channel(&color, slot, *number as f64)?;
                }
            }
            NumericEdit::Whole(_) => return Err(EditError::Rejected),
        }
        *self = next;
        Ok(())
    }
}

fn opaque(color: Color) -> Color {
    let mut color = color;
    let [_, _, _, alpha] = channels_mut(&mut color);
    *alpha = 1.0;
    color
}

/// The swatch of a color row, filled with the row's color at full alpha.
#[derive(Component)]
pub(crate) struct ColorSwatch {
    pub(super) row: Entity,
}

pub(super) fn spawn_swatch(row: &mut EntityCommandQueue, color: Color, theme: &UITheme) {
    let side = UIValue::Px(theme.control_height);
    let entity = row.entity();
    row.add_child((
        theme
            .canvas()
            .fill(opaque(color))
            .bordered()
            .radius_sm()
            .size(side, side)
            .fixed(),
        Interactable,
        ColorSwatch { row: entity },
        toggle_color_picker(entity),
    ));
}

/// The popup every color row shares, holding a picker for the row whose swatch opened it last.
#[derive(Component)]
pub(crate) struct ColorPickerPopup {
    row: Entity,
}

fn picker_panel(swatch: Entity) -> UIAnchoredPanel {
    UIAnchoredPanel::new(UIAnchorTarget::from_node(swatch))
        .with_owner(swatch)
        .with_side(UIAnchorSide::Below)
        .with_align(UIAnchorAlign::Start)
        .with_gap(4.0)
        .with_open(true)
}

fn toggle_color_picker(row: Entity) -> Listener<UIClick> {
    (move |on: On<UIClick>,
           values: Query<&PropertyRowValue>,
           popups: Query<(Entity, &mut ColorPickerPopup, &mut UIAnchoredPanel)>,
           theme: Res<UITheme>,
           mut cmd: CommandQueue| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        let Some(NumericSnapshot::Color(current)) = values
            .get_entity(row)
            .and_then(|value| value.downcast::<NumericSnapshot>())
        else {
            return;
        };
        let current = *current;
        let mut popup = match popups.iter().next() {
            Some((entity, mut popup, mut panel)) => {
                if panel.open && panel.owner == Some(on.entity()) {
                    panel.open = false;
                    return;
                }
                if popup.row != row {
                    popup.row = row;
                }
                **panel = picker_panel(on.entity());
                cmd.entity(entity).despawn_children()
            }
            None => cmd.spawn((
                theme.popup().padding(theme.spacing_sm),
                picker_panel(on.entity()),
                ColorPickerPopup { row },
            )),
        };
        let picker = popup
            .spawn_child(theme.color_picker(current).on_change(commit_picked(row)))
            .entity();
        popup.insert(restore_on_escape(picker));
    })
    .into_listener()
}

fn commit_picked(row: Entity) -> Listener<UIColorChanged> {
    (move |on: On<UIColorChanged>,
           rows: Query<&PropertyRow>,
           mut commits: ResMut<PropertyCommits>| {
        let Some(row) = rows.get_entity(row) else {
            return;
        };
        let edit = NumericEdit::Whole(NumericSnapshot::Color(on.signal().color));
        if let Err(error) = commits.push::<Color, NumericFields>(row, edit) {
            log::warn!("Color pick dropped: {error}");
        }
    })
    .into_listener()
}

/// Closes the color popup once the row it was opened for is gone.
pub(crate) fn close_color_picker(
    rows: Query<&PropertyRow>,
    popups: Query<(&ColorPickerPopup, &mut UIAnchoredPanel)>,
) {
    for (popup, mut panel) in popups.iter() {
        if panel.open && rows.get_entity(popup.row).is_none() {
            panel.open = false;
        }
    }
}

pub(super) fn forget_scene(
    _: On<Remove<EditTarget>>,
    panels: Query<&mut UIAnchoredPanel, With<ColorPickerPopup>>,
) {
    for mut panel in panels.iter() {
        if panel.open {
            panel.open = false;
        }
    }
}

/// Writes each color row's value, at full alpha, into its swatch.
pub(crate) fn refresh_color_swatches(
    rows: Query<&PropertyRowValue>,
    mut swatches: Query<(&ColorSwatch, &mut UIMaterial)>,
) {
    for (swatch, mut material) in swatches.iter_mut() {
        let Some(NumericSnapshot::Color(color)) = rows
            .get_entity(swatch.row)
            .and_then(|value| value.downcast::<NumericSnapshot>())
        else {
            continue;
        };
        let fill = opaque(*color).to_linear();
        if material.color != fill {
            material.color = fill;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ChannelEdit {
        color: Color,
        slot: usize,
        number: f64,
        space: Option<ColorSpace>,
        edited: Option<Color>,
    }

    fn row(color: Color, slot: usize, number: f64, edited: Option<Color>) -> ChannelEdit {
        ChannelEdit {
            color,
            slot,
            number,
            space: Some(ColorSpace::of(&color)),
            edited,
        }
    }

    #[test]
    fn a_channel_edit_keeps_the_space_wraps_hue_in_degrees_and_refuses_what_the_space_cannot_hold()
    {
        let linear = Color::rgba(0.5, 0.5, 0.5, 1.0);
        let srgb = Color::srgba(0.5, 0.25, 0.75, 1.0);
        let hsl = Color::hsl(180.0, 0.5, 0.5, 1.0);
        let rows = [
            row(linear, 0, 4.0, Some(Color::rgba(4.0, 0.5, 0.5, 1.0))),
            row(linear, 0, -0.1, None),
            row(linear, 3, 1.5, None),
            row(srgb, 0, 0.9, Some(Color::srgba(0.9, 0.25, 0.75, 1.0))),
            row(srgb, 1, 1.2, None),
            row(hsl, 0, 200.0, Some(Color::hsl(200.0, 0.5, 0.5, 1.0))),
            row(hsl, 0, 450.0, Some(Color::hsl(90.0, 0.5, 0.5, 1.0))),
            row(hsl, 0, -90.0, Some(Color::hsl(270.0, 0.5, 0.5, 1.0))),
            row(hsl, 0, 360.0, Some(Color::hsl(0.0, 0.5, 0.5, 1.0))),
            row(hsl, 2, 0.75, Some(Color::hsl(180.0, 0.5, 0.75, 1.0))),
            row(hsl, 1, 1.5, None),
            row(srgb, 0, f64::NAN, None),
            ChannelEdit {
                space: Some(ColorSpace::Hsl),
                ..row(srgb, 0, 0.25, None)
            },
        ];
        for row in rows {
            let mut color = row.color;
            let result = color.edit(&NumericEdit::Slot {
                slot: row.slot,
                number: row.number,
                space: row.space,
            });
            let label = format!("{:?} slot {} = {}", row.color, row.slot, row.number);
            match row.edited {
                Some(edited) => {
                    assert_eq!(result, Ok(()), "{label}");
                    assert_eq!(color, edited, "{label}");
                }
                None => {
                    assert_eq!(result, Err(EditError::Rejected), "{label}");
                    assert_eq!(color, row.color, "{label}");
                }
            }
        }
    }
}
