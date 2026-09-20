use ecs::{
    command::CommandQueue, events::event_reader::EventReader, Component, Entity, Query, Res, ResMut,
};
use editable::Editable;
use glam::{EulerRot, Quat, Vec3};
use ui::{
    focus::{FocusedWidget, UIFocusGained, UIFocusLost, UIFocusable},
    interaction::Interactable,
    material::UIMaterial,
    node::{UINode, UIRect},
    text::TextComponent,
    text_input::{UITextInput, UITextInputCancelled, UITextInputSubmitted},
    theme::UITheme,
    transform::UIValue,
};

use crate::inspector::rows::{
    EditError, PropertyCommits, PropertyEditor, PropertyRow, PropertyRowValue,
};

/// One text field per number: one for `Number`, three for `Vec3`.
pub(crate) struct NumericFields;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum NumericSnapshot {
    Number(f64),
    Vec3([f64; 3]),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct NumericEdit {
    pub slot: usize,
    pub number: f64,
}

pub(crate) fn register_defaults(registry: &mut super::registry::InspectorRegistry) {
    registry.register_property_editor::<f32, _>(NumericFields);
    registry.register_property_editor::<f64, _>(NumericFields);
    registry.register_property_editor::<Vec3, _>(NumericFields);
    registry.register_property_editor::<Quat, _>(NumericFields);
}

pub(crate) trait NumericValue: Editable {
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
                let number = edit.number as $ty;
                if edit.slot != 0 || !number.is_finite() {
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

impl NumericValue for Vec3 {
    fn numbers(&self) -> NumericSnapshot {
        NumericSnapshot::Vec3([self.x as f64, self.y as f64, self.z as f64])
    }
    fn edit(&mut self, edit: &NumericEdit) -> Result<(), EditError> {
        let number = edit.number as f32;
        if edit.slot >= 3 || !number.is_finite() {
            return Err(EditError::Rejected);
        }
        let mut next = *self;
        next[edit.slot] = number;
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
    if row.type_id == TypeId::of::<f32>() {
        commits.push::<f32, NumericFields>(row, edit)
    } else if row.type_id == TypeId::of::<f64>() {
        commits.push::<f64, NumericFields>(row, edit)
    } else if row.type_id == TypeId::of::<Vec3>() {
        commits.push::<Vec3, NumericFields>(row, edit)
    } else if row.type_id == TypeId::of::<Quat>() {
        commits.push::<Quat, NumericFields>(row, edit)
    } else {
        Err(EditError::TypeMismatch)
    }
}

/// One field of a [`NumericFields`] row.
#[derive(Component)]
pub(crate) struct NumericSlot {
    row: Entity,
    slot: usize,
    /// The text this field last showed or committed. A commit compares its
    /// input against this, so neither display rounding nor a repeated focus
    /// loss becomes an edit.
    displayed: String,
}

impl<T: NumericValue> PropertyEditor<T> for NumericFields {
    type Snapshot = NumericSnapshot;
    type Edit = NumericEdit;
    fn snapshot(&self, value: &T) -> NumericSnapshot {
        value.numbers()
    }
    fn apply(&self, value: &mut T, edit: &NumericEdit) -> Result<(), EditError> {
        value.edit(edit)
    }
    fn build(&self, cmd: &mut CommandQueue, row: Entity, value: &NumericSnapshot, theme: &UITheme) {
        for slot in 0..slot_count(value) {
            let field = cmd
                .spawn((
                    UINode {
                        flex_grow: 1.0,
                        width: UIValue::Px(0.0),
                        min_width: UIValue::Px(0.0),
                        height: UIValue::Px(theme.control_height),
                        padding: UIRect::axes(field_leading(theme), theme.spacing_xs),
                        ..Default::default()
                    }
                    .clipped(),
                    TextComponent {
                        color: theme.text,
                        font_size: theme.font_size_sm,
                        line_height: theme.line_height(theme.font_size_sm),
                        wrap: false,
                        ..Default::default()
                    },
                    UITextInput::new(""),
                    UIMaterial {
                        corner_radius: theme.radius_md,
                        ..UIMaterial::with_border(theme.canvas, theme.border, 1.0)
                    },
                    Interactable,
                    UIFocusable,
                    NumericSlot {
                        row,
                        slot,
                        displayed: String::new(),
                    },
                ))
                .entity();
            cmd.add_child(row, field);
        }
    }
}

fn field_leading(theme: &UITheme) -> f32 {
    let line = theme.line_height(theme.font_size_sm);
    ((theme.control_height - line) / 2.0).max(0.0)
}

fn slot_count(value: &NumericSnapshot) -> usize {
    match value {
        NumericSnapshot::Number(_) => 1,
        NumericSnapshot::Vec3(_) => 3,
    }
}

fn format_slot(value: &NumericSnapshot, slot: usize) -> String {
    let number = match value {
        NumericSnapshot::Number(number) => (slot == 0).then_some(*number),
        NumericSnapshot::Vec3(components) => components.get(slot).copied(),
    };
    number.map(|n| format!("{n:.3}")).unwrap_or_default()
}

/// The value to commit when `slot` finishes editing with `text`, or `None` to
/// commit nothing.
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
    if !number.is_finite() {
        return None;
    }
    Some(NumericEdit { slot, number })
}

/// Restores a field's text to its last-known-good value, e.g. after an
/// unparseable edit or an explicit cancel.
fn revert(slot: &NumericSlot, input: &mut UITextInput) {
    input.value = slot.displayed.clone();
    input.cursor = input.value.len();
    input.selection_anchor = None;
}

/// Selects a field's whole text when it gains focus, so the first keystroke
/// replaces it rather than landing after it. `UITextInput`'s typing path
/// deletes the current selection before inserting.
pub(crate) fn select_numeric_field_on_focus(
    mut gained: EventReader<UIFocusGained>,
    fields: Query<(&NumericSlot, &mut UITextInput)>,
) {
    for event in gained.read() {
        let Some((_, mut input)) = fields.get_entity(event.0) else {
            continue;
        };
        select_all(&mut input);
    }
}

fn select_all(input: &mut UITextInput) {
    input.selection_anchor = Some(0);
    input.cursor = input.value.len();
}

pub(crate) fn commit_numeric_fields(
    mut submitted: EventReader<UITextInputSubmitted>,
    mut lost: EventReader<UIFocusLost>,
    rows: Query<(&PropertyRow, &PropertyRowValue)>,
    fields: Query<(&mut NumericSlot, &mut UITextInput)>,
    mut commits: ResMut<PropertyCommits>,
) {
    let finished: Vec<Entity> = submitted
        .read()
        .map(|event| event.entity)
        .chain(lost.read().map(|event| event.0))
        .collect();
    for entity in finished {
        let Some((mut slot, mut input)) = fields.get_entity(entity) else {
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
                // Enter keeps focus; without this the focus loss that follows
                // would commit the same text again.
                slot.displayed = input.value.clone();
            }
            // Unparseable text never commits and must not linger on screen:
            // Enter keeps focus, and refresh skips the focused field, so
            // nothing else would clear it.
            None => revert(&slot, &mut input),
        }
    }
}

pub(crate) fn cancel_numeric_fields(
    mut cancelled: EventReader<UITextInputCancelled>,
    fields: Query<(&NumericSlot, &mut UITextInput)>,
    mut focused: ResMut<FocusedWidget>,
) {
    for event in cancelled.read() {
        let Some((slot, mut input)) = fields.get_entity(event.entity) else {
            continue;
        };
        revert(&slot, &mut input);
        if **focused == Some(event.entity) {
            **focused = None;
        }
    }
}

/// Writes each row's value into its fields, except the focused one: its text is
/// the user's edit buffer, and the value re-derived from the component (e.g.
/// euler angles from a quaternion) need not match what they typed.
pub(crate) fn refresh_numeric_fields(
    focused: Res<FocusedWidget>,
    rows: Query<&PropertyRowValue>,
    fields: Query<(Entity, &mut NumericSlot, &mut UITextInput)>,
) {
    for (entity, mut slot, mut input) in fields.iter() {
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
    fn unparseable_text_does_not_commit() {
        let value = NumericSnapshot::Number(1.0);
        assert_eq!(numeric_commit(&value, 0, "abc", "1.000"), None);
        assert_eq!(numeric_commit(&value, 0, "", "1.000"), None);
    }

    #[test]
    fn a_slot_is_replaced_within_a_vec3() {
        let value = NumericSnapshot::Vec3([1.0, 2.0, 3.0]);
        assert_eq!(
            numeric_commit(&value, 1, " -4.5 ", "2.000"),
            Some(NumericEdit {
                slot: 1,
                number: -4.5
            })
        );
    }

    #[test]
    fn focus_selects_the_whole_field() {
        let mut input = UITextInput::new("");
        input.value = "1.000".into();
        input.cursor = 5;
        input.selection_anchor = None;

        select_all(&mut input);

        assert_eq!(input.selection_anchor, Some(0));
        assert_eq!(input.cursor, input.value.len());
    }

    /// The bug this guards: a field with no vertical padding drew its text
    /// against the top edge of a 28px-tall box, because the renderer starts at
    /// the content box's top-left and nothing had moved it down.
    #[test]
    fn a_fields_text_sits_in_the_middle_of_its_box() {
        let theme = UITheme::default();
        let leading = field_leading(&theme);
        let line = theme.line_height(theme.font_size_sm);
        assert!(leading > 0.0, "a line shorter than the control needs room");
        assert_eq!(
            leading * 2.0 + line,
            theme.control_height,
            "the line and the space above and below it fill the control exactly"
        );

        // A theme whose text is taller than its controls cannot be centred, and
        // must not be pushed out of its box trying.
        let cramped = UITheme {
            control_height: 8.0,
            ..theme
        };
        assert_eq!(field_leading(&cramped), 0.0);
    }

    #[test]
    fn a_number_has_one_slot() {
        let value = NumericSnapshot::Number(1.0);
        assert_eq!(
            numeric_commit(&value, 0, "7", "1.000"),
            Some(NumericEdit {
                slot: 0,
                number: 7.0
            })
        );
        assert_eq!(numeric_commit(&value, 1, "7", "1.000"), None);
        assert_eq!(slot_count(&value), 1);
        assert_eq!(slot_count(&NumericSnapshot::Vec3([0.0; 3])), 3);
    }
    #[test]
    fn numeric_adapters_validate_before_mutating() {
        let mut small = 1.0_f32;
        let mut large = 2.0_f64;
        let mut vector = Vec3::ONE;
        let mut rotation = Quat::IDENTITY;
        for number in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let edit = NumericEdit { slot: 0, number };
            assert_eq!(small.edit(&edit), Err(EditError::Rejected));
            assert_eq!(large.edit(&edit), Err(EditError::Rejected));
            assert_eq!(vector.edit(&edit), Err(EditError::Rejected));
            assert_eq!(rotation.edit(&edit), Err(EditError::Rejected));
        }
        let overflow = NumericEdit {
            slot: 0,
            number: 1e300,
        };
        assert_eq!(small.edit(&overflow), Err(EditError::Rejected));
        assert_eq!(vector.edit(&overflow), Err(EditError::Rejected));
        assert_eq!(rotation.edit(&overflow), Err(EditError::Rejected));
        let bad_slot = NumericEdit {
            slot: 3,
            number: 0.0,
        };
        assert_eq!(small.edit(&bad_slot), Err(EditError::Rejected));
        assert_eq!(large.edit(&bad_slot), Err(EditError::Rejected));
        assert_eq!(vector.edit(&bad_slot), Err(EditError::Rejected));
        assert_eq!(rotation.edit(&bad_slot), Err(EditError::Rejected));
        assert_eq!(small, 1.0);
        assert_eq!(large, 2.0);
        assert_eq!(vector, Vec3::ONE);
        assert_eq!(rotation, Quat::IDENTITY);
        small
            .edit(&NumericEdit {
                slot: 0,
                number: 2.5,
            })
            .unwrap();
        large
            .edit(&NumericEdit {
                slot: 0,
                number: -7.25,
            })
            .unwrap();
        assert_eq!(small.numbers(), NumericSnapshot::Number(2.5));
        assert_eq!(large.numbers(), NumericSnapshot::Number(-7.25));
    }

    #[test]
    fn quaternion_adapter_uses_xyz_degrees_and_normalizes() {
        let original = Quat::from_euler(EulerRot::XYZ, 0.3, -1.1, 2.0);
        let NumericSnapshot::Vec3(degrees) = original.numbers() else {
            panic!("expected degrees");
        };
        let mut copy = original;
        copy.edit(&NumericEdit {
            slot: 0,
            number: degrees[0],
        })
        .unwrap();
        assert!(original.dot(copy).abs() > 1.0 - 1e-5);
        assert!((copy.length() - 1.0).abs() < 1e-6);
        let mut quarter_turn = Quat::IDENTITY;
        quarter_turn
            .edit(&NumericEdit {
                slot: 1,
                number: 90.0,
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

    #[test]
    fn non_finite_text_never_queues_an_edit() {
        for text in ["NaN", "inf", "-inf"] {
            assert_eq!(
                numeric_commit(&NumericSnapshot::Number(1.0), 0, text, "1.000"),
                None
            );
        }
    }

    #[test]
    fn transform_numeric_widget_submit_refresh_and_cancel_smoke_test() {
        use super::super::{apply_property_commits, InspectorRegistry, Property};
        use ecs::{events::event_channel::EventChannel, IntoSystem, Res, Resource, System, World};
        use essential::transform::Transform;
        use std::any::TypeId;
        use ui::{
            focus::{FocusedWidget, UIFocusLost},
            text_input::{UITextInputCancelled, UITextInputSubmitted},
        };

        #[derive(Resource)]
        struct Build {
            property: Property,
            entity: Entity,
        }
        fn build(data: Res<Build>, mut cmd: CommandQueue) {
            let row = cmd
                .spawn((
                    data.property.row(data.entity, TypeId::of::<Transform>()),
                    data.property.value.clone(),
                ))
                .entity();
            <NumericFields as PropertyEditor<Vec3>>::build(
                &NumericFields,
                &mut cmd,
                row,
                data.property
                    .value
                    .snapshot::<Vec3, NumericFields>()
                    .unwrap(),
                &UITheme::default(),
            );
        }
        let mut world = World::default();
        let entity = world.spawn(Transform::IDENTITY);
        let mut registry = InspectorRegistry::default();
        registry.register_component::<Transform>();
        let property = registry
            .collect_component(&world, entity, TypeId::of::<Transform>())
            .unwrap()
            .remove(0);
        world.insert_resource(registry);
        world.insert_resource(Build { property, entity });
        world.insert_resource(PropertyCommits::default());
        world.insert_resource(FocusedWidget::default());
        world.insert_resource(EventChannel::<UITextInputSubmitted>::default());
        world.insert_resource(EventChannel::<UIFocusLost>::default());
        world.insert_resource(EventChannel::<UITextInputCancelled>::default());
        let mut build = build.into_system();
        build.initialize(&mut world);
        build.run_and_apply(&mut world);
        let mut refresh = refresh_numeric_fields.into_system();
        refresh.initialize(&mut world);
        refresh.run_and_apply(&mut world);
        let mut fields = world.query::<(Entity, &NumericSlot), ()>();
        assert_eq!(fields.iter(&mut world).count(), 3);
        let (field, row) = fields
            .iter(&mut world)
            .find(|(_, slot)| slot.slot == 0)
            .map(|(entity, slot)| (entity, slot.row))
            .unwrap();
        assert_eq!(
            world
                .get_component_for_entity::<UITextInput>(field)
                .unwrap()
                .value,
            "0.000"
        );
        **world.get_resource_mut::<FocusedWidget>().unwrap() = Some(field);
        world
            .get_component_for_entity_mut::<UITextInput>(field)
            .unwrap()
            .value = "2.5".into();
        refresh.run_and_apply(&mut world);
        assert_eq!(
            world
                .get_component_for_entity::<UITextInput>(field)
                .unwrap()
                .value,
            "2.5",
            "refresh preserves focused text"
        );
        world
            .get_resource_mut::<EventChannel<UITextInputSubmitted>>()
            .unwrap()
            .push_event(UITextInputSubmitted {
                entity: field,
                value: "2.5".into(),
            });
        let mut submit = commit_numeric_fields.into_system();
        submit.initialize(&mut world);
        submit.run_and_apply(&mut world);
        apply_property_commits(&mut world);
        assert_eq!(
            world
                .get_component_for_entity::<Transform>(entity)
                .unwrap()
                .translation,
            Vec3::new(2.5, 0.0, 0.0)
        );
        let snapshot = world
            .get_resource::<InspectorRegistry>()
            .unwrap()
            .collect_component(&world, entity, TypeId::of::<Transform>())
            .unwrap()
            .remove(0)
            .value;
        *world
            .get_component_for_entity_mut::<PropertyRowValue>(row)
            .unwrap() = snapshot;
        **world.get_resource_mut::<FocusedWidget>().unwrap() = None;
        refresh.run_and_apply(&mut world);
        assert_eq!(
            world
                .get_component_for_entity::<UITextInput>(field)
                .unwrap()
                .value,
            "2.500"
        );
        world
            .get_component_for_entity_mut::<UITextInput>(field)
            .unwrap()
            .value = "unfinished".into();
        **world.get_resource_mut::<FocusedWidget>().unwrap() = Some(field);
        world
            .get_resource_mut::<EventChannel<UITextInputCancelled>>()
            .unwrap()
            .push_event(UITextInputCancelled { entity: field });
        let mut cancel = cancel_numeric_fields.into_system();
        cancel.initialize(&mut world);
        cancel.run_and_apply(&mut world);
        assert_eq!(
            world
                .get_component_for_entity::<UITextInput>(field)
                .unwrap()
                .value,
            "2.500"
        );
        assert_eq!(**world.get_resource::<FocusedWidget>().unwrap(), None);
        assert!(world
            .get_resource::<PropertyCommits>()
            .unwrap()
            .0
            .is_empty());
    }
}
