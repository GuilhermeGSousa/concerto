use std::time::Duration;

use concerto_color::{Color, LinearRgba};
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
use concerto_foundation::time::Instant;
use concerto_window::{
    clipboard::{ClipboardPasted, WindowClipboard},
    input::{Input, InputState, KeyCode, MouseButton, PhysicalKey},
    plugin::Window,
};
use glam::Vec2;
use glyphon::{
    Action, Attrs, Buffer, Cursor, Edit, Editor, FontSystem, LayoutRun, Shaping, Wrap,
    cosmic_text::{Change, Motion, Selection},
};

use crate::{
    focus::FocusedWidget,
    interaction::{DOUBLE_CLICK, DRAG_THRESHOLD, UIInputState},
    material::UIMaterial,
    node::{Position, UILayout, UINode, UITextMeasure, ui_text_attrs, ui_text_metrics},
    text::UIText,
    theme::UITheme,
    transform::UIValue,
};

const BLINK: Duration = Duration::from_millis(530);

/// A single-line text input, edited through cosmic-text while focused and selected whole when it gains focus; with a `UIText` and a `UINode` it is drawn by a label child that holds that text.
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
    pub(crate) selection: Entity,
    pub(crate) label: Entity,
    pub(crate) caret: Entity,
}

struct Written {
    value: String,
    cursor: usize,
    selection_anchor: Option<usize>,
}

impl Written {
    fn of(input: &UITextInput) -> Self {
        Self {
            value: input.value.clone(),
            cursor: input.cursor,
            selection_anchor: input.selection_anchor,
        }
    }
}

struct PointerPress {
    origin: Vec2,
    focusing: bool,
    dragging: bool,
}

#[derive(Component)]
pub(crate) struct UITextEditState {
    editor: Editor<'static>,
    attrs: Attrs<'static>,
    written: Written,
    focused: bool,
    undo: Vec<Change>,
    redo: Vec<Change>,
    typing: bool,
    blink_start: Instant,
    press_time: Option<Instant>,
    press_count: u32,
    pointer: Option<PointerPress>,
}

impl UITextEditState {
    fn new(font_system: &mut FontSystem, text: &UIText, input: &UITextInput) -> Self {
        let mut buffer = Buffer::new(font_system, ui_text_metrics(text));
        buffer.set_wrap(font_system, Wrap::None);
        let mut state = Self {
            editor: Editor::new(buffer),
            attrs: ui_text_attrs(text),
            written: Written::of(input),
            focused: false,
            undo: Vec::new(),
            redo: Vec::new(),
            typing: false,
            blink_start: Instant::now(),
            press_time: None,
            press_count: 0,
            pointer: None,
        };
        state.load(font_system, input);
        state
    }

    fn load(&mut self, font_system: &mut FontSystem, input: &UITextInput) {
        let attrs = self.attrs;
        self.editor.with_buffer_mut(|buffer| {
            buffer.set_text(font_system, &input.value, attrs, Shaping::Advanced)
        });
        self.place(input);
    }

    fn sync(&mut self, font_system: &mut FontSystem, input: &UITextInput) {
        if input.value != self.written.value {
            self.load(font_system, input);
            self.forget_history();
        } else if input.cursor != self.written.cursor
            || input.selection_anchor != self.written.selection_anchor
        {
            self.place(input);
        }
    }

    fn line_text(&self) -> String {
        self.editor
            .with_buffer(|buffer| buffer.lines.first().map(|line| line.text().to_owned()))
            .unwrap_or_default()
    }

    fn place(&mut self, input: &UITextInput) {
        let line = self.line_text();
        let cursor = char_floor(&line, input.cursor);
        let anchor = input
            .selection_anchor
            .map(|anchor| char_floor(&line, anchor));
        self.editor.set_cursor(Cursor::new(0, cursor));
        self.editor.set_selection(match anchor {
            Some(anchor) if anchor != cursor => Selection::Normal(Cursor::new(0, anchor)),
            _ => Selection::None,
        });
    }

    fn value(&self) -> String {
        self.editor.with_buffer(|buffer| {
            buffer
                .lines
                .iter()
                .map(|line| line.text())
                .collect::<Vec<_>>()
                .join("\n")
        })
    }

    fn selection_range(&self) -> Option<(Cursor, Cursor)> {
        self.editor
            .selection_bounds()
            .filter(|(start, end)| (start.line, start.index) != (end.line, end.index))
    }

    fn caret(&self) -> (usize, Option<usize>) {
        let cursor = self.editor.cursor().index;
        match self.selection_range() {
            Some((start, end)) if start.index == cursor => (cursor, Some(end.index)),
            Some((start, end)) => (end.index, Some(start.index)),
            None => (cursor, None),
        }
    }

    fn collapse_empty_selection(&mut self) {
        if self.selection_range().is_none() {
            self.editor.set_selection(Selection::None);
        }
    }

    fn select_all(&mut self) {
        self.typing = false;
        let end = self.line_text().len();
        self.editor.set_cursor(Cursor::new(0, end));
        self.editor
            .set_selection(Selection::Normal(Cursor::new(0, 0)));
        self.collapse_empty_selection();
    }

    fn motion(&mut self, font_system: &mut FontSystem, motion: Motion, extend: bool) {
        self.typing = false;
        if !extend {
            self.editor.set_selection(Selection::None);
        } else if self.editor.selection() == Selection::None {
            self.editor
                .set_selection(Selection::Normal(self.editor.cursor()));
        }
        self.editor.action(font_system, Action::Motion(motion));
        self.collapse_empty_selection();
    }

