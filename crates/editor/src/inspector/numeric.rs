use concerto_ecs::command::EntityCommandQueue;
use concerto_ecs::{Component, Entity, Query, Res, ResMut, events::event_reader::EventReader};
use concerto_reflect::Reflect;
use concerto_ui::elements::prelude::*;
use concerto_ui::{
    focus::{FocusedWidget, UIFocusLost},
    node::{FlexWrap, UINode, UIRect},
    text::UIText,
    text_input::{UITextInput, UITextInputCancelled, UITextInputSubmitted},
    theme::UITheme,
    transform::UIValue,
};
use glam::{EulerRot, Quat, Vec2, Vec3};

use crate::inspector::color::{self, ColorSpace};
use crate::property::{
    EditContext, EditError, PropertyCommits, PropertyEditor, PropertyRow, PropertyRowValue,
};

/// One text field per number: one for `Number`, `Count` and `Integer`, two for `Vec2`, three for `Vec3`, and for `Color` a swatch beside the label with four labelled fields in the color's own space on a line below.
#[derive(Default)]
pub(crate) struct NumericFields;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum NumericSnapshot {
    Number(f64),
    Count(usize),
    Integer(i64),
    Vec2([f64; 2]),
    Vec3([f64; 3]),
    Color(concerto_color::Color),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum NumericEdit {
    Slot {
        slot: usize,
        number: f64,
        space: Option<ColorSpace>,
    },
    Whole(NumericSnapshot),
}

pub(crate) fn register_defaults(registry: &mut super::registry::InspectorRegistry) {
    registry.register_property_editor::<f32, NumericFields>();
    registry.register_property_editor::<f64, NumericFields>();
    registry.register_property_editor::<usize, NumericFields>();
    registry.register_property_editor::<u32, NumericFields>();
    registry.register_property_editor::<i32, NumericFields>();
    registry.register_property_editor::<Vec2, NumericFields>();
    registry.register_property_editor::<Vec3, NumericFields>();
    registry.register_property_editor::<Quat, NumericFields>();
}

pub(crate) trait NumericValue: Reflect {
    fn numbers(&self) -> NumericSnapshot;
    fn edit(&mut self, edit: &NumericEdit) -> Result<(), EditError>;
}

macro_rules! scalar {
    ($ty:ty) => {
        impl NumericValue for $ty {
            fn numbers(&self) -> NumericSnapshot {
                NumericSnapshot::Number(*self as f64)
            }
            fn edit(&mut self, edit: &NumericEdit) -> Result<(), EditError> {
                let number = match *edit {
                    NumericEdit::Slot {
                        slot: 0, number, ..
                    }
                    | NumericEdit::Whole(NumericSnapshot::Number(number)) => number as $ty,
                    _ => return Err(EditError::Rejected),
                };
                if !number.is_finite() {
                    return Err(EditError::Rejected);
                }
                *self = number;
                Ok(())
            }
        }
    };
}
scalar!(f32);
scalar!(f64);

fn count_of(number: f64) -> Option<usize> {
    (number >= 0.0 && number.fract() == 0.0 && number < usize::MAX as f64)
        .then_some(number as usize)
}

impl NumericValue for usize {
    fn numbers(&self) -> NumericSnapshot {
        NumericSnapshot::Count(*self)
    }
    fn edit(&mut self, edit: &NumericEdit) -> Result<(), EditError> {
        *self = match *edit {
            NumericEdit::Slot {
                slot: 0, number, ..
            } => count_of(number).ok_or(EditError::Rejected)?,
            NumericEdit::Whole(NumericSnapshot::Count(count)) => count,
            _ => return Err(EditError::Rejected),
        };
        Ok(())
    }
}

impl NumericValue for u32 {
    fn numbers(&self) -> NumericSnapshot {
        NumericSnapshot::Count(*self as usize)
    }
    fn edit(&mut self, edit: &NumericEdit) -> Result<(), EditError> {
        let count = match *edit {
            NumericEdit::Slot {
                slot: 0, number, ..
            } => count_of(number).ok_or(EditError::Rejected)?,
            NumericEdit::Whole(NumericSnapshot::Count(count)) => count,
            _ => return Err(EditError::Rejected),
        };
        *self = u32::try_from(count).map_err(|_| EditError::Rejected)?;
        Ok(())
    }
}

fn integer_of(number: f64) -> Option<i64> {
    (number.fract() == 0.0 && number >= i64::MIN as f64 && number < i64::MAX as f64)
        .then_some(number as i64)
}

impl NumericValue for i32 {
    fn numbers(&self) -> NumericSnapshot {
        NumericSnapshot::Integer(*self as i64)
    }
    fn edit(&mut self, edit: &NumericEdit) -> Result<(), EditError> {
        let integer = match *edit {
            NumericEdit::Slot {
                slot: 0, number, ..
            } => integer_of(number).ok_or(EditError::Rejected)?,
            NumericEdit::Whole(NumericSnapshot::Integer(integer)) => integer,
            _ => return Err(EditError::Rejected),
        };
        *self = i32::try_from(integer).map_err(|_| EditError::Rejected)?;
        Ok(())
    }
}

impl NumericValue for Vec2 {
    fn numbers(&self) -> NumericSnapshot {
        NumericSnapshot::Vec2([self.x as f64, self.y as f64])
    }
    fn edit(&mut self, edit: &NumericEdit) -> Result<(), EditError> {
        let mut next = *self;
        match *edit {
            NumericEdit::Slot { slot, number, .. } if slot < 2 => next[slot] = number as f32,
            NumericEdit::Whole(NumericSnapshot::Vec2(numbers)) => {
                next = Vec2::from_array(numbers.map(|number| number as f32))
            }
            _ => return Err(EditError::Rejected),
        }
        if !next.is_finite() {
            return Err(EditError::Rejected);
        }
        *self = next;
        Ok(())
    }
}

impl NumericValue for Vec3 {
    fn numbers(&self) -> NumericSnapshot {
        NumericSnapshot::Vec3([self.x as f64, self.y as f64, self.z as f64])
    }
    fn edit(&mut self, edit: &NumericEdit) -> Result<(), EditError> {
        let mut next = *self;
        match *edit {
            NumericEdit::Slot { slot, number, .. } if slot < 3 => next[slot] = number as f32,
            NumericEdit::Whole(NumericSnapshot::Vec3(numbers)) => {
                next = Vec3::from_array(numbers.map(|number| number as f32))
            }
            _ => return Err(EditError::Rejected),
        }
        if !next.is_finite() {
            return Err(EditError::Rejected);
        }
        *self = next;
        Ok(())
    }
}

impl NumericValue for Quat {
    fn numbers(&self) -> NumericSnapshot {
        let (x, y, z) = self.to_euler(EulerRot::XYZ);
        Vec3::new(x, y, z).map(f32::to_degrees).numbers()
    }
    fn edit(&mut self, edit: &NumericEdit) -> Result<(), EditError> {
        let NumericSnapshot::Vec3(degrees) = self.numbers() else {
            return Err(EditError::TypeMismatch);
        };
        let mut degrees = Vec3::new(degrees[0] as f32, degrees[1] as f32, degrees[2] as f32);
        degrees.edit(edit)?;
        let radians = degrees.map(f32::to_radians);
        let next = Quat::from_euler(EulerRot::XYZ, radians.x, radians.y, radians.z).normalize();
        if !next.is_finite() {
            return Err(EditError::Rejected);
        }
        *self = next;
        Ok(())
    }
}

fn queue_numeric(
    commits: &mut PropertyCommits,
    row: &PropertyRow,
    edit: NumericEdit,
) -> Result<(), EditError> {
    use std::any::TypeId;
    let type_id = row.address.type_id;
    if type_id == TypeId::of::<f32>() {
        commits.push::<f32, NumericFields>(row, edit)
    } else if type_id == TypeId::of::<f64>() {
        commits.push::<f64, NumericFields>(row, edit)
    } else if type_id == TypeId::of::<usize>() {
        commits.push::<usize, NumericFields>(row, edit)
    } else if type_id == TypeId::of::<u32>() {
        push_checked::<u32>(commits, row, edit)
    } else if type_id == TypeId::of::<i32>() {
        push_checked::<i32>(commits, row, edit)
    } else if type_id == TypeId::of::<Vec2>() {
        commits.push::<Vec2, NumericFields>(row, edit)
    } else if type_id == TypeId::of::<Vec3>() {
        commits.push::<Vec3, NumericFields>(row, edit)
    } else if type_id == TypeId::of::<Quat>() {
        commits.push::<Quat, NumericFields>(row, edit)
    } else if type_id == TypeId::of::<concerto_color::Color>() {
        commits.push::<concerto_color::Color, NumericFields>(row, edit)
    } else {
        Err(EditError::TypeMismatch)
    }
}

fn push_checked<T: NumericValue + Default>(
    commits: &mut PropertyCommits,
    row: &PropertyRow,
    edit: NumericEdit,
) -> Result<(), EditError> {
    T::default().edit(&edit)?;
    commits.push::<T, NumericFields>(row, edit)
}

/// Queues `number` as the whole value of a float `row`.
///
/// Any other numeric row, such as a `usize` or a vector, takes the edit here and refuses it when it is applied.
///
/// # Errors
///
/// Returns [`EditError::TypeMismatch`] when `NumericFields` does not edit `row` or its value is not numeric, and
/// [`EditError::Rejected`] when `row` holds a `u32` or an `i32`.
#[cfg(any(test, feature = "testing"))]
pub fn commit_number(
    commits: &mut PropertyCommits,
    row: &PropertyRow,
    number: f64,
) -> Result<(), EditError> {
    queue_numeric(
        commits,
        row,
        NumericEdit::Whole(NumericSnapshot::Number(number)),
    )
}

/// One field of a [`NumericFields`] row.
#[derive(Component)]
pub(crate) struct NumericSlot {
    pub(super) row: Entity,
    pub(super) slot: usize,
    displayed: String,
}

/// The letter naming one field of a [`NumericFields`] row.
#[derive(Component)]
pub(crate) struct NumericSlotLabel {
    pub(super) row: Entity,
    pub(super) slot: usize,
}

impl<T: NumericValue> PropertyEditor<T> for NumericFields {
    type Snapshot = NumericSnapshot;
    type Edit = NumericEdit;
    fn snapshot(&self, value: &T) -> NumericSnapshot {
        value.numbers()
    }
    fn apply(&self, value: &mut T, edit: &NumericEdit, _: &EditContext) -> Result<(), EditError> {
        value.edit(edit)
    }
    fn edit_to(&self, value: &T) -> Option<NumericEdit> {
        Some(NumericEdit::Whole(value.numbers()))
    }
    fn follows_snapshot(&self) -> bool {
        true
    }
    fn row_node(&self, value: &NumericSnapshot, theme: &UITheme) -> UINode {
        let node = super::standard_row_node(theme);
        match value {
            NumericSnapshot::Color(_) => node.with_flex_wrap(FlexWrap::Wrap),
            NumericSnapshot::Number(_)
            | NumericSnapshot::Count(_)
            | NumericSnapshot::Integer(_)
            | NumericSnapshot::Vec2(_)
            | NumericSnapshot::Vec3(_) => node,
        }
    }
    fn build(&self, row: &mut EntityCommandQueue, value: &NumericSnapshot, theme: &UITheme) {
        let entity = row.entity();
        let NumericSnapshot::Color(color) = value else {
            for slot in 0..slot_count(value) {
                spawn_slot(row, entity, value, slot, theme);
            }
            return;
        };
        color::spawn_swatch(row, *color, theme);
        let mut line = row.spawn_child(theme.row().width(UIValue::Percent(100.0)));
        for slot in 0..slot_count(value) {
            spawn_slot(&mut line, entity, value, slot, theme);
        }
    }
}

fn spawn_slot(
    parent: &mut EntityCommandQueue,
    row: Entity,
    value: &NumericSnapshot,
    slot: usize,
    theme: &UITheme,
) {
    let field = (
        theme
            .text_field("")
            .small()
            .grow()
            .shrink(1.0)
            .width(UIValue::Px(0.0))
            .min_width(UIValue::Px(0.0))
            .padding(UIRect::axes(field_leading(theme), theme.spacing_xs))
            .clipped(),
        NumericSlot {
            row,
            slot,
            displayed: String::new(),
        },
    );
    let Some(letter) = slot_label(value, slot) else {
        parent.add_child(field);
        return;
    };
    parent
        .spawn_child(
            theme
                .row()
                .gap(theme.spacing_xs)
                .grow()
                .shrink(1.0)
                .width(UIValue::Px(0.0))
                .min_width(UIValue::Px(0.0)),
        )
        .add_child((
            theme.label(letter).small().muted().single_line().fixed(),
            NumericSlotLabel { row, slot },
        ))
        .add_child(field);
}

pub(super) fn field_leading(theme: &UITheme) -> f32 {
    let line = theme.line_height(theme.font_size_sm);
    ((theme.control_height - line) / 2.0).max(0.0)
}

fn slot_count(value: &NumericSnapshot) -> usize {
    match value {
        NumericSnapshot::Number(_) | NumericSnapshot::Count(_) | NumericSnapshot::Integer(_) => 1,
        NumericSnapshot::Vec2(_) => 2,
        NumericSnapshot::Vec3(_) => 3,
        NumericSnapshot::Color(_) => 4,
    }
}

pub(super) fn format_slot(value: &NumericSnapshot, slot: usize) -> String {
    let number = match value {
        NumericSnapshot::Count(count) => return count.to_string(),
        NumericSnapshot::Integer(integer) => return integer.to_string(),
        NumericSnapshot::Number(number) => (slot == 0).then_some(*number),
        NumericSnapshot::Vec2(components) => components.get(slot).copied(),
        NumericSnapshot::Vec3(components) => components.get(slot).copied(),
        NumericSnapshot::Color(value) => color::shown(value, slot),
    };
    number.map(|n| format!("{n:.3}")).unwrap_or_default()
}

pub(super) fn slot_label(value: &NumericSnapshot, slot: usize) -> Option<&'static str> {
    let letters = match value {
        NumericSnapshot::Number(_)
        | NumericSnapshot::Count(_)
        | NumericSnapshot::Integer(_)
        | NumericSnapshot::Vec2(_)
        | NumericSnapshot::Vec3(_) => {
            return None;
        }
        NumericSnapshot::Color(concerto_color::Color::Hsla(_)) => ["H", "S", "L", "A"],
        NumericSnapshot::Color(_) => ["R", "G", "B", "A"],
    };
    letters.get(slot).copied()
}

