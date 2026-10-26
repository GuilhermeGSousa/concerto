use concerto_color::Color;
use concerto_ecs::{
    command::CommandQueue,
    component::Component,
    entity::Entity,
    events::{Event, event_reader::EventReader, event_writer::EventWriter},
    query::{
        Query,
        filter::{Added, With},
    },
    resource::{Res, ResMut},
};
use concerto_window::{
    clipboard::{ClipboardPasted, WindowClipboard},
    input::{Input, InputState, KeyCode, PhysicalKey},
};

use crate::{
    focus::FocusedWidget,
    material::UIMaterial,
    node::{Position, UINode},
    text::UIText,
    transform::UIValue,
};

/// A single-line text input widget; with a `UIText` and a `UINode` it is drawn by a label child that holds that text.
#[derive(Component)]
pub struct UITextInput {
    /// Current string value of the field.
    pub value: String,
    /// Shown when the field is empty.
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

#[derive(Component)]
pub(crate) struct UITextInputParts {
    #[expect(dead_code)]
    pub(crate) selection: Entity,
    pub(crate) label: Entity,
    #[expect(dead_code)]
    pub(crate) caret: Entity,
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

/// Processes keyboard input for focused [`UITextInput`] widgets.
pub(crate) fn update_text_inputs(
    mut focused: ResMut<FocusedWidget>,
    input: Res<Input>,
    mut clipboard: ResMut<WindowClipboard>,
    text_inputs: Query<(Entity, &mut UITextInput)>,
    mut writer: EventWriter<UITextInputChanged>,
    mut submitted: EventWriter<UITextInputSubmitted>,
    mut cancelled: EventWriter<UITextInputCancelled>,
    mut pasted: EventReader<ClipboardPasted>,
) {
    let pastes: Vec<String> = pasted
        .read()
        .filter(|paste| !paste.0.is_empty())
        .map(|paste| single_line(&paste.0))
        .collect();
    for (entity, mut text_input) in text_inputs.iter() {
        if **focused == Some(entity) {
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
                clipboard.request_paste();
            }
            for paste in &pastes {
                delete_selection(&mut text_input);
                let cursor = text_input.cursor;
                text_input.value.insert_str(cursor, paste);
                text_input.cursor += paste.len();
                changed = true;
            }

            for &c in input.typed_chars().iter().filter(|_| !command) {
                delete_selection(&mut text_input);
                let cursor = text_input.cursor;
                text_input.value.insert(cursor, c);
                text_input.cursor += c.len_utf8();
                changed = true;
            }

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
                    if **focused == Some(entity) {
                        **focused = None;
                    }
                }
                None => {}
            }
        }
    }
}

fn single_line(text: &str) -> String {
    text.replace("\r\n", " ").replace(['\r', '\n'], " ")
}

fn shown_text(input: &UITextInput) -> &str {
    if input.value.is_empty() {
        &input.placeholder
    } else {
        &input.value
    }
}

/// Moves each new text field's text onto a label child between a selection quad and a caret quad.
pub(crate) fn setup_text_inputs(
    fields: Query<(Entity, &UITextInput, &UIText), (Added<UITextInput>, With<UINode>)>,
    mut cmd: CommandQueue,
) {
    for (entity, input, text) in fields.iter() {
        let line_height = UIValue::Px(text.line_height);
        let quad = |width: f32| {
            (
                UINode::default()
                    .with_position(Position::Absolute)
                    .with_size(UIValue::Px(width), line_height),
                UIMaterial::flat(Color::TRANSPARENT),
            )
        };
        let label_text = UIText {
            text: shown_text(input).to_owned(),
            ..text.clone()
        };
        let mut field = cmd.entity(entity);
        let selection = field.spawn_child(quad(0.0)).entity();
        let label = field
            .spawn_child((
                UINode::default().with_min_width(UIValue::Px(0.0)),
                label_text,
            ))
            .entity();
        let caret = field.spawn_child(quad(1.0)).entity();
        field.insert(UITextInputParts {
            selection,
            label,
            caret,
        });
        cmd.remove::<UIText>(entity);
    }
}

