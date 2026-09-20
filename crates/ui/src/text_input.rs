use ecs::{
    component::Component,
    entity::Entity,
    events::{Event, event_writer::EventWriter},
    query::Query,
    resource::{Res, ResMut, Resource},
};
use window::input::{Input, InputState, KeyCode, PhysicalKey};

use crate::{focus::FocusedWidget, text::TextComponent};

/// A single-line text input widget.
///
/// Place this on an entity that also has [`UINode`](crate::node::UINode),
/// [`UIMaterial`], [`TextComponent`], and
/// [`Interactable`](crate::interaction::Interactable).
///
/// The `update_text_inputs` system reads typed characters from `Input` when
/// this entity has focus, updates `TextComponent::text` each frame (showing
/// a `|` cursor when focused, or the placeholder when empty and unfocused),
/// and fires [`UITextInputChanged`] whenever `value` changes.
#[derive(Component)]
pub struct UITextInput {
    /// Current string value of the field.
    pub value: String,
    /// Shown when the field is empty and unfocused.
    pub placeholder: String,
    /// Byte offset of the insert cursor within `value`.
    pub cursor: usize,
    pub selection_anchor: Option<usize>,
}

impl UITextInput {
    pub fn new(placeholder: impl Into<String>) -> Self {
        Self {
            value: String::new(),
            placeholder: placeholder.into(),
            cursor: 0,
            selection_anchor: None,
        }
    }
}

/// Fired whenever a [`UITextInput`]'s value changes.
#[derive(Event)]
pub struct UITextInputChanged {
    pub entity: Entity,
    pub value: String,
}

/// Fired when Enter is pressed in a focused [`UITextInput`].
#[derive(Event)]
pub struct UITextInputSubmitted {
    pub entity: Entity,
    pub value: String,
}

/// Fired when Escape is pressed in a focused [`UITextInput`].
#[derive(Event)]
pub struct UITextInputCancelled {
    pub entity: Entity,
}

#[derive(Debug, PartialEq, Eq)]
enum Finish {
    Submit,
    Cancel,
}

fn finish_key(just_pressed: impl Fn(KeyCode) -> bool) -> Option<Finish> {
    if just_pressed(KeyCode::Enter) || just_pressed(KeyCode::NumpadEnter) {
        Some(Finish::Submit)
    } else if just_pressed(KeyCode::Escape) {
        Some(Finish::Cancel)
    } else {
        None
    }
}

/// Dummy resource marker so the event can be registered.
#[derive(Resource)]
pub struct TextInputResource;

