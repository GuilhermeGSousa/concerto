use concerto_color::Color;

use crate::inspector::numeric::{NumericEdit, NumericSnapshot, NumericValue};
use crate::inspector::rows::EditError;

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
        if edit.space != Some(ColorSpace::of(self)) {
            return Err(EditError::Rejected);
        }
        let number = channel(self, edit.slot, edit.number)?;
        let mut next = *self;
        *channels_mut(&mut next)
            .into_iter()
            .nth(edit.slot)
            .ok_or(EditError::Rejected)? = number;
        *self = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inspector::numeric::format_slot;

    fn edit(color: &mut Color, slot: usize, number: f64) -> Result<(), EditError> {
        let space = Some(ColorSpace::of(color));
        color.edit(&NumericEdit {
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
    fn an_edit_made_against_another_space_is_refused() {
        let color = Color::srgba(0.5, 0.5, 0.5, 1.0);
        for space in [Some(ColorSpace::Hsl), None] {
            let mut copy = color;
            assert_eq!(
                copy.edit(&NumericEdit {
                    slot: 0,
                    number: 0.25,
                    space,
                }),
                Err(EditError::Rejected)
            );
            assert_eq!(copy, color);
        }
        let mut copy = color;
        copy.edit(&NumericEdit {
            slot: 0,
            number: 0.25,
            space: Some(ColorSpace::Srgb),
        })
        .unwrap();
        assert_eq!(copy, Color::srgba(0.25, 0.5, 0.5, 1.0));
    }
}