fn accepts(current: &NumericSnapshot, slot: usize, number: f64) -> bool {
    match current {
        NumericSnapshot::Number(_) | NumericSnapshot::Vec2(_) | NumericSnapshot::Vec3(_) => true,
        NumericSnapshot::Count(_) => count_of(number).is_some(),
        NumericSnapshot::Integer(_) => integer_of(number).is_some(),
        NumericSnapshot::Color(value) => color::channel(value, slot, number).is_ok(),
    }
}

fn numeric_commit(
    current: &NumericSnapshot,
    slot: usize,
    text: &str,
    displayed: &str,
) -> Option<NumericEdit> {
    if text == displayed || slot >= slot_count(current) {
        return None;
    }
    let number: f64 = text.trim().parse().ok()?;
    if !number.is_finite() || !accepts(current, slot, number) {
        return None;
    }
    let space = match current {
        NumericSnapshot::Color(value) => Some(ColorSpace::of(value)),
        NumericSnapshot::Number(_)
        | NumericSnapshot::Count(_)
        | NumericSnapshot::Integer(_)
        | NumericSnapshot::Vec2(_)
        | NumericSnapshot::Vec3(_) => None,
    };
    Some(NumericEdit::Slot {
        slot,
        number,
        space,
    })
}

fn revert(slot: &NumericSlot, input: &mut UITextInput) {
    input.value = slot.displayed.clone();
    input.cursor = input.value.len();
    input.selection_anchor = None;
}