/// Shows each text field's value on its label, or its placeholder while the value is empty.
pub(crate) fn project_text_inputs(
    fields: Query<(&UITextInput, &UITextInputParts)>,
    labels: Query<&mut UIText>,
) {
    for (input, parts) in fields.iter() {
        let Some(mut text) = labels.get_entity(parts.label) else {
            continue;
        };
        let shown = shown_text(input);
        if text.text != shown {
            text.text = shown.to_owned();
        }
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
    use concerto_ecs::{
        IntoSetConfig, IntoSystem, IntoSystemConfig, Schedule, System, World,
        entity::hierarchy::{ChildOf, Children},
        events::event_channel::EventChannel,
        system::{executor::single_thread::SingleThreadedExecutor, schedule::CompiledSchedule},
    };
    use concerto_window::{clipboard::answer_paste_requests, plugin::Window};

    use super::*;
    use crate::{
        anchor::UIPanelStack,
        elements::prelude::*,
        node::{
            AlignItems, FlexDirection, UILayout, UILayoutDiagnostics, UILayoutEngine,
            UITextMeasure, compute_ui_nodes, install_layout_listeners,
        },
        sets::UiSet,
        text::fonts::{UIFonts, build_font_system},
        theme::UITheme,
    };

    struct Fields {
        world: World,
        schedule: CompiledSchedule,
        root: Entity,
    }

    impl Fields {
        fn new() -> Self {
            let mut world = World::new();
            world.insert_resource(Window::new((800, 600), 1.0));
            world.insert_resource(UILayoutEngine::default());
            world.insert_resource(UILayoutDiagnostics::default());
            world.insert_resource(UIPanelStack::default());
            world.insert_resource(UITextMeasure::new(build_font_system(&UIFonts::default())));
            install_layout_listeners(&mut world);
            let mut schedule = Schedule::new();
            schedule.configure_sets((UiSet::Setup, UiSet::Project, UiSet::Layout).chain());
            schedule.add_system(setup_text_inputs.in_set(UiSet::Setup));
            schedule.add_system(project_text_inputs.in_set(UiSet::Project));
            schedule.add_system(compute_ui_nodes.in_set(UiSet::Layout));
            let schedule = schedule.compile::<SingleThreadedExecutor>(&mut world);
            let root = world.spawn(
                UINode::default()
                    .with_size(UIValue::Px(800.0), UIValue::Px(600.0))
                    .with_flex_direction(FlexDirection::Column)
                    .with_align_items(AlignItems::Start),
            );
            Self {
                world,
                schedule,
                root,
            }
        }

        fn spawn(&mut self, bundle: impl concerto_ecs::component::bundle::IntoBundle) -> Entity {
            let entity = self.world.spawn(bundle);
            self.world.insert(ChildOf::new(self.root), entity);
            entity
        }

        fn frame(&mut self) {
            self.schedule.run(&mut self.world);
            self.world.tick();
        }

        fn children(&self, entity: Entity) -> Vec<Entity> {
            self.world
                .get_component_for_entity::<Children>(entity)
                .map(|children| children.iter().copied().collect())
                .unwrap_or_default()
        }

        fn get<T: Component>(&self, entity: Entity) -> &T {
            self.world
                .get_component_for_entity::<T>(entity)
                .unwrap_or_else(|| panic!("{entity:?} has no {}", std::any::type_name::<T>()))
        }

        fn label(&self, field: Entity) -> Entity {
            self.children(field)[1]
        }

        fn set_value(&mut self, field: Entity, value: &str) {
            self.world
                .get_component_for_entity_mut::<UITextInput>(field)
                .unwrap()
                .value = value.into();
        }
    }

    #[test]
    fn a_text_field_gets_a_selection_a_label_and_a_caret() {
        let t = UITheme::nocturne();
        let mut fields = Fields::new();
        let field = fields.spawn(t.text_field("Name").small().value("Alice"));
        fields.frame();

        assert!(
            fields
                .world
                .get_component_for_entity::<UIText>(field)
                .is_none(),
            "the text moves to the label"
        );
        let children = fields.children(field);
        assert_eq!(children.len(), 3);
        let (selection, label, caret) = (children[0], children[1], children[2]);

        let text = fields.get::<UIText>(label);
        assert_eq!(text.text, "Alice");
        assert_eq!(text.font_size, t.font_size_sm);
        assert_eq!(text.color, t.text);
        assert!(!text.wrap);
        assert_eq!(fields.get::<UINode>(label).min_width, UIValue::Px(0.0));
        assert_eq!(fields.get::<UINode>(label).position, Position::Relative);

        for quad in [selection, caret] {
            assert_eq!(fields.get::<UINode>(quad).position, Position::Absolute);
            assert_eq!(
                fields.get::<UINode>(quad).height,
                UIValue::Px(text.line_height)
            );
            assert!(
                fields
                    .world
                    .get_component_for_entity::<UIText>(quad)
                    .is_none()
            );
        }
        assert_eq!(fields.get::<UINode>(selection).width, UIValue::Px(0.0));
        assert_eq!(fields.get::<UINode>(caret).width, UIValue::Px(1.0));
    }

    #[test]
    fn an_input_without_text_gets_no_children() {
        let mut fields = Fields::new();
        let input = fields.spawn((UINode::default(), UITextInput::new("")));
        fields.frame();

        assert!(fields.children(input).is_empty());
    }

    #[test]
    fn the_label_shows_the_placeholder_while_the_value_is_empty() {
        let t = UITheme::nocturne();
        let mut fields = Fields::new();
        let field = fields.spawn(t.text_field("Search…"));
        fields.frame();
        let label = fields.label(field);
        assert_eq!(fields.get::<UIText>(label).text, "Search…");

        fields.set_value(field, "bunny");
        fields.frame();
        assert_eq!(fields.get::<UIText>(label).text, "bunny");

        fields.set_value(field, "");
        fields.frame();
        assert_eq!(fields.get::<UIText>(label).text, "Search…");
    }

    #[test]
    fn a_value_wider_than_the_field_is_clipped_to_the_field() {
        let t = UITheme::nocturne();
        let mut fields = Fields::new();
        let field = fields.spawn(
            t.text_field("")
                .width(UIValue::Px(80.0))
                .value("a value much wider than eighty logical pixels"),
        );
        fields.frame();
        fields.frame();

        let rect = fields.get::<UILayout>(field).rect;
        let label = fields.get::<UILayout>(fields.label(field));
        let clip = label.clip_rect.intersection(label.content_rect);
        assert!(clip.has_area(), "{clip:?}");
        assert!(
            clip.min.x >= rect.min.x && clip.min.y >= rect.min.y,
            "{clip:?} in {rect:?}"
        );
        assert!(
            clip.max().x <= rect.max().x && clip.max().y <= rect.max().y,
            "{clip:?} in {rect:?}"
        );
    }

    struct Editing {
        world: World,
        edit: Box<dyn System<In = ()>>,
        answer: Box<dyn System<In = ()>>,
    }

    impl Editing {
        fn new() -> Self {
            let mut world = World::new();
            world.insert_resource(FocusedWidget::default());
            world.insert_resource(Input::new());
            world.insert_resource(WindowClipboard::default());
            world.insert_resource(EventChannel::<UITextInputChanged>::default());
            world.insert_resource(EventChannel::<UITextInputSubmitted>::default());
            world.insert_resource(EventChannel::<UITextInputCancelled>::default());
            world.insert_resource(EventChannel::<ClipboardPasted>::default());
            let mut edit: Box<dyn System<In = ()>> = Box::new(update_text_inputs.into_system());
            edit.initialize(&mut world);
            let mut answer: Box<dyn System<In = ()>> =
                Box::new(answer_paste_requests.into_system());
            answer.initialize(&mut world);
            Self {
                world,
                edit,
                answer,
            }
        }

        fn field(&mut self, value: &str, cursor: usize, anchor: Option<usize>) -> Entity {
            let mut input = UITextInput::new("");
            input.value = value.into();
            input.cursor = cursor;
            input.selection_anchor = anchor;
            self.world.spawn(input)
        }

        fn focus(&mut self, field: Option<Entity>) {
            **self.world.get_resource_mut::<FocusedWidget>().unwrap() = field;
        }

        fn paste(&mut self, text: &str) {
            self.world
                .get_resource_mut::<EventChannel<ClipboardPasted>>()
                .unwrap()
                .push_event(ClipboardPasted(text.into()));
        }

        fn input(&mut self) -> &mut Input {
            self.world.get_resource_mut::<Input>().unwrap()
        }

        fn frame(&mut self) {
            self.answer.run_and_apply((), &mut self.world);
            self.edit.run_and_apply((), &mut self.world);
            self.input().update();
        }

        fn value(&self, field: Entity) -> &str {
            &self
                .world
                .get_component_for_entity::<UITextInput>(field)
                .unwrap()
                .value
        }

        fn cursor(&self, field: Entity) -> usize {
            self.world
                .get_component_for_entity::<UITextInput>(field)
                .unwrap()
                .cursor
        }
    }

    #[test]
    fn a_paste_lands_in_the_focused_field_with_line_breaks_as_spaces() {
        let mut editing = Editing::new();
        let other = editing.field("other", 0, None);
        let field = editing.field("ab", 1, None);
        editing.focus(Some(field));
        editing.paste("x\ny\r\nz");
        editing.frame();

        assert_eq!(editing.value(field), "ax y zb");
        assert_eq!(editing.cursor(field), 6);
        assert_eq!(editing.value(other), "other");
    }

    #[test]
    fn a_paste_replaces_the_selection() {
        let mut editing = Editing::new();
        let field = editing.field("abcd", 3, Some(1));
        editing.focus(Some(field));
        editing.paste("X");
        editing.frame();

        assert_eq!(editing.value(field), "aXd");
        assert_eq!(editing.cursor(field), 2);
    }

    #[test]
    fn a_paste_without_a_focused_field_is_dropped() {
        let mut editing = Editing::new();
        let field = editing.field("ab", 1, None);
        editing.paste("x");
        editing.frame();
        assert_eq!(editing.value(field), "ab");

        editing.focus(Some(field));
        editing.frame();
        assert_eq!(editing.value(field), "ab");
    }

    #[test]
    fn an_empty_paste_keeps_the_selection() {
        let mut editing = Editing::new();
        let field = editing.field("abc", 3, Some(0));
        editing.focus(Some(field));
        editing.paste("");
        editing.frame();

        assert_eq!(editing.value(field), "abc");
        let input = editing
            .world
            .get_component_for_entity::<UITextInput>(field)
            .unwrap();
        assert_eq!((input.cursor, input.selection_anchor), (3, Some(0)));
    }

    #[test]
    fn command_v_pastes_the_clipboard_on_a_later_frame() {
        let mut editing = Editing::new();
        let field = editing.field("", 0, None);
        editing.focus(Some(field));
        editing
            .world
            .get_resource_mut::<WindowClipboard>()
            .unwrap()
            .write("copied");
        editing
            .input()
            .update_key_input(PhysicalKey::Code(KeyCode::ControlLeft), true);
        editing
            .input()
            .update_key_input(PhysicalKey::Code(KeyCode::KeyV), true);
        editing.frame();
        assert_eq!(editing.value(field), "");

        editing.frame();
        assert_eq!(editing.value(field), "copied");
        editing.frame();
        assert_eq!(editing.value(field), "copied");
    }

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