/// Processes keyboard input for focused [`UITextInput`] widgets.
///
/// - Typed printable characters are inserted at the cursor.
/// - Backspace (on press) removes the character before the cursor.
/// - Left/Right arrow keys move the cursor by one `char`.
/// - `TextComponent::text` is updated every frame to reflect the current
///   value + cursor indicator (focused) or placeholder (empty + unfocused).
pub(crate) fn update_text_inputs(
    focused: Res<FocusedWidget>,
    input: Res<Input>,
    mut clipboard: ResMut<window::plugin::WindowClipboard>,
    text_inputs: Query<(Entity, &mut UITextInput, &mut TextComponent)>,
    mut writer: EventWriter<UITextInputChanged>,
    mut submitted: EventWriter<UITextInputSubmitted>,
    mut cancelled: EventWriter<UITextInputCancelled>,
) {
    for (entity, mut text_input, mut text) in text_inputs.iter() {
        let is_focused = **focused == Some(entity);

        if is_focused {
            let mut changed = false;
            let command = input.is_held(PhysicalKey::Code(KeyCode::ControlLeft))
                || input.is_held(PhysicalKey::Code(KeyCode::ControlRight))
                || input.is_held(PhysicalKey::Code(KeyCode::SuperLeft))
                || input.is_held(PhysicalKey::Code(KeyCode::SuperRight));
            let shift = input.is_held(PhysicalKey::Code(KeyCode::ShiftLeft))
                || input.is_held(PhysicalKey::Code(KeyCode::ShiftRight));

            if command && input.is_just_pressed(PhysicalKey::Code(KeyCode::KeyA)) {
                text_input.selection_anchor = Some(0);
                text_input.cursor = text_input.value.len();
            }
            if command
                && input.is_just_pressed(PhysicalKey::Code(KeyCode::KeyC))
                && let Some((start, end)) = selection(&text_input)
            {
                clipboard.write(text_input.value[start..end].to_owned());
            }
            if command
                && input.is_just_pressed(PhysicalKey::Code(KeyCode::KeyX))
                && let Some((start, end)) = selection(&text_input)
            {
                clipboard.write(text_input.value[start..end].to_owned());
                text_input.value.replace_range(start..end, "");
                text_input.cursor = start;
                text_input.selection_anchor = None;
                changed = true;
            }
            if command && input.is_just_pressed(PhysicalKey::Code(KeyCode::KeyV)) {
                delete_selection(&mut text_input);
                let value = clipboard.read().to_owned();
                let cursor = text_input.cursor;
                text_input.value.insert_str(cursor, &value);
                text_input.cursor += value.len();
                changed = true;
            }

            // --- printable characters ---
            for &c in input.typed_chars().iter().filter(|_| !command) {
                delete_selection(&mut text_input);
                let cursor = text_input.cursor;
                text_input.value.insert(cursor, c);
                text_input.cursor += c.len_utf8();
                changed = true;
            }

            // --- backspace: delete character before cursor ---
            let backspace = input.get_key_state(PhysicalKey::Code(KeyCode::Backspace));
            if backspace == InputState::Pressed && text_input.cursor > 0 {
                if !delete_selection(&mut text_input) {
                    let pos = previous_boundary(&text_input.value, text_input.cursor);
                    text_input.value.remove(pos);
                    text_input.cursor = pos;
                }
                changed = true;
            }

            if input.get_key_state(PhysicalKey::Code(KeyCode::Delete)) == InputState::Pressed {
                if delete_selection(&mut text_input) {
                    changed = true;
                } else if text_input.cursor < text_input.value.len() {
                    let end = next_boundary(&text_input.value, text_input.cursor);
                    let cursor = text_input.cursor;
                    text_input.value.replace_range(cursor..end, "");
                    changed = true;
                }
            }

            // --- arrow keys: move cursor ---
            let left = input.get_key_state(PhysicalKey::Code(KeyCode::ArrowLeft));
            if left == InputState::Pressed && text_input.cursor > 0 {
                begin_or_clear_selection(&mut text_input, shift);
                text_input.cursor = previous_boundary(&text_input.value, text_input.cursor);
            }

            let right = input.get_key_state(PhysicalKey::Code(KeyCode::ArrowRight));
            if right == InputState::Pressed && text_input.cursor < text_input.value.len() {
                begin_or_clear_selection(&mut text_input, shift);
                text_input.cursor = next_boundary(&text_input.value, text_input.cursor);
            }
            if input.is_just_pressed(PhysicalKey::Code(KeyCode::Home)) {
                begin_or_clear_selection(&mut text_input, shift);
                text_input.cursor = 0;
            }
            if input.is_just_pressed(PhysicalKey::Code(KeyCode::End)) {
                begin_or_clear_selection(&mut text_input, shift);
                text_input.cursor = text_input.value.len();
            }

            if changed {
                writer.write(UITextInputChanged {
                    entity,
                    value: text_input.value.clone(),
                });
            }

            match finish_key(|key| input.is_just_pressed(PhysicalKey::Code(key))) {
                Some(Finish::Submit) => {
                    submitted.write(UITextInputSubmitted {
                        entity,
                        value: text_input.value.clone(),
                    });
                }
                Some(Finish::Cancel) => {
                    cancelled.write(UITextInputCancelled { entity });
                }
                None => {}
            }
        }

        // --- update displayed text and border colour ---
        let display = if is_focused {
            let mut s = text_input.value.clone();
            s.insert(text_input.cursor, '|');
            s
        } else if text_input.value.is_empty() {
            text_input.placeholder.clone()
        } else {
            text_input.value.clone()
        };

        if text.text != display {
            text.text = display;
        }

        // The caret says where focus is; a field keeps whatever border the
        // panel gave it.
    }
}

fn selection(input: &UITextInput) -> Option<(usize, usize)> {
    let anchor = input.selection_anchor?;
    (anchor != input.cursor).then_some((anchor.min(input.cursor), anchor.max(input.cursor)))
}

fn delete_selection(input: &mut UITextInput) -> bool {
    let Some((start, end)) = selection(input) else {
        return false;
    };
    input.value.replace_range(start..end, "");
    input.cursor = start;
    input.selection_anchor = None;
    true
}

fn begin_or_clear_selection(input: &mut UITextInput, shift: bool) {
    if shift {
        input.selection_anchor.get_or_insert(input.cursor);
    } else {
        input.selection_anchor = None;
    }
}

fn previous_boundary(value: &str, cursor: usize) -> usize {
    value[..cursor]
        .char_indices()
        .next_back()
        .map_or(0, |(index, _)| index)
}

fn next_boundary(value: &str, cursor: usize) -> usize {
    value[cursor..]
        .char_indices()
        .nth(1)
        .map_or(value.len(), |(offset, _)| cursor + offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_boundaries_and_selection_are_safe() {
        let value = "aé東";
        assert_eq!(previous_boundary(value, value.len()), 3);
        assert_eq!(next_boundary(value, 1), 3);
        let mut input = UITextInput::new("");
        input.value = value.into();
        input.cursor = value.len();
        input.selection_anchor = Some(1);
        assert!(delete_selection(&mut input));
        assert_eq!(input.value, "a");
        assert_eq!(input.cursor, 1);
    }

    #[test]
    fn enter_submits_and_escape_cancels() {
        assert_eq!(
            finish_key(|key| key == KeyCode::Enter),
            Some(Finish::Submit)
        );
        assert_eq!(
            finish_key(|key| key == KeyCode::NumpadEnter),
            Some(Finish::Submit)
        );
        assert_eq!(
            finish_key(|key| key == KeyCode::Escape),
            Some(Finish::Cancel)
        );
        assert_eq!(finish_key(|key| key == KeyCode::KeyA), None);
    }
}