pub(crate) fn commit_numeric_fields(
    mut submitted: EventReader<UITextInputSubmitted>,
    mut lost: EventReader<UIFocusLost>,
    rows: Query<(&PropertyRow, &PropertyRowValue)>,
    mut fields: Query<(&mut NumericSlot, &mut UITextInput)>,
    mut commits: ResMut<PropertyCommits>,
) {
    let finished: Vec<Entity> = submitted
        .read()
        .map(|event| event.entity)
        .chain(lost.read().map(|event| event.0))
        .collect();
    for entity in finished {
        let Some((mut slot, mut input)) = fields.get_entity_mut(entity) else {
            continue;
        };
        let Some((row, current)) = rows.get_entity(slot.row) else {
            continue;
        };
        let Some(current) = current.downcast::<NumericSnapshot>() else {
            continue;
        };
        match numeric_commit(current, slot.slot, &input.value, &slot.displayed) {
            Some(value) => {
                if let Err(error) = queue_numeric(&mut commits, row, value) {
                    log::warn!("Numeric edit dropped: {error}");
                    revert(&slot, &mut input);
                    continue;
                }
                slot.displayed = input.value.clone();
            }
            None => revert(&slot, &mut input),
        }
    }
}

pub(crate) fn cancel_numeric_fields(
    mut cancelled: EventReader<UITextInputCancelled>,
    mut fields: Query<(&NumericSlot, &mut UITextInput)>,
    mut focused: ResMut<FocusedWidget>,
) {
    for event in cancelled.read() {
        let Some((slot, mut input)) = fields.get_entity_mut(event.entity) else {
            continue;
        };
        revert(slot, &mut input);
        if **focused == Some(event.entity) {
            **focused = None;
        }
    }
}

