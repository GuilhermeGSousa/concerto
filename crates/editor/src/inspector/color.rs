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
    let turns = degrees.rem_euclid(360.0) / 360.0;
    if turns >= 1.0 {
        0.0
    } else {
        positive_zero(turns)
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
    let number = f64::from(*channels(color).get(slot)?);
    Some(match (color, slot) {
        (Color::Hsla(_), 0) => number * 360.0,
        _ => number,
    })
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
    swatches: Query<(&ColorSwatch, &mut UIMaterial)>,
) {
    for (swatch, mut material) in swatches.iter() {
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
    use crate::inspector::numeric::{format_slot, slot_label};

    fn edit(color: &mut Color, slot: usize, number: f64) -> Result<(), EditError> {
        let space = Some(ColorSpace::of(color));
        color.edit(&NumericEdit::Slot {
            slot,
            number,
            space,
        })
    }

    fn edited(color: Color, slot: usize, number: f64) -> Color {
        let mut copy = color;
        assert_eq!(
            edit(&mut copy, slot, number),
            Ok(()),
            "{color:?} slot {slot} = {number}"
        );
        copy
    }

    fn refused(color: Color, slot: usize, number: f64) {
        let mut copy = color;
        assert_eq!(
            edit(&mut copy, slot, number),
            Err(EditError::Rejected),
            "{color:?} slot {slot} = {number}"
        );
        assert_eq!(copy, color);
    }

    fn shown_slots(color: Color) -> Vec<String> {
        (0..4)
            .map(|slot| format_slot(&color.numbers(), slot))
            .collect()
    }

    #[test]
    fn a_color_shows_the_channels_of_its_own_space() {
        let linear = Color::rgba(0.1, 0.2, 0.3, 0.4);
        let srgb = Color::srgba(0.5, 0.25, 0.75, 1.0);
        let hsl = Color::hsl(0.25, 0.5, 0.5, 1.0);
        for color in [linear, srgb, hsl] {
            assert_eq!(color.numbers(), NumericSnapshot::Color(color));
        }
        assert_eq!(shown_slots(linear), ["0.100", "0.200", "0.300", "0.400"]);
        assert_eq!(shown_slots(srgb), ["0.500", "0.250", "0.750", "1.000"]);
        assert_eq!(shown_slots(hsl), ["90.000", "0.500", "0.500", "1.000"]);
        let letters = |snapshot: NumericSnapshot| -> Vec<Option<&'static str>> {
            (0..4).map(|slot| slot_label(&snapshot, slot)).collect()
        };
        let rgba = [Some("R"), Some("G"), Some("B"), Some("A")];
        assert_eq!(letters(linear.numbers()), rgba);
        assert_eq!(letters(srgb.numbers()), rgba);
        assert_eq!(
            letters(hsl.numbers()),
            [Some("H"), Some("S"), Some("L"), Some("A")]
        );
        assert_eq!(letters(NumericSnapshot::Number(1.0)), [None; 4]);
        assert_eq!(letters(NumericSnapshot::Vec3([1.0; 3])), [None; 4]);
    }

    #[test]
    fn the_swatch_drops_alpha_in_the_values_own_space() {
        assert_eq!(
            opaque(Color::srgba(0.5, 0.5, 0.5, 0.2)),
            Color::srgba(0.5, 0.5, 0.5, 1.0)
        );
        assert_eq!(
            opaque(Color::hsl(0.25, 0.5, 0.5, 0.0)),
            Color::hsl(0.25, 0.5, 0.5, 1.0)
        );
        assert_eq!(
            opaque(Color::rgba(2.0, 0.5, 0.0, 0.5)),
            Color::rgba(2.0, 0.5, 0.0, 1.0)
        );
    }

    #[test]
    fn editing_a_channel_keeps_the_color_space() {
        assert_eq!(
            edited(Color::rgba(0.1, 0.2, 0.3, 0.4), 0, 0.9),
            Color::rgba(0.9, 0.2, 0.3, 0.4)
        );
        assert_eq!(
            edited(Color::srgba(0.5, 0.25, 0.75, 1.0), 0, 0.9),
            Color::srgba(0.9, 0.25, 0.75, 1.0)
        );
        assert_eq!(
            edited(Color::hsl(0.25, 0.5, 0.5, 1.0), 0, 180.0),
            Color::hsl(0.5, 0.5, 0.5, 1.0)
        );
        assert_eq!(
            edited(Color::hsl(0.25, 0.5, 0.5, 1.0), 2, 0.75),
            Color::hsl(0.25, 0.5, 0.75, 1.0)
        );
    }

    #[test]
    fn linear_channels_accept_hdr_and_refuse_negatives() {
        let color = Color::rgba(0.5, 0.5, 0.5, 1.0);
        assert_eq!(edited(color, 0, 4.0), Color::rgba(4.0, 0.5, 0.5, 1.0));
        refused(color, 0, -0.1);
        refused(color, 3, 1.5);
    }

    #[test]
    fn srgb_and_hsl_channels_refuse_values_outside_zero_to_one() {
        let srgb = Color::srgba(0.5, 0.5, 0.5, 0.5);
        let hsl = Color::hsl(0.5, 0.5, 0.5, 0.5);
        refused(srgb, 1, 1.2);
        refused(srgb, 2, -0.01);
        refused(hsl, 1, 1.5);
        refused(hsl, 2, -0.5);
        refused(hsl, 3, 2.0);
        refused(srgb, 3, -0.5);
        for bound in [0.0, 1.0] {
            for slot in 0..4 {
                assert_eq!(channels(&edited(srgb, slot, bound))[slot], bound as f32);
            }
            for slot in 1..4 {
                assert_eq!(channels(&edited(hsl, slot, bound))[slot], bound as f32);
            }
        }
    }

    #[test]
    fn hue_is_edited_in_degrees_and_wraps() {
        let hsl = Color::hsl(0.5, 0.5, 0.5, 1.0);
        let hue = |degrees: f64| channels(&edited(hsl, 0, degrees))[0];
        assert_eq!(hue(450.0), 0.25);
        assert_eq!(hue(-90.0), 0.75);
        assert_eq!(hue(360.0), 0.0);
        assert_eq!(hue(-1e-9), 0.0);
    }

    #[test]
    fn colors_refuse_non_finite_and_out_of_range_slots_without_mutating() {
        for color in [
            Color::rgba(0.5, 0.5, 0.5, 1.0),
            Color::srgba(0.5, 0.5, 0.5, 1.0),
            Color::hsl(0.5, 0.5, 0.5, 1.0),
        ] {
            for slot in 0..4 {
                for number in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1e300] {
                    refused(color, slot, number);
                }
            }
            refused(color, 4, 0.5);
        }
    }

    #[test]
    fn channel_checks_use_the_f32_value_and_store_positive_zero() {
        let srgb = Color::srgba(0.5, 0.5, 0.5, 1.0);
        assert_eq!(channels(&edited(srgb, 0, 1.0000000001))[0], 1.0);
        for zero in [
            channels(&edited(Color::rgba(0.5, 0.5, 0.5, 1.0), 0, -0.0))[0],
            channels(&edited(srgb, 0, -1e-50))[0],
            channels(&edited(Color::hsl(0.5, 0.5, 0.5, 1.0), 0, -0.0))[0],
        ] {
            assert_eq!(zero, 0.0);
            assert!(zero.is_sign_positive());
        }
    }

    #[test]
    fn a_whole_color_edit_sets_the_color_and_its_space() {
        let mut copy = Color::srgba(0.5, 0.5, 0.5, 1.0);
        let linear = Color::rgba(2.0, 0.25, 0.0, 0.5);
        assert_eq!(
            copy.edit(&NumericEdit::Whole(NumericSnapshot::Color(linear))),
            Ok(())
        );
        assert_eq!(copy, linear);
    }

    #[test]
    fn a_whole_color_edit_out_of_range_is_refused() {
        let color = Color::srgba(0.5, 0.5, 0.5, 1.0);
        for bad in [
            Color::srgba(1.5, 0.5, 0.5, 1.0),
            Color::rgba(0.5, 0.5, 0.5, 2.0),
            Color::rgba(f32::NAN, 0.5, 0.5, 1.0),
        ] {
            let mut copy = color;
            assert_eq!(
                copy.edit(&NumericEdit::Whole(NumericSnapshot::Color(bad))),
                Err(EditError::Rejected),
                "{bad:?}"
            );
            assert_eq!(copy, color);
        }
    }

    #[test]
    fn an_edit_made_against_another_space_is_refused() {
        let color = Color::srgba(0.5, 0.5, 0.5, 1.0);
        for space in [Some(ColorSpace::Hsl), None] {
            let mut copy = color;
            assert_eq!(
                copy.edit(&NumericEdit::Slot {
                    slot: 0,
                    number: 0.25,
                    space,
                }),
                Err(EditError::Rejected)
            );
            assert_eq!(copy, color);
        }
        let mut copy = color;
        copy.edit(&NumericEdit::Slot {
            slot: 0,
            number: 0.25,
            space: Some(ColorSpace::Srgb),
        })
        .unwrap();
        assert_eq!(copy, Color::srgba(0.25, 0.5, 0.5, 1.0));
    }
}