    fn edit(&mut self, typed: bool, apply: impl FnOnce(&mut Editor<'static>)) {
        self.editor.start_change();
        apply(&mut self.editor);
        let Some(change) = self
            .editor
            .finish_change()
            .filter(|change| !change.items.is_empty())
        else {
            return;
        };
        self.redo.clear();
        match self.undo.last_mut() {
            Some(run) if typed && self.typing => run.items.extend(change.items),
            _ => self.undo.push(change),
        }
        self.typing = typed;
    }

    fn delete_word(&mut self, font_system: &mut FontSystem, motion: Motion) {
        if self.selection_range().is_none() {
            self.editor
                .set_selection(Selection::Normal(self.editor.cursor()));
            self.editor.action(font_system, Action::Motion(motion));
            self.collapse_empty_selection();
        }
        self.edit(false, |editor| {
            editor.delete_selection();
        });
    }

    fn copy(&self) -> Option<String> {
        self.selection_range()
            .and_then(|_| self.editor.copy_selection())
    }

    fn undo(&mut self) {
        self.typing = false;
        if let Some(change) = self.undo.pop() {
            let mut reverse = change.clone();
            reverse.reverse();
            self.editor.set_selection(Selection::None);
            self.editor.apply_change(&reverse);
            self.redo.push(change);
        }
    }

    fn redo(&mut self) {
        self.typing = false;
        if let Some(change) = self.redo.pop() {
            self.editor.set_selection(Selection::None);
            self.editor.apply_change(&change);
            self.undo.push(change);
        }
    }

    fn hit_point(&self, at: Vec2) -> (i32, i32) {
        let line_height = self
            .editor
            .with_buffer(|buffer| buffer.metrics().line_height);
        let y = at.y.clamp(0.0, (line_height - 1.0).max(0.0));
        (at.x.round() as i32, y as i32)
    }

    fn press(
        &mut self,
        font_system: &mut FontSystem,
        at: Vec2,
        extend: bool,
        focusing: bool,
        now: Instant,
    ) {
        self.typing = false;
        self.blink_start = now;
        let count = count_press(self.press_time, self.press_count, now);
        self.press_time = Some(now);
        self.press_count = count;
        self.pointer = Some(PointerPress {
            origin: at,
            focusing,
            dragging: false,
        });
        if focusing {
            return;
        }
        let (x, y) = self.hit_point(at);
        let action = match count {
            1 if extend => Action::Drag { x, y },
            1 => Action::Click { x, y },
            2 => Action::DoubleClick { x, y },
            _ => Action::TripleClick { x, y },
        };
        self.editor.shape_as_needed(font_system, false);
        self.editor.action(font_system, action);
        self.collapse_empty_selection();
    }

    fn drag(&mut self, font_system: &mut FontSystem, at: Vec2) {
        let Some(press) = self.pointer.as_mut() else {
            return;
        };
        if !press.dragging && press.origin.distance(at) < DRAG_THRESHOLD {
            return;
        }
        let restart = press.focusing && !press.dragging;
        press.dragging = true;
        let origin = press.origin;
        self.editor.shape_as_needed(font_system, false);
        if restart {
            let (x, y) = self.hit_point(origin);
            self.editor.action(font_system, Action::Click { x, y });
        }
        let (x, y) = self.hit_point(at);
        self.editor.action(font_system, Action::Drag { x, y });
        self.collapse_empty_selection();
    }

    fn marks(&self) -> (f32, Option<(f32, f32)>) {
        let cursor = self.editor.cursor().index;
        let selection = self.selection_range();
        self.editor.with_buffer(|buffer| {
            let Some(run) = buffer.layout_runs().next() else {
                return (0.0, None);
            };
            let highlight = selection.and_then(|(start, end)| {
                run.highlight(
                    Cursor::new(start.line, start.index),
                    Cursor::new(end.line, end.index),
                )
            });
            (caret_x(&run, cursor), highlight)
        })
    }

    fn forget_history(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.typing = false;
    }
}

fn count_press(last: Option<Instant>, count: u32, now: Instant) -> u32 {
    match last {
        Some(at) if now.duration_since(at) < DOUBLE_CLICK => (count + 1).min(3),
        _ => 1,
    }
}

fn caret_x(run: &LayoutRun, index: usize) -> f32 {
    run.glyphs
        .iter()
        .find(|glyph| glyph.start >= index)
        .map(|glyph| glyph.x)
        .or_else(|| run.glyphs.last().map(|glyph| glyph.x + glyph.w))
        .unwrap_or(0.0)
}

fn caret_shown(start: Instant, now: Instant) -> bool {
    (now.duration_since(start).as_millis() / BLINK.as_millis()).is_multiple_of(2)
}

fn char_floor(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
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

fn shift_held(input: &Input) -> bool {
    input.is_held(PhysicalKey::Code(KeyCode::ShiftLeft))
        || input.is_held(PhysicalKey::Code(KeyCode::ShiftRight))
}

fn apply_pointer(
    state: &mut UITextEditState,
    font_system: &mut FontSystem,
    input: &Input,
    pointer: &UIInputState,
    entity: Entity,
    at: Vec2,
    focusing: bool,
    now: Instant,
) {
    match input.get_mouse_button_state(MouseButton::Left) {
        InputState::Pressed if pointer.pressed(MouseButton::Left) == Some(entity) => {
            state.press(font_system, at, shift_held(input), focusing, now);
        }
        InputState::Down if pointer.captured(MouseButton::Left) == Some(entity) => {
            state.drag(font_system, at);
        }
        _ => {}
    }
}

fn apply_keys(
    state: &mut UITextEditState,
    font_system: &mut FontSystem,
    input: &Input,
    clipboard: &mut WindowClipboard,
    pastes: &[String],
) {
    let held = |key| input.is_held(PhysicalKey::Code(key));
    let pressed = |key| input.is_just_pressed(PhysicalKey::Code(key));
    let command = held(KeyCode::ControlLeft)
        || held(KeyCode::ControlRight)
        || held(KeyCode::SuperLeft)
        || held(KeyCode::SuperRight);
    let shift = shift_held(input);

    if command && pressed(KeyCode::KeyA) {
        state.select_all();
    }
    if command
        && pressed(KeyCode::KeyC)
        && let Some(text) = state.copy()
    {
        clipboard.write(text);
    }
    if command
        && pressed(KeyCode::KeyX)
        && let Some(text) = state.copy()
    {
        clipboard.write(text);
        state.edit(false, |editor| {
            editor.delete_selection();
        });
    }
    if command && pressed(KeyCode::KeyV) {
        clipboard.request_paste();
    }
    if command && pressed(KeyCode::KeyZ) {
        if shift {
            state.redo();
        } else {
            state.undo();
        }
    }
    if command && pressed(KeyCode::KeyY) {
        state.redo();
    }
    for paste in pastes {
        state.edit(false, |editor| editor.insert_string(paste, None));
    }
    for &character in input.typed_chars().iter().filter(|_| !command) {
        state.edit(true, |editor| {
            editor.action(font_system, Action::Insert(character))
        });
    }

    if pressed(KeyCode::Backspace) {
        if command {
            state.delete_word(font_system, Motion::PreviousWord);
        } else {
            state.edit(false, |editor| {
                editor.action(font_system, Action::Backspace)
            });
        }
    }
    if pressed(KeyCode::Delete) {
        if command {
            state.delete_word(font_system, Motion::NextWord);
        } else {
            state.edit(false, |editor| editor.action(font_system, Action::Delete));
        }
    }

    let moves = [
        (KeyCode::ArrowLeft, Motion::Left, Motion::LeftWord),
        (KeyCode::ArrowRight, Motion::Right, Motion::RightWord),
        (KeyCode::Home, Motion::Home, Motion::Home),
        (KeyCode::End, Motion::End, Motion::End),
    ];
    for (key, motion, word) in moves {
        if pressed(key) {
            state.motion(font_system, if command { word } else { motion }, shift);
        }
    }
}

/// Edits the focused [`UITextInput`] through its cosmic-text editor and writes the result back to it.
pub(crate) fn update_text_inputs(
    mut focused: ResMut<FocusedWidget>,
    (input, window, pointer): (Res<Input>, Res<Window>, Res<UIInputState>),
    mut clipboard: ResMut<WindowClipboard>,
    mut measure: ResMut<UITextMeasure>,
    fields: Query<(
        Entity,
        &mut UITextInput,
        Option<&mut UITextEditState>,
        Option<&UITextInputParts>,
    )>,
    texts: Query<&UIText>,
    layouts: Query<&UILayout>,
    mut cmd: CommandQueue,
    (mut changed, mut submitted, mut cancelled): (
        EventWriter<UITextInputChanged>,
        EventWriter<UITextInputSubmitted>,
        EventWriter<UITextInputCancelled>,
    ),
    mut pasted: EventReader<ClipboardPasted>,
) {
    let pastes: Vec<String> = pasted
        .read()
        .filter(|paste| !paste.0.is_empty())
        .map(|paste| single_line(&paste.0))
        .collect();
    let now = Instant::now();
    let keystroke = input.just_pressed_keys().next().is_some()
        || !input.typed_chars().is_empty()
        || !pastes.is_empty();
    for (entity, mut text_input, mut existing, parts) in fields.iter() {
        if **focused != Some(entity) {
            if let Some(state) = existing.as_mut()
                && state.focused
            {
                state.focused = false;
                state.forget_history();
                state.press_time = None;
                state.pointer = None;
            }
            continue;
        }

        let font_system = measure.font_system();
        let label = parts.map_or(entity, |parts| parts.label);
        let mut created = None;
        let state: &mut UITextEditState = match existing.as_mut() {
            Some(state) => state,
            None => {
                let default_text = UIText::default();
                let text = texts.get_entity(label);
                created.insert(UITextEditState::new(
                    font_system,
                    text.unwrap_or(&default_text),
                    &text_input,
                ))
            }
        };

        state.sync(font_system, &text_input);
        let focusing = !state.focused;
        if focusing {
            state.focused = true;
            state.blink_start = now;
            state.select_all();
        }
        if let Some(layout) = layouts.get_entity(label) {
            let at = window.logical_pointer_position(&input) - layout.content_rect.min;
            apply_pointer(
                state,
                font_system,
                &input,
                &pointer,
                entity,
                at,
                focusing,
                now,
            );
        }
        apply_keys(state, font_system, &input, &mut clipboard, &pastes);
        if keystroke {
            state.blink_start = now;
        }
        state.editor.shape_as_needed(font_system, false);

        let value = state.value();
        let (cursor, selection_anchor) = state.caret();
        if text_input.value != value {
            text_input.value = value.clone();
            changed.write(UITextInputChanged {
                entity,
                value: value.clone(),
            });
        }
        if text_input.cursor != cursor {
            text_input.cursor = cursor;
        }
        if text_input.selection_anchor != selection_anchor {
            text_input.selection_anchor = selection_anchor;
        }
        state.written = Written {
            value,
            cursor,
            selection_anchor,
        };
        if let Some(state) = created {
            cmd.insert(state, entity);
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
                **focused = None;
            }
            None => {}
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

fn quad_color(color: Color, shown: bool) -> LinearRgba {
    let mut color = color.to_linear();
    if !shown {
        color.a = 0.0;
    }
    color
}

/// Moves each new text field's text onto a label child between a selection quad and a caret quad, both hidden until it has focus.
pub(crate) fn setup_text_inputs(
    fields: Query<(Entity, &UITextInput, &UIText), (Added<UITextInput>, With<UINode>)>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    for (entity, input, text) in fields.iter() {
        let line_height = UIValue::Px(text.line_height);
        let quad = |width: f32, color: Color| {
            let mut material = UIMaterial::flat(color);
            material.color = quad_color(color, false);
            (
                UINode::default()
                    .with_position(Position::Absolute)
                    .with_size(UIValue::Px(width), line_height),
                material,
            )
        };
        let label_text = UIText {
            text: shown_text(input).to_owned(),
            ..text.clone()
        };
        let mut field = cmd.entity(entity);
        let selection = field.spawn_child(quad(0.0, theme.text_selection)).entity();
        let label = field
            .spawn_child((
                UINode::default().with_min_width(UIValue::Px(0.0)),
                label_text,
            ))
            .entity();
        let caret = field.spawn_child(quad(1.0, theme.caret)).entity();
        field.insert(UITextInputParts {
            selection,
            label,
            caret,
        });
        cmd.remove::<UIText>(entity);
    }
}

fn project_quad(
    quads: &Query<(&mut UINode, &mut UIMaterial)>,
    quad: Entity,
    place: Option<(Vec2, f32)>,
    color: Color,
    shown: bool,
) {
    let Some((mut node, mut material)) = quads.get_entity(quad) else {
        return;
    };
    let color = quad_color(color, shown);
    if material.color != color {
        material.color = color;
    }
    let Some((at, width)) = place else {
        return;
    };
    let (left, top, width) = (UIValue::Px(at.x), UIValue::Px(at.y), UIValue::Px(width));
    if node.inset.left != left || node.inset.top != top || node.width != width {
        node.inset.left = left;
        node.inset.top = top;
        node.width = width;
    }
}

/// Shows each text field's value on its label, or its placeholder while the value is empty, and places its caret and selection quads over the label while it has focus, blinking the caret.
pub(crate) fn project_text_inputs(
    fields: Query<(
        &UITextInput,
        &UITextInputParts,
        Option<&UITextEditState>,
        Option<&UILayout>,
    )>,
    labels: Query<(&mut UIText, Option<&UILayout>)>,
    quads: Query<(&mut UINode, &mut UIMaterial)>,
    theme: Res<UITheme>,
) {
    let now = Instant::now();
    for (input, parts, state, layout) in fields.iter() {
        let Some((mut text, label)) = labels.get_entity(parts.label) else {
            continue;
        };
        let shown = shown_text(input);
        if text.text != shown {
            text.text = shown.to_owned();
        }

        let offset = layout
            .zip(label)
            .map(|(field, label)| label.content_rect.min - field.rect.min);
        let focused = state.filter(|state| state.focused).zip(offset);
        let (caret, selection, blink) = match focused {
            Some((state, offset)) => {
                let (caret_x, highlight) = state.marks();
                (
                    Some((offset + Vec2::new(caret_x, 0.0), 1.0)),
                    highlight.map(|(x, width)| (offset + Vec2::new(x, 0.0), width)),
                    caret_shown(state.blink_start, now),
                )
            }
            None => (None, None, false),
        };
        project_quad(
            &quads,
            parts.selection,
            selection,
            theme.text_selection,
            selection.is_some(),
        );
        project_quad(&quads, parts.caret, caret, theme.caret, blink);
    }
}

#[cfg(test)]
mod tests {
    use concerto_ecs::{
        IntoSetConfig, IntoSystem, IntoSystemConfig, Schedule, System, World,
        entity::hierarchy::{ChildOf, Children},
        events::{event_channel::EventChannel, event_cursor::EventCursor},
        system::{executor::single_thread::SingleThreadedExecutor, schedule::CompiledSchedule},
    };
    use concerto_window::{
        clipboard::answer_paste_requests,
        input::{MouseButton, actions::ActionFired},
        plugin::Window,
    };
    use glam::Vec2;

    use super::*;
    use crate::{
        anchor::UIPanelStack,
        elements::prelude::*,
        focus::{UIFocusGained, UIFocusLost, update_focus},
        interaction::{HoveredNode, Interactable, UIInputState, update_ui_interaction},
        node::{
            AlignItems, FlexDirection, UIBox, UILayout, UILayoutDiagnostics, UILayoutEngine,
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
            world.insert_resource(UITheme::nocturne());
            world.insert_resource(Input::new());
            world.insert_resource(FocusedWidget::default());
            world.insert_resource(UIInputState::default());
            world.insert_resource(WindowClipboard::default());
            world.insert_resource(EventChannel::<UITextInputChanged>::default());
            world.insert_resource(EventChannel::<UITextInputSubmitted>::default());
            world.insert_resource(EventChannel::<UITextInputCancelled>::default());
            world.insert_resource(EventChannel::<ClipboardPasted>::default());
            install_layout_listeners(&mut world);
            let mut schedule = Schedule::new();
            schedule.configure_sets(
                (UiSet::Widgets, UiSet::Setup, UiSet::Project, UiSet::Layout).chain(),
            );
            schedule.add_system(update_text_inputs.in_set(UiSet::Widgets));
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
            self.world.get_resource_mut::<Input>().unwrap().update();
        }

        fn frames(&mut self, count: usize) {
            for _ in 0..count {
                self.frame();
            }
        }

        fn focus(&mut self, field: Option<Entity>) {
            **self.world.get_resource_mut::<FocusedWidget>().unwrap() = field;
        }

        fn key(&mut self, key: KeyCode) {
            let input = self.world.get_resource_mut::<Input>().unwrap();
            input.update_key_input(PhysicalKey::Code(key), true);
            self.frame();
            let input = self.world.get_resource_mut::<Input>().unwrap();
            input.update_key_input(PhysicalKey::Code(key), false);
        }

        fn parts(&self, field: Entity) -> (Entity, Entity, Entity) {
            let children = self.children(field);
            (children[0], children[1], children[2])
        }

        fn alpha(&self, quad: Entity) -> f32 {
            self.get::<UIMaterial>(quad).color.a
        }

        fn show_the_caret_for(&mut self, field: Entity, since: Duration) {
            self.world
                .get_component_for_entity_mut::<UITextEditState>(field)
                .unwrap()
                .blink_start = Instant::now() - since;
        }

        fn layout_passes(&self) -> u64 {
            self.world
                .get_resource::<UILayoutDiagnostics>()
                .unwrap()
                .layout_passes
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

    #[test]
    fn the_caret_and_selection_sit_on_the_glyphs_they_mark() {
        let t = UITheme::nocturne();
        let mut fields = Fields::new();
        fields.spawn(t.text_field("above"));
        let field = fields.spawn(
            t.text_field("")
                .width(UIValue::Px(300.0))
                .value("hello world"),
        );
        fields.frame();
        fields.focus(Some(field));
        fields.frames(2);
        {
            let input = fields
                .world
                .get_component_for_entity_mut::<UITextInput>(field)
                .unwrap();
            input.cursor = 7;
            input.selection_anchor = Some(2);
        }
        fields.frames(3);

        let (selection, label, caret) = fields.parts(field);
        let offset =
            fields.get::<UILayout>(label).content_rect.min - fields.get::<UILayout>(field).rect.min;
        assert!(offset.x > 0.0 && offset.y > 0.0, "{offset:?}");
        let (highlight, glyph_x) =
            fields
                .get::<UITextEditState>(field)
                .editor
                .with_buffer(|buffer| {
                    let run = buffer.layout_runs().next().unwrap();
                    let highlight = run.highlight(Cursor::new(0, 2), Cursor::new(0, 7)).unwrap();
                    let glyph = run.glyphs.iter().find(|glyph| glyph.start == 7).unwrap();
                    (highlight, glyph.x)
                });

        let node = fields.get::<UINode>(selection);
        assert_eq!(node.inset.left, UIValue::Px(offset.x + highlight.0));
        assert_eq!(node.inset.top, UIValue::Px(offset.y));
        assert_eq!(node.width, UIValue::Px(highlight.1));
        let node = fields.get::<UINode>(caret);
        assert_eq!(node.inset.left, UIValue::Px(offset.x + glyph_x));
        assert_eq!(node.inset.top, UIValue::Px(offset.y));

        let text_min = fields.get::<UILayout>(label).content_rect.min;
        let drawn = fields.get::<UILayout>(caret).rect.min;
        assert_eq!(
            drawn,
            Vec2::new((text_min.x + glyph_x).round(), text_min.y),
            "Taffy rounds the insets to whole logical pixels"
        );
        let drawn = fields.get::<UILayout>(selection).rect;
        assert_eq!(drawn.min.x, (text_min.x + highlight.0).round());
        assert!((drawn.size.x - highlight.1).abs() <= 1.0, "{drawn:?}");

        assert_eq!(
            fields.get::<UIMaterial>(selection).color,
            t.text_selection.to_linear()
        );
    }

    #[test]
    fn the_caret_shows_for_530_ms_then_hides_for_530_ms() {
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        assert!(caret_shown(start, start));
        assert!(caret_shown(start, at(529)));
        assert!(!caret_shown(start, at(530)));
        assert!(!caret_shown(start, at(1059)));
        assert!(caret_shown(start, at(1060)));
        assert!(!caret_shown(start, at(1590)));
    }

    #[test]
    fn the_caret_blinks_by_alpha_and_a_keystroke_shows_it_again() {
        let t = UITheme::nocturne();
        let mut fields = Fields::new();
        let field = fields.spawn(t.text_field("").value("abc"));
        fields.frame();
        fields.focus(Some(field));
        fields.frames(2);
        let (_, _, caret) = fields.parts(field);
        fields.show_the_caret_for(field, Duration::ZERO);
        fields.frame();
        assert_eq!(fields.get::<UIMaterial>(caret).color, t.caret.to_linear());
        let visible = fields.get::<UINode>(caret).visible;

        fields.show_the_caret_for(field, Duration::from_millis(795));
        fields.frame();
        assert_eq!(fields.alpha(caret), 0.0);
        assert_eq!(fields.get::<UINode>(caret).visible, visible);

        fields.key(KeyCode::ArrowLeft);
        assert_eq!(fields.get::<UIMaterial>(caret).color, t.caret.to_linear());
    }

    #[test]
    fn an_idle_focused_field_writes_no_node_and_lays_nothing_out() {
        let t = UITheme::nocturne();
        let mut fields = Fields::new();
        let field = fields.spawn(t.text_field("").value("hello"));
        fields.frame();
        fields.focus(Some(field));
        fields.frames(4);
        let passes = fields.layout_passes();

        fields.frames(2);
        fields.show_the_caret_for(field, Duration::from_millis(795));
        fields.frame();
        fields.show_the_caret_for(field, Duration::ZERO);
        fields.frame();
        assert_eq!(fields.layout_passes(), passes);
    }

    #[test]
    fn an_unfocused_field_hides_its_caret_and_selection() {
        let t = UITheme::nocturne();
        let mut fields = Fields::new();
        let field = fields.spawn(t.text_field("").value("abc"));
        fields.frames(2);
        let (selection, _, caret) = fields.parts(field);
        assert_eq!(fields.alpha(selection), 0.0);
        assert_eq!(fields.alpha(caret), 0.0);

        fields.focus(Some(field));
        fields.frames(2);
        fields.show_the_caret_for(field, Duration::ZERO);
        fields.frame();
        assert!(fields.alpha(selection) > 0.0, "focus selects everything");
        assert!(fields.alpha(caret) > 0.0);

        fields.focus(None);
        fields.frame();
        assert_eq!(fields.alpha(selection), 0.0);
        assert_eq!(fields.alpha(caret), 0.0);
    }

    const COMMAND: KeyCode = KeyCode::ControlLeft;
    const SHIFT: KeyCode = KeyCode::ShiftLeft;
    const SCALE: f32 = 1.5;
    const LABEL: Vec2 = Vec2::new(40.0, 20.0);

    fn layout(min: Vec2, size: Vec2) -> UILayout {
        let rect = UIBox { min, size };
        UILayout {
            rect,
            content_rect: rect,
            clip_rect: rect,
            paint_order: 0,
        }
    }

    struct Editing {
        world: World,
        systems: Vec<Box<dyn System<In = ()>>>,
    }

    impl Editing {
        fn new() -> Self {
            let mut world = World::new();
            world.insert_resource(Window::new((1200, 900), SCALE as f64));
            world.insert_resource(FocusedWidget::default());
            world.insert_resource(HoveredNode::default());
            world.insert_resource(UIInputState::default());
            world.insert_resource(Input::new());
            world.insert_resource(WindowClipboard::default());
            world.insert_resource(UITextMeasure::new(build_font_system(&UIFonts::default())));
            world.insert_resource(EventChannel::<UITextInputChanged>::default());
            world.insert_resource(EventChannel::<UITextInputSubmitted>::default());
            world.insert_resource(EventChannel::<UITextInputCancelled>::default());
            world.insert_resource(EventChannel::<ClipboardPasted>::default());
            world.insert_resource(EventChannel::<ActionFired>::default());
            world.insert_resource(EventChannel::<UIFocusGained>::default());
            world.insert_resource(EventChannel::<UIFocusLost>::default());
            let mut systems: Vec<Box<dyn System<In = ()>>> = vec![
                Box::new(update_ui_interaction.into_system()),
                Box::new(answer_paste_requests.into_system()),
                Box::new(update_focus.into_system()),
                Box::new(update_text_inputs.into_system()),
            ];
            for system in &mut systems {
                system.initialize(&mut world);
            }
            Self { world, systems }
        }

        fn field(&mut self, value: &str, cursor: usize, anchor: Option<usize>) -> Entity {
            let mut input = UITextInput::new("");
            input.value = value.into();
            input.cursor = cursor;
            input.selection_anchor = anchor;
            self.world.spawn(input)
        }

        fn focused(&mut self, value: &str, cursor: usize, anchor: Option<usize>) -> Entity {
            let field = self.field(value, 0, None);
            self.focus(Some(field));
            self.frame();
            self.set_caret(field, cursor, anchor);
            field
        }

        fn input_of(&mut self, field: Entity) -> &mut UITextInput {
            self.world
                .get_component_for_entity_mut::<UITextInput>(field)
                .unwrap()
        }

        fn set_caret(&mut self, field: Entity, cursor: usize, anchor: Option<usize>) {
            let input = self.input_of(field);
            input.cursor = cursor;
            input.selection_anchor = anchor;
        }

        fn set_value(&mut self, field: Entity, value: &str) {
            let input = self.input_of(field);
            input.value = value.into();
            input.cursor = value.len();
            input.selection_anchor = None;
        }

        fn focus(&mut self, field: Option<Entity>) {
            **self.world.get_resource_mut::<FocusedWidget>().unwrap() = field;
        }

        fn focused_widget(&self) -> Option<Entity> {
            **self.world.get_resource::<FocusedWidget>().unwrap()
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
            for system in &mut self.systems {
                system.run_and_apply((), &mut self.world);
            }
            self.input().update();
        }

        fn keys(&mut self, keys: &[KeyCode]) {
            for &key in keys {
                self.input().update_key_input(PhysicalKey::Code(key), true);
            }
            self.frame();
            for &key in keys {
                self.input().update_key_input(PhysicalKey::Code(key), false);
            }
        }

        fn type_text(&mut self, text: &str) {
            for character in text.chars() {
                self.input().push_typed_char(character);
            }
            self.frame();
        }

        fn laid_out(&mut self, value: &str) -> Entity {
            let label = self.world.spawn((
                UIText {
                    text: value.into(),
                    wrap: false,
                    ..UIText::default()
                },
                layout(LABEL, Vec2::new(300.0, 18.0)),
            ));
            let selection = self.world.spawn(UIMaterial::flat(Color::TRANSPARENT));
            let caret = self.world.spawn(UIMaterial::flat(Color::TRANSPARENT));
            let mut input = UITextInput::new("");
            input.value = value.into();
            self.world.spawn((
                input,
                UITextInputParts {
                    selection,
                    label,
                    caret,
                },
                layout(LABEL - Vec2::new(6.0, 4.0), Vec2::new(312.0, 26.0)),
                Interactable,
            ))
        }

        fn x_of(&self, field: Entity, index: usize) -> f32 {
            self.world
                .get_component_for_entity::<UITextEditState>(field)
                .expect("the field has been focused")
                .editor
                .with_buffer(|buffer| {
                    let run = buffer.layout_runs().next().unwrap();
                    let glyph = run
                        .glyphs
                        .iter()
                        .find(|glyph| glyph.start == index)
                        .unwrap();
                    glyph.x + glyph.w * 0.25
                })
        }

        fn point(&mut self, x: f32) {
            let at = (LABEL + Vec2::new(x, 9.0)) * SCALE;
            self.input().update_mouse_position(at.x as f64, at.y as f64);
        }

        fn hold(&mut self, x: f32) {
            self.point(x);
            self.input().update_mouse_button(MouseButton::Left, true);
            self.frame();
        }

        fn drag(&mut self, x: f32) {
            self.point(x);
            self.frame();
        }

        fn release(&mut self) {
            self.input().update_mouse_button(MouseButton::Left, false);
            self.frame();
        }

        fn click(&mut self, x: f32) {
            self.hold(x);
            self.release();
        }

        fn shift_click(&mut self, x: f32) {
            self.input()
                .update_key_input(PhysicalKey::Code(SHIFT), true);
            self.click(x);
            self.input()
                .update_key_input(PhysicalKey::Code(SHIFT), false);
        }

        fn let_the_last_press_age(&mut self, field: Entity) {
            let state = self
                .world
                .get_component_for_entity_mut::<UITextEditState>(field)
                .unwrap();
            state.press_time = state.press_time.map(|at| at - DOUBLE_CLICK);
        }

        fn selection(&self, field: Entity) -> Option<(usize, usize)> {
            let (cursor, anchor) = self.caret(field);
            anchor.map(|anchor| (anchor.min(cursor), anchor.max(cursor)))
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

        fn caret(&self, field: Entity) -> (usize, Option<usize>) {
            let input = self
                .world
                .get_component_for_entity::<UITextInput>(field)
                .unwrap();
            (input.cursor, input.selection_anchor)
        }

        fn changes(&self) -> usize {
            self.world
                .get_resource::<EventChannel<UITextInputChanged>>()
                .unwrap()
                .len()
        }
    }

    #[test]
    fn a_paste_lands_in_the_focused_field_with_line_breaks_as_spaces() {
        let mut editing = Editing::new();
        let other = editing.field("other", 0, None);
        let field = editing.focused("ab", 1, None);
        editing.paste("x\ny\r\nz");
        editing.frame();

        assert_eq!(editing.value(field), "ax y zb");
        assert_eq!(editing.cursor(field), 6);
        assert_eq!(editing.value(other), "other");

        editing.keys(&[COMMAND, KeyCode::KeyZ]);
        assert_eq!(editing.value(field), "ab");
    }

    #[test]
    fn a_paste_replaces_the_selection() {
        let mut editing = Editing::new();
        let field = editing.focused("abcd", 3, Some(1));
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
        let field = editing.focused("abc", 3, Some(0));
        editing.paste("");
        editing.frame();

        assert_eq!(editing.value(field), "abc");
        assert_eq!(editing.caret(field), (3, Some(0)));
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
        editing.keys(&[COMMAND, KeyCode::KeyV]);
        assert_eq!(editing.value(field), "");

        editing.frame();
        assert_eq!(editing.value(field), "copied");
        editing.frame();
        assert_eq!(editing.value(field), "copied");
    }

    #[test]
    fn arrows_move_one_character_and_shift_extends_the_selection() {
        let mut editing = Editing::new();
        let field = editing.focused("aé東", 1, None);

        editing.keys(&[KeyCode::ArrowRight]);
        assert_eq!(editing.caret(field), (3, None));
        editing.keys(&[SHIFT, KeyCode::ArrowRight]);
        assert_eq!(editing.caret(field), (6, Some(3)));
        editing.keys(&[SHIFT, KeyCode::ArrowLeft]);
        assert_eq!(editing.caret(field), (3, None));
        editing.keys(&[SHIFT, KeyCode::ArrowLeft]);
        assert_eq!(editing.caret(field), (1, Some(3)));
        editing.keys(&[KeyCode::ArrowLeft]);
        assert_eq!(editing.caret(field), (0, None));
        editing.keys(&[KeyCode::ArrowLeft]);
        assert_eq!(editing.caret(field), (0, None));
        assert_eq!(editing.changes(), 0, "moving the caret changes no value");
    }

    #[test]
    fn command_arrows_move_one_word() {
        let mut editing = Editing::new();
        let field = editing.focused("hello big world", 0, None);

        editing.keys(&[COMMAND, KeyCode::ArrowRight]);
        assert_eq!(editing.caret(field), (5, None));
        editing.keys(&[COMMAND, KeyCode::ArrowRight]);
        assert_eq!(editing.caret(field), (9, None));
        editing.keys(&[COMMAND, SHIFT, KeyCode::ArrowLeft]);
        assert_eq!(editing.caret(field), (6, Some(9)));
        editing.keys(&[COMMAND, KeyCode::ArrowLeft]);
        assert_eq!(editing.caret(field), (0, None));
    }

    #[test]
    fn home_and_end_jump_to_the_ends_and_shift_extends() {
        let mut editing = Editing::new();
        let field = editing.focused("abc", 1, None);

        editing.keys(&[KeyCode::End]);
        assert_eq!(editing.caret(field), (3, None));
        editing.keys(&[SHIFT, KeyCode::Home]);
        assert_eq!(editing.caret(field), (0, Some(3)));
        editing.keys(&[KeyCode::Home]);
        assert_eq!(editing.caret(field), (0, None));
        editing.keys(&[SHIFT, KeyCode::End]);
        assert_eq!(editing.caret(field), (3, Some(0)));
    }

    #[test]
    fn backspace_and_delete_remove_the_selection_or_one_character() {
        let mut editing = Editing::new();
        let field = editing.focused("abcd", 2, None);

        editing.keys(&[KeyCode::Backspace]);
        assert_eq!(
            (editing.value(field), editing.caret(field)),
            ("acd", (1, None))
        );
        editing.keys(&[KeyCode::Delete]);
        assert_eq!(
            (editing.value(field), editing.caret(field)),
            ("ad", (1, None))
        );

        editing.set_value(field, "aé東x");
        editing.set_caret(field, 6, None);
        editing.keys(&[KeyCode::Backspace]);
        assert_eq!(
            (editing.value(field), editing.caret(field)),
            ("aéx", (3, None))
        );

        editing.set_caret(field, 1, Some(4));
        editing.keys(&[KeyCode::Delete]);
        assert_eq!(
            (editing.value(field), editing.caret(field)),
            ("a", (1, None))
        );
        editing.set_caret(field, 1, Some(0));
        editing.keys(&[KeyCode::Backspace]);
        assert_eq!(
            (editing.value(field), editing.caret(field)),
            ("", (0, None))
        );

        let changes = editing.changes();
        editing.keys(&[KeyCode::Backspace]);
        editing.keys(&[KeyCode::Delete]);
        assert_eq!(editing.changes(), changes, "nothing left to delete");
    }

    #[test]
    fn command_backspace_and_delete_remove_a_word_or_the_selection() {
        let mut editing = Editing::new();
        let field = editing.focused("hello big world", 9, None);

        editing.keys(&[COMMAND, KeyCode::Backspace]);
        assert_eq!(
            (editing.value(field), editing.caret(field)),
            ("hello  world", (6, None))
        );
        editing.keys(&[COMMAND, KeyCode::Delete]);
        assert_eq!(
            (editing.value(field), editing.caret(field)),
            ("hello ", (6, None))
        );

        editing.set_caret(field, 0, Some(2));
        editing.keys(&[COMMAND, KeyCode::Delete]);
        assert_eq!(
            (editing.value(field), editing.caret(field)),
            ("llo ", (0, None))
        );
        editing.set_caret(field, 4, Some(3));
        editing.keys(&[COMMAND, KeyCode::Backspace]);
        assert_eq!(
            (editing.value(field), editing.caret(field)),
            ("llo", (3, None))
        );
    }

    #[test]
    fn command_a_selects_everything() {
        let mut editing = Editing::new();
        let field = editing.focused("abc", 1, None);

        editing.keys(&[COMMAND, KeyCode::KeyA]);
        assert_eq!(editing.caret(field), (3, Some(0)));
    }

    #[test]
    fn copy_and_cut_put_the_selection_on_the_clipboard() {
        let mut editing = Editing::new();
        let field = editing.focused("abc", 3, Some(1));

        editing.keys(&[COMMAND, KeyCode::KeyC]);
        assert_eq!(editing.value(field), "abc");
        editing.keys(&[KeyCode::End]);
        editing.keys(&[COMMAND, KeyCode::KeyV]);
        editing.frame();
        assert_eq!(editing.value(field), "abcbc");

        editing.set_caret(field, 5, Some(3));
        editing.keys(&[COMMAND, KeyCode::KeyX]);
        assert_eq!(
            (editing.value(field), editing.caret(field)),
            ("abc", (3, None))
        );
        editing.keys(&[KeyCode::Home]);
        editing.keys(&[COMMAND, KeyCode::KeyV]);
        editing.frame();
        assert_eq!(editing.value(field), "bcabc");
    }

    #[test]
    fn undo_takes_back_a_typed_run_in_one_step_and_redo_restores_it() {
        let mut editing = Editing::new();
        let field = editing.focused("", 0, None);
        editing.type_text("ab");
        editing.type_text("c");
        editing.keys(&[KeyCode::Backspace]);
        assert_eq!(editing.value(field), "ab");

        editing.keys(&[COMMAND, KeyCode::KeyZ]);
        assert_eq!(
            (editing.value(field), editing.caret(field)),
            ("abc", (3, None))
        );
        editing.keys(&[COMMAND, KeyCode::KeyZ]);
        assert_eq!(
            (editing.value(field), editing.caret(field)),
            ("", (0, None))
        );
        editing.keys(&[COMMAND, KeyCode::KeyZ]);
        assert_eq!(editing.value(field), "");

        editing.keys(&[COMMAND, KeyCode::KeyY]);
        assert_eq!(
            (editing.value(field), editing.caret(field)),
            ("abc", (3, None))
        );
        editing.keys(&[COMMAND, SHIFT, KeyCode::KeyZ]);
        assert_eq!(editing.value(field), "ab");
        editing.keys(&[COMMAND, KeyCode::KeyY]);
        assert_eq!(editing.value(field), "ab");

        editing.keys(&[COMMAND, KeyCode::KeyZ]);
        editing.type_text("!");
        editing.keys(&[COMMAND, KeyCode::KeyY]);
        assert_eq!(editing.value(field), "abc!", "a new edit clears redo");
    }

    #[test]
    fn a_caret_move_ends_a_typed_run() {
        let mut editing = Editing::new();
        let field = editing.focused("", 0, None);
        editing.type_text("ab");
        editing.keys(&[KeyCode::ArrowLeft]);
        editing.type_text("c");
        assert_eq!(editing.value(field), "acb");

        editing.keys(&[COMMAND, KeyCode::KeyZ]);
        assert_eq!(editing.value(field), "ab");
        editing.keys(&[COMMAND, KeyCode::KeyZ]);
        assert_eq!(editing.value(field), "");
    }

    #[test]
    fn history_clears_on_an_outside_write_and_on_blur() {
        let mut editing = Editing::new();
        let field = editing.focused("", 0, None);
        editing.type_text("ab");
        editing.set_value(field, "xyz");
        editing.frame();
        editing.keys(&[COMMAND, KeyCode::KeyZ]);
        assert_eq!(editing.value(field), "xyz");

        editing.type_text("1");
        assert_eq!(editing.value(field), "xyz1");
        editing.focus(None);
        editing.frame();
        editing.focus(Some(field));
        editing.frame();
        editing.keys(&[COMMAND, KeyCode::KeyZ]);
        assert_eq!(editing.value(field), "xyz1");
    }

    #[test]
    fn gaining_focus_by_a_direct_write_selects_everything() {
        let mut editing = Editing::new();
        let field = editing.field("abc", 0, None);
        editing.focus(Some(field));
        editing.frame();
        assert_eq!(editing.caret(field), (3, Some(0)));

        editing.type_text("x");
        assert_eq!(editing.value(field), "x");
    }

    #[test]
    fn gaining_focus_by_a_press_selects_everything() {
        let mut editing = Editing::new();
        let field = editing.laid_out("abc");
        editing.click(5.0);

        assert_eq!(editing.focused_widget(), Some(field));
        assert_eq!(editing.caret(field), (3, Some(0)));
    }

    #[test]
    fn a_press_places_the_caret_and_a_shift_press_extends_the_selection() {
        let mut editing = Editing::new();
        let field = editing.laid_out("hello world");
        editing.click(0.0);
        editing.let_the_last_press_age(field);

        let x = editing.x_of(field, 6);
        editing.click(x);
        assert_eq!(editing.caret(field), (6, None));
        editing.let_the_last_press_age(field);

        let x = editing.x_of(field, 2);
        editing.shift_click(x);
        assert_eq!(editing.caret(field), (2, Some(6)));
        editing.let_the_last_press_age(field);

        let x = editing.x_of(field, 9);
        editing.shift_click(x);
        assert_eq!(editing.caret(field), (9, Some(6)));
    }

    #[test]
    fn a_press_restarts_the_blink() {
        let mut editing = Editing::new();
        let field = editing.laid_out("hello");
        editing.click(0.0);
        editing.let_the_last_press_age(field);
        let before = Instant::now();
        editing
            .world
            .get_component_for_entity_mut::<UITextEditState>(field)
            .unwrap()
            .blink_start = before - Duration::from_secs(2);

        editing.click(5.0);
        let state = editing
            .world
            .get_component_for_entity::<UITextEditState>(field)
            .unwrap();
        assert!(state.blink_start >= before);
    }

    #[test]
    fn a_press_ends_a_typed_run() {
        let mut editing = Editing::new();
        let field = editing.laid_out("ab");
        editing.click(0.0);
        editing.let_the_last_press_age(field);
        let x = editing.x_of(field, 1);
        editing.click(x);
        editing.type_text("c");
        editing.let_the_last_press_age(field);
        editing.click(x);
        editing.type_text("d");
        assert_eq!(editing.value(field), "adcb");

        editing.keys(&[COMMAND, KeyCode::KeyZ]);
        assert_eq!(editing.value(field), "acb");
    }

    #[test]
    fn a_wobble_under_the_drag_threshold_keeps_the_focusing_select_all() {
        let mut editing = Editing::new();
        let field = editing.laid_out("hello world");
        editing.hold(20.0);
        editing.drag(20.0 + DRAG_THRESHOLD - 1.0);
        assert_eq!(editing.caret(field), (11, Some(0)));
        editing.release();
        assert_eq!(editing.caret(field), (11, Some(0)));
    }

    #[test]
    fn a_drag_selects_from_the_press_to_the_pointer() {
        let mut editing = Editing::new();
        let field = editing.laid_out("hello world");
        editing.click(0.0);
        let (two, seven, nine) = (
            editing.x_of(field, 2),
            editing.x_of(field, 7),
            editing.x_of(field, 9),
        );
        editing.focus(None);
        editing.frame();

        editing.hold(two);
        editing.drag(seven);
        assert_eq!(
            editing.caret(field),
            (7, Some(2)),
            "a drag replaces the select-all"
        );
        editing.drag(nine);
        assert_eq!(editing.caret(field), (9, Some(2)));
        editing.release();
        assert_eq!(editing.caret(field), (9, Some(2)));
        editing.let_the_last_press_age(field);

        editing.hold(seven);
        editing.drag(two);
        assert_eq!(editing.caret(field), (2, Some(7)));
        editing.release();
    }

    #[test]
    fn a_double_press_selects_a_word_that_survives_the_next_frame() {
        let mut editing = Editing::new();
        let field = editing.laid_out("hello big world");
        editing.click(0.0);
        editing.let_the_last_press_age(field);
        let x = editing.x_of(field, 7);

        editing.click(x);
        assert_eq!(editing.caret(field), (7, None));
        editing.click(x);
        assert_eq!(editing.selection(field), Some((6, 9)));
        editing.frame();
        editing.frame();
        assert_eq!(editing.selection(field), Some((6, 9)));

        editing.type_text("small");
        assert_eq!(editing.value(field), "hello small world");
    }

    #[test]
    fn a_triple_press_selects_the_whole_line() {
        let mut editing = Editing::new();
        let field = editing.laid_out("hello big world");
        editing.click(0.0);
        editing.let_the_last_press_age(field);
        let x = editing.x_of(field, 7);

        editing.click(x);
        assert_eq!(editing.caret(field), (7, None));
        editing.click(x);
        assert_eq!(editing.selection(field), Some((6, 9)));
        editing.click(x);
        assert_eq!(editing.selection(field), Some((0, 15)));
    }

    #[test]
    fn a_press_after_blur_starts_a_new_count() {
        let mut editing = Editing::new();
        let field = editing.laid_out("hello big world");
        editing.click(0.0);
        editing.focus(None);
        editing.frame();
        editing.click(0.0);
        assert_eq!(editing.caret(field), (15, Some(0)));
        let x = editing.x_of(field, 7);
        editing.let_the_last_press_age(field);
        editing.click(x);
        assert_eq!(editing.caret(field), (7, None));
    }

    #[test]
    fn presses_inside_the_double_click_window_count_up_to_three() {
        let start = Instant::now();
        let soon = DOUBLE_CLICK / 2;
        assert_eq!(count_press(None, 0, start), 1);
        assert_eq!(count_press(Some(start), 1, start + soon), 2);
        assert_eq!(count_press(Some(start), 2, start + soon), 3);
        assert_eq!(count_press(Some(start), 3, start + soon), 3);
        assert_eq!(count_press(Some(start), 1, start + DOUBLE_CLICK), 1);
        assert_eq!(count_press(Some(start), 2, start + DOUBLE_CLICK * 2), 1);
    }

    #[test]
    fn enter_submits_and_escape_cancels_and_drops_focus() {
        let mut editing = Editing::new();
        let field = editing.focused("abc", 3, None);

        editing.keys(&[KeyCode::Enter]);
        let submitted: Vec<(Entity, String)> = editing
            .world
            .get_resource::<EventChannel<UITextInputSubmitted>>()
            .unwrap()
            .read_from(&mut EventCursor::default())
            .map(|event| (event.entity, event.value.clone()))
            .collect();
        assert_eq!(submitted, [(field, "abc".to_owned())]);
        assert_eq!(editing.focused_widget(), Some(field));

        editing.keys(&[KeyCode::Escape]);
        let cancelled = editing
            .world
            .get_resource::<EventChannel<UITextInputCancelled>>()
            .unwrap()
            .len();
        assert_eq!(cancelled, 1);
        assert_eq!(editing.focused_widget(), None);
        assert_eq!(editing.value(field), "abc");
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