/// Writes each row's value into its fields, except the focused one.
pub(crate) fn refresh_numeric_fields(
    focused: Res<FocusedWidget>,
    rows: Query<&PropertyRowValue>,
    mut fields: Query<(Entity, &mut NumericSlot, &mut UITextInput)>,
) {
    for (entity, mut slot, mut input) in fields.iter_mut() {
        if **focused == Some(entity) {
            continue;
        }
        let Some(value) = rows.get_entity(slot.row) else {
            continue;
        };
        let Some(value) = value.downcast::<NumericSnapshot>() else {
            continue;
        };
        let text = format_slot(value, slot.slot);
        if input.value != text {
            input.value = text.clone();
            input.cursor = input.value.len();
            input.selection_anchor = None;
        }
        if slot.displayed != text {
            slot.displayed = text;
        }
    }
}

/// Writes the letter of each labelled field from its row's value.
pub(crate) fn refresh_numeric_labels(
    rows: Query<&PropertyRowValue>,
    mut labels: Query<(&NumericSlotLabel, &mut UIText)>,
) {
    for (label, mut text) in labels.iter_mut() {
        let Some(letter) = rows
            .get_entity(label.row)
            .and_then(|value| value.downcast::<NumericSnapshot>())
            .and_then(|value| slot_label(value, label.slot))
        else {
            continue;
        };
        if text.text != letter {
            text.text = letter.into();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unchanged_text_does_not_commit() {
        let value = NumericSnapshot::Vec3([1.23456, 0.0, 0.0]);
        let displayed = format_slot(&value, 0);
        assert_eq!(displayed, "1.235");
        assert_eq!(
            numeric_commit(&value, 0, &displayed, &displayed),
            None,
            "tabbing through a rounded field must not truncate the stored value"
        );
    }

    #[test]
    fn quaternion_adapter_uses_xyz_degrees_and_normalizes() {
        let original = Quat::from_euler(EulerRot::XYZ, 0.3, -1.1, 2.0);
        let NumericSnapshot::Vec3(degrees) = original.numbers() else {
            panic!("expected degrees");
        };
        let mut copy = original;
        copy.edit(&NumericEdit::Slot {
            slot: 0,
            number: degrees[0],
            space: None,
        })
        .unwrap();
        assert!(original.dot(copy).abs() > 1.0 - 1e-5);
        assert!((copy.length() - 1.0).abs() < 1e-6);
        let mut quarter_turn = Quat::IDENTITY;
        quarter_turn
            .edit(&NumericEdit::Slot {
                slot: 1,
                number: 90.0,
                space: None,
            })
            .unwrap();
        assert!(
            quarter_turn
                .dot(Quat::from_rotation_y(90_f32.to_radians()))
                .abs()
                > 1.0 - 1e-5
        );
        let NumericSnapshot::Vec3([x, y, z]) = quarter_turn.numbers() else {
            panic!("expected degrees");
        };
        assert!(x.abs() < 1e-3 && (y - 90.0).abs() < 1e-3 && z.abs() < 1e-3);
    }
}
