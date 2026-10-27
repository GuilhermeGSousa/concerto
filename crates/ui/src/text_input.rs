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
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    focus::FocusedWidget,
    interaction::{DOUBLE_CLICK, DRAG_THRESHOLD, UIInputState},
    material::UIMaterial,
    node::{Overflow, Position, UILayout, UINode, UITextMeasure, ui_text_attrs, ui_text_metrics},
    text::UIText,
    theme::UITheme,
    transform::UIValue,
};

const BLINK: Duration = Duration::from_millis(530);
const CARET: f32 = 1.0;

/// A single-line text input, edited through cosmic-text while focused and selected whole when it gains focus; with a `UIText` and a `UINode` it is drawn by a label inside it that holds that text, and scrolls horizontally to keep the caret visible.
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
    pub(crate) viewport: Entity,
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
    at: Vec2,
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
    scroll: f32,
    follow: bool,
    word: Option<(usize, usize)>,
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
            scroll: 0.0,
            follow: false,
            word: None,
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
        self.set_caret(cursor, anchor);
    }

    fn set_caret(&mut self, cursor: usize, anchor: Option<usize>) {
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
        let Some((start, end)) = self.selection_range() else {
            return (cursor, None);
        };
        let at_start = match self.editor.selection() {
            Selection::Line(press) => cursor < press.index,
            _ => start.index == cursor,
        };
        if at_start {
            (start.index, Some(end.index))
        } else {
            (end.index, Some(start.index))
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
        let (cursor, anchor) = self.caret();
        self.set_caret(cursor, anchor);
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
        (screen, at): (Vec2, Vec2),
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
            origin: screen,
            at,
            focusing,
            dragging: false,
        });
        if focusing {
            return;
        }
        self.follow = true;
        let (x, y) = self.hit_point(at);
        self.editor.shape_as_needed(font_system, false);
        if extend && self.word.is_some() {
            self.extend_word_to(at);
            self.collapse_empty_selection();
            return;
        }
        if count == 2 {
            self.select_word_at(at);
            self.collapse_empty_selection();
            return;
        }
        let action = match count {
            1 if extend => Action::Drag { x, y },
            1 => Action::Click { x, y },
            _ => Action::TripleClick { x, y },
        };
        self.editor.action(font_system, action);
        self.collapse_empty_selection();
    }

    fn drag(&mut self, font_system: &mut FontSystem, screen: Vec2, at: Vec2) {
        let Some(press) = self.pointer.as_mut() else {
            return;
        };
        if !press.dragging && press.origin.distance(screen) < DRAG_THRESHOLD {
            return;
        }
        let restart = press.focusing && !press.dragging;
        press.dragging = true;
        let pressed_at = press.at;
        self.follow = true;
        self.editor.shape_as_needed(font_system, false);
        if restart {
            let (x, y) = self.hit_point(pressed_at);
            self.editor.action(font_system, Action::Click { x, y });
        }
        if self.word.is_some() {
            self.extend_word_to(at);
        } else {
            let (x, y) = self.hit_point(at);
            self.editor.action(font_system, Action::Drag { x, y });
        }
        self.collapse_empty_selection();
    }

    fn extend_word_to(&mut self, at: Vec2) {
        let (Some((start, end)), Some(index)) = (self.word, self.glyph_at(at)) else {
            return;
        };
        let (pointer_start, pointer_end) = word_at(&self.line_text(), index);
        let (anchor, cursor) = if pointer_start < start {
            (end, pointer_start)
        } else {
            (start, pointer_end)
        };
        self.editor.set_cursor(Cursor::new(0, cursor));
        self.editor
            .set_selection(Selection::Normal(Cursor::new(0, anchor)));
    }

    fn select_word_at(&mut self, at: Vec2) {
        let Some(index) = self.glyph_at(at) else {
            return;
        };
        let (start, end) = word_at(&self.line_text(), index);
        self.word = Some((start, end));
        self.editor.set_cursor(Cursor::new(0, end));
        self.editor
            .set_selection(Selection::Normal(Cursor::new(0, start)));
    }

    fn glyph_at(&self, at: Vec2) -> Option<usize> {
        self.editor.with_buffer(|buffer| {
            let run = buffer.layout_runs().next()?;
            run.glyphs
                .iter()
                .find(|glyph| glyph.x <= at.x && at.x <= glyph.x + glyph.w)
                .map(|glyph| glyph.start)
        })
    }

    fn marks(&self) -> (f32, f32, Option<(f32, f32)>) {
        let (cursor, _) = self.caret();
        let selection = self.selection_range();
        self.editor.with_buffer(|buffer| {
            let Some(run) = buffer.layout_runs().next() else {
                return (0.0, 0.0, None);
            };
            let highlight = selection.and_then(|(start, end)| {
                run.highlight(
                    Cursor::new(start.line, start.index),
                    Cursor::new(end.line, end.index),
                )
            });
            (caret_x(&run, cursor), run.line_w, highlight)
        })
    }

    fn forget_history(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.typing = false;
    }
}

fn word_at(line: &str, index: usize) -> (usize, usize) {
    let mut last = (0, 0);
    for (start, segment) in line.split_word_bound_indices() {
        last = (start, start + segment.len());
        if index < last.1 {
            return last;
        }
    }
    last
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

fn followed_scroll(scroll: f32, caret_x: f32, line_w: f32, visible: f32) -> f32 {
    let mut scroll = scroll;
    if caret_x + CARET - scroll > visible {
        scroll = (caret_x + CARET - visible).ceil();
    }
    if caret_x - scroll < 0.0 {
        scroll = caret_x.floor();
    }
    scroll.clamp(0.0, (line_w + CARET - visible).ceil().max(0.0))
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
    (screen, at): (Vec2, Vec2),
    focusing: bool,
    now: Instant,
) {
    match input.get_mouse_button_state(MouseButton::Left) {
        InputState::Pressed if pointer.pressed(MouseButton::Left) == Some(entity) => {
            state.press(font_system, (screen, at), shift_held(input), focusing, now);
        }
        InputState::Down if pointer.captured(MouseButton::Left) == Some(entity) => {
            state.drag(font_system, screen, at);
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
) -> bool {
    let held = |key| input.is_held(PhysicalKey::Code(key));
    let pressed = |key| input.is_just_pressed(PhysicalKey::Code(key));
    let command = held(KeyCode::ControlLeft)
        || held(KeyCode::ControlRight)
        || held(KeyCode::SuperLeft)
        || held(KeyCode::SuperRight);
    let shift = shift_held(input);
    let mut acted = false;

    if command && pressed(KeyCode::KeyA) {
        state.select_all();
        acted = true;
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
        acted = true;
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
        acted = true;
    }
    if command && pressed(KeyCode::KeyY) {
        state.redo();
        acted = true;
    }
    for paste in pastes {
        state.edit(false, |editor| editor.insert_string(paste, None));
        acted = true;
    }
    for &character in input.typed_chars().iter().filter(|_| !command) {
        state.edit(true, |editor| {
            editor.action(font_system, Action::Insert(character))
        });
        acted = true;
    }

    if pressed(KeyCode::Backspace) {
        acted = true;
        if command {
            state.delete_word(font_system, Motion::PreviousWord);
        } else {
            state.edit(false, |editor| {
                editor.action(font_system, Action::Backspace)
            });
        }
    }
    if pressed(KeyCode::Delete) {
        acted = true;
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
            acted = true;
        }
    }
    acted
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
        let viewport = parts.map_or(entity, |parts| parts.viewport);
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
            state.follow = false;
            state.blink_start = now;
            state.select_all();
        }
        if let Some(layout) = layouts.get_entity(viewport) {
            let screen = window.logical_pointer_position(&input);
            let at = screen - layout.rect.min + Vec2::new(state.scroll, 0.0);
            apply_pointer(
                state,
                font_system,
                &input,
                &pointer,
                entity,
                (screen, at),
                focusing,
                now,
            );
        }
        if apply_keys(state, font_system, &input, &mut clipboard, &pastes) {
            state.follow = true;
        }
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

/// Moves each new text field's text onto a label in a viewport child that clips to the field's content box; the viewport holds a selection quad under the label and the field a caret quad, both hidden until it has focus.
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
            ellipsis: false,
            ..text.clone()
        };
        let mut field = cmd.entity(entity);
        let mut viewport = field.spawn_child(
            UINode::default()
                .with_flex_grow(1.0)
                .with_flex_shrink(1.0)
                .with_min_width(UIValue::Px(0.0))
                .with_overflow_x(Overflow::Hidden),
        );
        let selection = viewport
            .spawn_child(quad(0.0, theme.text_selection))
            .entity();
        let label = viewport
            .spawn_child((UINode::default().with_flex_shrink(0.0), label_text))
            .entity();
        let viewport = viewport.entity();
        let caret = field.spawn_child(quad(CARET, theme.caret)).entity();
        field.insert(UITextInputParts {
            viewport,
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

/// Shows each text field's value on its label, or its placeholder while the value is empty, and while it has focus scrolls the label in whole pixels to follow the caret once its text overflowed the viewport, placing its selection quad in the viewport and its caret quad on the field over the glyphs they mark, blinking the caret and hiding it outside the visible text; placement waits for the viewport's first layout, and blur scrolls back to the start.
pub(crate) fn project_text_inputs(
    fields: Query<(
        &UITextInput,
        &UITextInputParts,
        Option<&mut UITextEditState>,
        Option<&UILayout>,
    )>,
    labels: Query<(&mut UIText, &mut UINode)>,
    layouts: Query<&UILayout>,
    quads: Query<(&mut UINode, &mut UIMaterial)>,
    theme: Res<UITheme>,
) {
    let now = Instant::now();
    for (input, parts, mut state, layout) in fields.iter() {
        let Some((mut text, mut label_node)) = labels.get_entity(parts.label) else {
            continue;
        };
        let shown = shown_text(input);
        if text.text != shown {
            text.text = shown.to_owned();
        }

        if let Some(state) = state
            .as_mut()
            .filter(|state| !state.focused && (state.scroll != 0.0 || state.follow))
        {
            state.scroll = 0.0;
            state.follow = false;
        }
        let kept = state.as_ref().map_or(0.0, |state| state.scroll);
        let placed = layout
            .zip(layouts.get_entity(parts.viewport))
            .zip(layouts.get_entity(parts.label));
        let focused = state.filter(|state| state.focused).zip(placed);
        let (caret, selection, blink, scroll) = match focused {
            Some((mut state, ((field, viewport), label))) => {
                let (caret_x, line_w, highlight) = state.marks();
                let visible = viewport.rect.size.x;
                let overflowed = label.rect.size.x > visible;
                let scroll = if !overflowed {
                    0.0
                } else if state.follow {
                    followed_scroll(state.scroll, caret_x, line_w, visible)
                } else {
                    state.scroll
                };
                if state.scroll != scroll {
                    state.scroll = scroll;
                }
                let label_y = label.content_rect.min.y - viewport.rect.min.y;
                let viewport_offset = viewport.rect.min - field.rect.min;
                let caret_x = caret_x - scroll;
                (
                    Some((viewport_offset + Vec2::new(caret_x, label_y), CARET)),
                    highlight.map(|(x, width)| (Vec2::new(x - scroll, label_y), width)),
                    caret_shown(state.blink_start, now)
                        && (!overflowed || (-CARET..=visible + CARET).contains(&caret_x)),
                    scroll,
                )
            }
            None => (None, None, false, kept),
        };
        let left = if scroll == 0.0 {
            UIValue::Auto
        } else {
            UIValue::Px(-scroll)
        };
        if label_node.inset.left != left {
            label_node.inset.left = left;
        }
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
        input::{
            MouseButton,
            actions::{ActionFired, ActionLabel},
        },
        plugin::Window,
    };
    use glam::Vec2;

    use super::*;
    use crate::{
        anchor::UIPanelStack,
        elements::prelude::*,
        focus::{UIFocusGained, UIFocusLost, UIFocusNext, UIFocusPrevious, update_focus},
        interaction::{HoveredNode, Interactable, UIInputState, update_ui_interaction},
        node::{
            AlignItems, FlexDirection, Overflow, UIBox, UILayout, UILayoutDiagnostics,
            UILayoutEngine, UIRect, UITextMeasure, compute_ui_nodes, install_layout_listeners,
        },
        sets::UiSet,
        testing::Watch,
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
            world.insert_resource(HoveredNode::default());
            world.insert_resource(EventChannel::<ActionFired>::default());
            world.insert_resource(EventChannel::<UIFocusGained>::default());
            world.insert_resource(EventChannel::<UIFocusLost>::default());
            install_layout_listeners(&mut world);
            let mut schedule = Schedule::new();
            schedule.configure_sets(
                (
                    UiSet::Input,
                    UiSet::Widgets,
                    UiSet::Setup,
                    UiSet::Project,
                    UiSet::Layout,
                )
                    .chain(),
            );
            schedule.add_system(
                (
                    update_ui_interaction,
                    update_focus.after(update_ui_interaction),
                )
                    .in_set(UiSet::Input),
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

        fn keys(&mut self, keys: &[KeyCode]) {
            let input = self.world.get_resource_mut::<Input>().unwrap();
            for &key in keys {
                input.update_key_input(PhysicalKey::Code(key), true);
            }
            self.frame();
            let input = self.world.get_resource_mut::<Input>().unwrap();
            for &key in keys {
                input.update_key_input(PhysicalKey::Code(key), false);
            }
        }

        fn tab(&mut self, previous: bool) {
            let action = if previous {
                UIFocusPrevious.intern()
            } else {
                UIFocusNext.intern()
            };
            self.world
                .get_resource_mut::<EventChannel<ActionFired>>()
                .unwrap()
                .push_event(ActionFired { action });
            if previous {
                self.keys(&[KeyCode::ShiftLeft, KeyCode::Tab]);
            } else {
                self.keys(&[KeyCode::Tab]);
            }
        }

        fn press(&mut self, field: Entity) {
            let at = self.get::<UILayout>(field).rect.min + Vec2::new(12.0, 10.0);
            let input = self.world.get_resource_mut::<Input>().unwrap();
            input.update_mouse_position(at.x as f64, at.y as f64);
            input.update_mouse_button(MouseButton::Left, true);
            self.frame();
            let input = self.world.get_resource_mut::<Input>().unwrap();
            input.update_mouse_button(MouseButton::Left, false);
            self.frame();
        }

        fn viewport(&self, field: Entity) -> Entity {
            self.children(field)[0]
        }

        fn scroll(&self, field: Entity) -> f32 {
            match self.get::<UINode>(self.label(field)).inset.left {
                UIValue::Auto => 0.0,
                UIValue::Px(left) => -left,
                other => panic!("the label is inset by {other:?}"),
            }
        }

        fn visible(&self, field: Entity) -> f32 {
            self.get::<UILayout>(self.viewport(field)).rect.size.x
        }

        fn line_width(&self, field: Entity) -> f32 {
            self.get::<UITextEditState>(field)
                .editor
                .with_buffer(|buffer| buffer.layout_runs().next().map_or(0.0, |run| run.line_w))
        }

        fn scroll_to_the_end(&self, field: Entity) -> f32 {
            (self.line_width(field) + CARET - self.visible(field)).ceil()
        }

        fn caret_is_inside(&self, field: Entity) -> bool {
            let (_, _, caret) = self.parts(field);
            let caret = self.get::<UILayout>(caret).rect;
            let field = self.get::<UILayout>(field).rect;
            caret.min.x >= field.min.x && caret.max().x <= field.max().x
        }

        fn long(&mut self, value: &str) -> Entity {
            let t = UITheme::nocturne();
            let field = self.spawn(t.text_field("").width(UIValue::Px(80.0)).value(value));
            self.frame();
            field
        }

        fn parts(&self, field: Entity) -> (Entity, Entity, Entity) {
            let viewport = self.children(self.viewport(field));
            (viewport[0], viewport[1], self.children(field)[1])
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
            self.children(self.viewport(field))[1]
        }

        fn type_text(&mut self, text: &str) {
            let input = self.world.get_resource_mut::<Input>().unwrap();
            for character in text.chars() {
                input.push_typed_char(character);
            }
            self.frame();
        }

        fn set_value(&mut self, field: Entity, value: &str) {
            self.world
                .get_component_for_entity_mut::<UITextInput>(field)
                .unwrap()
                .value = value.into();
        }

        fn point(&mut self, at: Vec2) {
            let input = self.world.get_resource_mut::<Input>().unwrap();
            input.update_mouse_position(at.x as f64, at.y as f64);
        }

        fn hold(&mut self, at: Vec2) {
            self.point(at);
            let input = self.world.get_resource_mut::<Input>().unwrap();
            input.update_mouse_button(MouseButton::Left, true);
            self.frame();
        }

        fn drag(&mut self, at: Vec2) {
            self.point(at);
            self.frame();
        }

        fn release(&mut self) {
            let input = self.world.get_resource_mut::<Input>().unwrap();
            input.update_mouse_button(MouseButton::Left, false);
            self.frame();
        }

        fn caret(&self, field: Entity) -> (usize, Option<usize>) {
            let input = self.get::<UITextInput>(field);
            (input.cursor, input.selection_anchor)
        }

        fn glyph(&self, field: Entity, index: usize) -> (f32, f32) {
            self.get::<UITextEditState>(field)
                .editor
                .with_buffer(|buffer| {
                    let run = buffer.layout_runs().next().unwrap();
                    let glyph = run
                        .glyphs
                        .iter()
                        .find(|glyph| glyph.start == index)
                        .unwrap();
                    (glyph.x, glyph.w)
                })
        }

        fn widest_glyph(&self, field: Entity) -> f32 {
            self.get::<UITextEditState>(field)
                .editor
                .with_buffer(|buffer| {
                    let run = buffer.layout_runs().next().unwrap();
                    run.glyphs.iter().map(|glyph| glyph.w).fold(0.0, f32::max)
                })
        }

        fn over_glyph(&self, field: Entity, index: usize) -> Vec2 {
            let (x, w) = self.glyph(field, index);
            let viewport = self.get::<UILayout>(self.viewport(field)).rect;
            viewport.min + Vec2::new(x + w * 0.25 - self.scroll(field), viewport.size.y / 2.0)
        }

        fn beside(&self, field: Entity, right: bool) -> Vec2 {
            let rect = self.get::<UILayout>(field).rect;
            let past = self.widest_glyph(field);
            let x = if right {
                rect.max().x + past
            } else {
                rect.min.x - past
            };
            Vec2::new(x, rect.min.y + rect.size.y / 2.0)
        }
    }

    #[test]
    fn a_text_field_gets_a_viewport_holding_a_selection_and_a_label_and_a_caret() {
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
        assert_eq!(children.len(), 2);
        let (viewport, caret) = (children[0], children[1]);
        let inside = fields.children(viewport);
        assert_eq!(inside.len(), 2);
        let (selection, label) = (inside[0], inside[1]);
        assert_eq!(
            fields.get::<UITextInputParts>(field).viewport,
            viewport,
            "the parts name the viewport"
        );

        let node = fields.get::<UINode>(viewport);
        assert_eq!(node.position, Position::Relative);
        assert_eq!(node.flex_grow, 1.0);
        assert_eq!(node.flex_shrink, 1.0);
        assert_eq!(node.min_width, UIValue::Px(0.0));
        assert_eq!(node.overflow_x, Overflow::Hidden);
        assert_eq!(node.padding, UIRect::default());
        assert!(
            fields
                .world
                .get_component_for_entity::<UIText>(viewport)
                .is_none()
        );

        let text = fields.get::<UIText>(label);
        assert_eq!(text.text, "Alice");
        assert_eq!(text.font_size, t.font_size_sm);
        assert_eq!(text.color, t.text);
        assert!(!text.wrap);
        assert!(!text.ellipsis);
        assert_eq!(fields.get::<UINode>(label).min_width, UIValue::Auto);
        assert_eq!(fields.get::<UINode>(label).flex_shrink, 0.0);
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
    fn a_long_value_and_its_selection_are_clipped_to_the_content_box() {
        let t = UITheme::nocturne();
        let mut fields = Fields::new();
        let field = fields.spawn(
            t.text_field("")
                .width(UIValue::Px(80.0))
                .value("a value much wider than eighty logical pixels"),
        );
        fields.frame();
        fields.focus(Some(field));
        fields.frames(3);

        let field_layout = fields.get::<UILayout>(field);
        let content = field_layout.content_rect;
        assert!(
            content.size.x < field_layout.rect.size.x,
            "the field has padding"
        );
        let (selection, label, _) = fields.parts(field);
        let label = fields.get::<UILayout>(label);
        let selection = fields.get::<UILayout>(selection);
        assert!(
            label.content_rect.size.x > content.size.x,
            "the label keeps the width of its text"
        );
        assert!(
            selection.rect.size.x > content.size.x,
            "focus selects the whole value"
        );
        for clip in [
            label.clip_rect.intersection(label.content_rect),
            selection.clip_rect.intersection(selection.rect),
        ] {
            assert!(clip.has_area(), "{clip:?}");
            assert!(
                clip.min.x >= content.min.x && clip.min.y >= content.min.y,
                "{clip:?} in {content:?}"
            );
            assert!(
                clip.max().x <= content.max().x && clip.max().y <= content.max().y,
                "{clip:?} in {content:?}"
            );
        }
    }

    #[test]
    fn a_content_sized_field_follows_its_text_while_typing_at_the_end() {
        let t = UITheme::nocturne();
        let mut fields = Fields::new();
        let field = fields.spawn(t.text_field("").value("abc"));
        fields.frame();
        fields.focus(Some(field));
        fields.frames(2);
        fields.key(KeyCode::End);
        assert_eq!(fields.scroll(field), 0.0);
        fields.frame();

        let mut last = fields.get::<UILayout>(field).rect.size.x;
        for character in ["d", "e", "W", "W"] {
            fields.type_text(character);
            assert_eq!(
                fields.get::<UINode>(fields.label(field)).inset.left,
                UIValue::Auto,
                "the field grows instead of scrolling"
            );
            let field_layout = fields.get::<UILayout>(field);
            let padding = field_layout.rect.size.x - field_layout.content_rect.size.x;
            let viewport = fields.get::<UILayout>(fields.viewport(field)).rect;
            let label = fields.get::<UILayout>(fields.label(field)).rect;
            assert!(field_layout.rect.size.x > last, "the field grows");
            assert_eq!(viewport.size.x, label.size.x);
            assert_eq!(field_layout.rect.size.x, label.size.x + padding);
            last = field_layout.rect.size.x;
        }
        assert_eq!(fields.get::<UITextInput>(field).value, "abcdeWW");
    }

    #[test]
    fn a_single_line_field_draws_no_ellipsis() {
        let t = UITheme::nocturne();
        let mut fields = Fields::new();
        let field = fields.spawn(
            t.text_field("")
                .single_line()
                .width(UIValue::Px(80.0))
                .value("a value much wider than eighty logical pixels"),
        );
        fields.frames(2);

        assert!(!fields.get::<UIText>(fields.label(field)).ellipsis);
    }

    const LONG: &str = "a value much wider than eighty logical pixels";

    #[test]
    fn typing_past_the_edge_scrolls_so_the_caret_stays_in_the_field() {
        let mut fields = Fields::new();
        let field = fields.long("abc");
        fields.focus(Some(field));
        fields.frames(2);
        fields.key(KeyCode::End);

        for _ in 0..20 {
            fields.type_text("w");
            let (_, _, caret) = fields.parts(field);
            let drawn = fields.get::<UILayout>(caret);
            assert!(
                fields.alpha(caret) == 0.0
                    || !drawn.clip_rect.intersection(drawn.rect).has_area()
                    || fields.caret_is_inside(field),
                "a drawn caret is inside the field"
            );
        }
        fields.frame();

        let scroll = fields.scroll(field);
        assert!(scroll > 0.0, "the text scrolled");
        assert_eq!(scroll, scroll.round(), "by whole pixels");
        assert_eq!(scroll, fields.scroll_to_the_end(field));
        let label = fields.label(field);
        assert_eq!(fields.get::<UINode>(label).inset.left, UIValue::Px(-scroll));
        let viewport = fields.get::<UILayout>(fields.viewport(field)).rect;
        assert_eq!(
            fields.get::<UILayout>(label).rect.min.x,
            viewport.min.x - scroll
        );
        fields.show_the_caret_for(field, Duration::ZERO);
        fields.frame();
        let (_, _, caret) = fields.parts(field);
        assert!(fields.alpha(caret) > 0.0);
        assert!(fields.caret_is_inside(field));
    }

    #[test]
    fn home_and_end_scroll_to_either_end_of_the_text() {
        let mut fields = Fields::new();
        let field = fields.long(LONG);
        fields.focus(Some(field));
        fields.frames(2);

        fields.key(KeyCode::End);
        let end = fields.scroll_to_the_end(field);
        assert!(end > 0.0);
        assert_eq!(fields.scroll(field), end);
        fields.key(KeyCode::Home);
        assert_eq!(
            fields.get::<UINode>(fields.label(field)).inset.left,
            UIValue::Auto
        );
        fields.key(KeyCode::End);
        assert_eq!(fields.scroll(field), end);
    }

    #[test]
    fn deleting_from_the_end_of_a_scrolled_value_pulls_the_scroll_back() {
        let mut fields = Fields::new();
        let field = fields.long(LONG);
        fields.focus(Some(field));
        fields.frames(2);
        fields.key(KeyCode::End);
        let before = fields.scroll(field);

        fields.key(KeyCode::Backspace);
        let after = fields.scroll(field);
        assert!(after < before, "{after} < {before}");
        assert_eq!(after, fields.scroll_to_the_end(field));

        fields.keys(&[KeyCode::ControlLeft, KeyCode::KeyA]);
        fields.key(KeyCode::Backspace);
        assert_eq!(fields.get::<UITextInput>(field).value, "");
        assert_eq!(
            fields.get::<UINode>(fields.label(field)).inset.left,
            UIValue::Auto
        );
    }

    #[test]
    fn blur_shows_the_start_of_the_value_again() {
        let mut fields = Fields::new();
        let field = fields.long(LONG);
        fields.focus(Some(field));
        fields.frames(2);
        fields.key(KeyCode::End);
        assert!(fields.scroll(field) > 0.0);

        fields.focus(None);
        fields.frame();
        let label = fields.label(field);
        assert_eq!(fields.get::<UINode>(label).inset.left, UIValue::Auto);
        fields.frame();
        assert_eq!(
            fields.get::<UILayout>(label).rect.min.x,
            fields.get::<UILayout>(fields.viewport(field)).rect.min.x
        );

        fields.focus(Some(field));
        fields.frames(2);
        assert_eq!(fields.scroll(field), 0.0, "refocusing keeps the start");
    }

    #[test]
    fn focusing_a_long_field_keeps_its_start_in_view() {
        for how in ["write", "press", "tab", "shift tab"] {
            let mut fields = Fields::new();
            let field = fields.long(LONG);
            fields.frame();
            match how {
                "write" => {
                    fields.focus(Some(field));
                    fields.frame();
                }
                "press" => fields.press(field),
                "tab" => fields.tab(false),
                _ => fields.tab(true),
            }
            assert_eq!(
                **fields.world.get_resource::<FocusedWidget>().unwrap(),
                Some(field),
                "{how}"
            );
            fields.frames(2);
            let input = fields.get::<UITextInput>(field);
            assert_eq!(
                (input.cursor, input.selection_anchor),
                (LONG.len(), Some(0)),
                "{how} selects everything"
            );
            assert_eq!(
                fields.get::<UINode>(fields.label(field)).inset.left,
                UIValue::Auto,
                "{how} keeps the start in view"
            );
        }
    }

    #[test]
    fn a_value_narrower_than_the_field_never_scrolls() {
        let t = UITheme::nocturne();
        let mut fields = Fields::new();
        let field = fields.spawn(t.text_field("").width(UIValue::Px(300.0)).value("hello"));
        fields.frame();
        fields.focus(Some(field));
        fields.frames(2);
        let label = fields.label(field);

        fields.key(KeyCode::End);
        assert_eq!(fields.get::<UINode>(label).inset.left, UIValue::Auto);
        fields.type_text("abc");
        assert_eq!(fields.get::<UINode>(label).inset.left, UIValue::Auto);
        fields.key(KeyCode::Home);
        assert_eq!(fields.get::<UINode>(label).inset.left, UIValue::Auto);
        fields.key(KeyCode::End);
        assert_eq!(fields.get::<UINode>(label).inset.left, UIValue::Auto);
    }

    #[test]
    fn the_caret_hides_while_it_lies_outside_the_visible_text() {
        let mut fields = Fields::new();
        let field = fields.long(LONG);
        fields.focus(Some(field));
        fields.frames(2);
        fields.show_the_caret_for(field, Duration::ZERO);
        fields.frame();
        let (selection, _, caret) = fields.parts(field);
        assert!(fields.alpha(selection) > 0.0, "focus selects everything");
        assert_eq!(fields.alpha(caret), 0.0, "the caret is past the right edge");

        fields.key(KeyCode::End);
        assert!(fields.alpha(caret) > 0.0, "End brings it into view");
        assert!(fields.caret_is_inside(field));
    }

    #[test]
    fn the_caret_stays_visible_while_stepping_left_across_the_left_edge() {
        for step in [vec![KeyCode::ArrowLeft], vec![COMMAND, KeyCode::ArrowLeft]] {
            let mut fields = Fields::new();
            let field = fields.long(LONG);
            fields.focus(Some(field));
            fields.frames(2);
            fields.key(KeyCode::End);
            let end = fields.scroll(field);
            let fractional = fields
                .get::<UITextEditState>(field)
                .editor
                .with_buffer(|buffer| {
                    let run = buffer.layout_runs().next().unwrap();
                    run.glyphs
                        .iter()
                        .any(|glyph| glyph.x < end && glyph.x.fract() >= 0.5)
                });
            assert!(fractional, "some glyph past the left edge has x.5 or more");

            let (_, _, caret) = fields.parts(field);
            while fields.caret(field).0 > 0 {
                fields.keys(&step);
                let at = fields.caret(field).0;
                assert!(
                    fields.alpha(caret) > 0.0,
                    "{step:?} to {at}: the caret shows"
                );
                let caret = fields.get::<UILayout>(caret).rect;
                let viewport = fields.get::<UILayout>(fields.viewport(field)).rect;
                assert!(
                    caret.min.x >= viewport.min.x && caret.max().x <= viewport.max().x,
                    "{step:?} to {at}: {caret:?} in {viewport:?}"
                );
            }
        }
    }

    #[test]
    fn a_field_without_padding_keeps_its_caret_inside_while_typing_at_the_end() {
        let t = UITheme::nocturne();
        let mut fields = Fields::new();
        let field = fields.spawn(
            t.text_field("")
                .bare()
                .height(UIValue::Auto)
                .padding(0.0)
                .single_line()
                .width(UIValue::Px(80.0))
                .value(LONG),
        );
        fields.frame();
        fields.focus(Some(field));
        fields.frames(2);
        fields.key(KeyCode::End);
        assert_eq!(
            fields.visible(field),
            fields.get::<UILayout>(field).rect.size.x,
            "the field has no padding"
        );

        let (_, _, caret) = fields.parts(field);
        for _ in 0..10 {
            fields.type_text("w");
            assert!(fields.alpha(caret) > 0.0, "the caret shows");
            assert!(
                fields.caret_is_inside(field),
                "the caret is inside the field"
            );
        }
    }

    #[test]
    fn typing_at_the_end_of_a_content_sized_field_never_hides_the_caret() {
        let t = UITheme::nocturne();
        let mut fields = Fields::new();
        let field = fields.spawn(t.text_field("").value("abc"));
        fields.frame();
        fields.focus(Some(field));
        fields.frames(2);
        fields.key(KeyCode::End);
        fields.frame();

        let (_, _, caret) = fields.parts(field);
        for character in ["d", "e", "W", "W", "m"] {
            fields.type_text(character);
            assert!(
                fields.alpha(caret) > 0.0,
                "typing {character} shows the caret"
            );
        }
    }

    #[test]
    fn an_idle_scrolled_field_writes_no_node_and_lays_nothing_out() {
        let mut fields = Fields::new();
        let field = fields.long(LONG);
        fields.focus(Some(field));
        fields.frames(2);
        fields.key(KeyCode::End);
        fields.frames(3);
        assert!(fields.scroll(field) > 0.0);
        let passes = fields.layout_passes();
        fn idle() {}
        let mut nodes = Watch::new::<UINode, _>(&mut fields.world, idle);
        nodes.run(&mut fields.world);

        fields.frames(2);
        fields.show_the_caret_for(field, Duration::from_millis(795));
        fields.frame();
        fields.show_the_caret_for(field, Duration::ZERO);
        fields.frame();
        assert_eq!(fields.layout_passes(), passes);
        assert_eq!(nodes.run(&mut fields.world), 0, "no UINode was written");
    }

    #[test]
    fn a_press_on_a_scrolled_field_places_the_caret_on_the_glyph_under_the_pointer() {
        let mut fields = Fields::new();
        let field = fields.long(LONG);
        fields.focus(Some(field));
        fields.frames(2);
        fields.key(KeyCode::End);
        let scroll = fields.scroll(field);
        assert!(scroll > 0.0);

        let index = LONG.len() - 4;
        let at = fields.over_glyph(field, index);
        fields.hold(at);
        fields.release();
        assert_eq!(fields.caret(field), (index, None));
        assert_eq!(fields.scroll(field), scroll);
    }

    #[test]
    fn a_press_and_drag_on_an_unfocused_long_field_selects_from_the_pressed_glyph() {
        let mut fields = Fields::new();
        let field = fields.long(LONG);
        fields.focus(Some(field));
        fields.frames(2);
        fields.focus(None);
        fields.frames(2);
        let (from, to) = (fields.over_glyph(field, 2), fields.over_glyph(field, 8));

        fields.hold(from);
        fields.drag(to);
        assert_eq!(fields.caret(field), (8, Some(2)));
        assert_eq!(fields.scroll(field), 0.0);
        let right = fields.beside(field, true);
        fields.drag(right);
        fields.frames(3);
        let (cursor, anchor) = fields.caret(field);
        assert_eq!(anchor, Some(2), "the selection starts at the pressed glyph");
        assert!(cursor > 8);
        assert!(fields.scroll(field) > 0.0);
        fields.release();
    }

    #[test]
    fn a_press_in_the_right_padding_stays_a_click_and_does_not_keep_scrolling() {
        let mut fields = Fields::new();
        let field = fields.long(LONG);
        fields.focus(Some(field));
        fields.frames(2);
        let rect = fields.get::<UILayout>(field).rect;
        let viewport = fields.get::<UILayout>(fields.viewport(field)).rect;
        let at = Vec2::new(rect.max().x - 1.0, viewport.min.y + viewport.size.y / 2.0);
        assert!(at.x > viewport.max().x, "the press is in the padding");

        fields.hold(at);
        let scroll = fields.scroll(field);
        assert!(
            scroll >= DRAG_THRESHOLD,
            "the press scrolls the text by {scroll}, at least the drag threshold"
        );
        let (cursor, anchor) = fields.caret(field);
        assert_eq!(anchor, None);
        for _ in 0..4 {
            fields.frame();
            assert_eq!(
                fields.caret(field),
                (cursor, None),
                "the press stays a click"
            );
            assert_eq!(
                fields.scroll(field),
                scroll,
                "the still pointer scrolls no further"
            );
        }
        fields.release();
        assert_eq!(fields.caret(field), (cursor, None));
        assert_eq!(fields.scroll(field), scroll);
    }

    #[test]
    fn holding_a_drag_past_an_edge_scrolls_further_each_frame() {
        let mut fields = Fields::new();
        let field = fields.long(LONG);
        fields.focus(Some(field));
        fields.frames(2);
        let end = fields.scroll_to_the_end(field);
        let from = fields.over_glyph(field, 2);
        fields.hold(from);

        let right = fields.beside(field, true);
        fields.drag(right);
        let mut scroll = fields.scroll(field);
        let mut frames = 1;
        while scroll < end {
            fields.frame();
            let next = fields.scroll(field);
            assert!(next > scroll, "{next} > {scroll} past the right edge");
            scroll = next;
            frames += 1;
            assert!(frames < 100);
        }
        assert!(frames > 2, "it scrolls a little on each frame");
        assert_eq!(fields.caret(field), (LONG.len(), Some(2)));

        let left = fields.beside(field, false);
        fields.drag(left);
        let mut scroll = fields.scroll(field);
        let mut frames = 1;
        while scroll > 0.0 {
            fields.frame();
            let next = fields.scroll(field);
            assert!(next < scroll, "{next} < {scroll} past the left edge");
            scroll = next;
            frames += 1;
            assert!(frames < 100);
        }
        assert!(frames > 2, "it scrolls back a little on each frame");
        assert_eq!(fields.caret(field), (0, Some(2)));
        fields.release();
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
        let in_viewport = fields.get::<UILayout>(label).content_rect.min
            - fields.get::<UILayout>(fields.viewport(field)).rect.min;
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
        assert_eq!(node.inset.left, UIValue::Px(in_viewport.x + highlight.0));
        assert_eq!(node.inset.top, UIValue::Px(in_viewport.y));
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
        fn idle() {}
        let mut nodes = Watch::new::<UINode, _>(&mut fields.world, idle);
        nodes.run(&mut fields.world);

        fields.frames(2);
        fields.show_the_caret_for(field, Duration::from_millis(795));
        fields.frame();
        fields.show_the_caret_for(field, Duration::ZERO);
        fields.frame();
        assert_eq!(fields.layout_passes(), passes);
        assert_eq!(nodes.run(&mut fields.world), 0, "no UINode was written");
    }

    #[test]
    fn gaining_focus_restarts_the_blink() {
        let mut editing = Editing::new();
        let field = editing.field("abc", 0, None);
        editing.focus(Some(field));
        editing.frame();
        editing.focus(None);
        editing.frame();
        let before = Instant::now();
        editing
            .world
            .get_component_for_entity_mut::<UITextEditState>(field)
            .unwrap()
            .blink_start = before - Duration::from_secs(2);

        editing.focus(Some(field));
        editing.frame();
        let state = editing
            .world
            .get_component_for_entity::<UITextEditState>(field)
            .unwrap();
        assert!(state.blink_start >= before);
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
            world.insert_resource(UITheme::nocturne());
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
                Box::new(project_text_inputs.into_system()),
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
                UINode::default(),
            ));
            let quad = || (UINode::default(), UIMaterial::flat(Color::TRANSPARENT));
            let viewport = self.world.spawn(layout(LABEL, Vec2::new(300.0, 18.0)));
            let selection = self.world.spawn(quad());
            let caret = self.world.spawn(quad());
            let mut input = UITextInput::new("");
            input.value = value.into();
            self.world.spawn((
                input,
                UITextInputParts {
                    viewport,
                    selection,
                    label,
                    caret,
                },
                layout(LABEL - Vec2::new(6.0, 4.0), Vec2::new(312.0, 26.0)),
                Interactable,
            ))
        }

        fn glyph(&self, field: Entity, index: usize) -> (f32, f32) {
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
                    (glyph.x, glyph.w)
                })
        }

        fn x_of(&self, field: Entity, index: usize) -> f32 {
            let (x, w) = self.glyph(field, index);
            x + w * 0.25
        }

        fn caret_inset(&self, field: Entity) -> UIValue {
            let parts = self
                .world
                .get_component_for_entity::<UITextInputParts>(field)
                .unwrap();
            self.world
                .get_component_for_entity::<UINode>(parts.caret)
                .unwrap()
                .inset
                .left
        }

        fn caret_inset_at(&self, field: Entity, index: usize) -> UIValue {
            let viewport = self
                .world
                .get_component_for_entity::<UITextInputParts>(field)
                .unwrap()
                .viewport;
            let offset = self
                .world
                .get_component_for_entity::<UILayout>(viewport)
                .unwrap()
                .rect
                .min
                - self
                    .world
                    .get_component_for_entity::<UILayout>(field)
                    .unwrap()
                    .rect
                    .min;
            UIValue::Px(offset.x + self.glyph(field, index).0)
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
    fn word_at_finds_the_segment_under_an_index() {
        assert_eq!(word_at("hello big world", 0), (0, 5));
        assert_eq!(word_at("hello big world", 4), (0, 5));
        assert_eq!(word_at("hello big world", 6), (6, 9));
        assert_eq!(word_at("hello big world", 8), (6, 9));
        assert_eq!(word_at("hello big world", 5), (5, 6));
        assert_eq!(word_at("hello, world", 5), (5, 6));
        assert_eq!(word_at("is a test", 3), (3, 4));
        assert_eq!(word_at("hello big world", 15), (10, 15));
        assert_eq!(word_at("hello big world", 40), (10, 15));
        assert_eq!(word_at("", 0), (0, 0));
    }

    #[test]
    fn glyph_at_tells_the_last_letter_of_a_word_from_the_space_after_it() {
        let mut editing = Editing::new();
        let field = editing.laid_out("hello big world");
        editing.click(0.0);
        let (o_x, o_w) = editing.glyph(field, 4);
        let (space_x, space_w) = editing.glyph(field, 5);
        let state = editing
            .world
            .get_component_for_entity::<UITextEditState>(field)
            .unwrap();

        assert_eq!(state.glyph_at(Vec2::new(o_x + o_w * 0.75, 9.0)), Some(4));
        assert_eq!(
            state.glyph_at(Vec2::new(space_x + space_w * 0.25, 9.0)),
            Some(5)
        );
        assert_eq!(state.glyph_at(Vec2::new(-50.0, 9.0)), None);
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
    fn a_double_press_at_a_word_edge_selects_only_that_word() {
        let mut editing = Editing::new();
        let field = editing.laid_out("hello big world");
        editing.click(0.0);
        let cases = [
            (6, 0.1, (6, 9)),
            (4, 0.9, (0, 5)),
            (5, 0.1, (5, 6)),
            (5, 0.9, (5, 6)),
            (0, 0.1, (0, 5)),
            (14, 0.9, (10, 15)),
        ];
        for (index, along, expected) in cases {
            let (glyph_x, glyph_w) = editing.glyph(field, index);
            let x = glyph_x + glyph_w * along;
            editing.let_the_last_press_age(field);
            editing.click(x);
            editing.click(x);
            assert_eq!(editing.selection(field), Some(expected), "{index} {along}");
            editing.let_the_last_press_age(field);
        }
    }

    #[test]
    fn a_double_press_on_a_one_letter_word_selects_it_alone() {
        let mut editing = Editing::new();
        let field = editing.laid_out("is a test");
        editing.click(0.0);
        for along in [0.1, 0.9] {
            let (glyph_x, glyph_w) = editing.glyph(field, 3);
            let x = glyph_x + glyph_w * along;
            editing.let_the_last_press_age(field);
            editing.click(x);
            editing.click(x);
            assert_eq!(editing.selection(field), Some((3, 4)), "{along}");
            assert_eq!(editing.caret(field), (4, Some(3)));
        }
    }

    #[test]
    fn a_double_press_puts_the_caret_at_the_word_end_and_shift_right_extends_from_it() {
        let mut editing = Editing::new();
        let field = editing.laid_out("hello big world");
        editing.click(0.0);
        editing.let_the_last_press_age(field);
        let x = editing.x_of(field, 7);

        editing.click(x);
        editing.click(x);
        assert_eq!(editing.caret(field), (9, Some(6)));
        assert_eq!(editing.caret_inset(field), editing.caret_inset_at(field, 9));

        editing.keys(&[SHIFT, KeyCode::ArrowRight]);
        assert_eq!(editing.caret(field), (10, Some(6)));
        assert_eq!(
            editing.caret_inset(field),
            editing.caret_inset_at(field, 10)
        );
    }

    #[test]
    fn dragging_after_a_double_press_extends_by_whole_words() {
        let mut editing = Editing::new();
        let field = editing.laid_out("hello big world");
        editing.click(0.0);
        editing.let_the_last_press_age(field);
        let x = editing.x_of(field, 7);

        editing.click(x);
        editing.hold(x);
        assert_eq!(editing.selection(field), Some((6, 9)));

        editing.drag(editing.x_of(field, 13));
        assert_eq!(editing.caret(field), (15, Some(6)));
        editing.drag(editing.x_of(field, 1));
        assert_eq!(editing.caret(field), (0, Some(9)));
        editing.drag(editing.x_of(field, 7));
        assert_eq!(editing.caret(field), (9, Some(6)));
        editing.release();
        assert_eq!(editing.caret(field), (9, Some(6)));
    }

    #[test]
    fn a_shift_press_after_a_double_press_extends_by_whole_words() {
        let mut editing = Editing::new();
        let field = editing.laid_out("hello big world");
        editing.click(0.0);
        editing.let_the_last_press_age(field);
        let x = editing.x_of(field, 7);

        editing.click(x);
        editing.click(x);
        editing.shift_click(editing.x_of(field, 13));
        assert_eq!(editing.caret(field), (15, Some(6)));

        editing.let_the_last_press_age(field);
        editing.shift_click(editing.x_of(field, 1));
        assert_eq!(editing.caret(field), (0, Some(9)));

        editing.let_the_last_press_age(field);
        editing.shift_click(editing.x_of(field, 8));
        assert_eq!(editing.caret(field), (9, Some(6)));
    }

    #[test]
    fn a_triple_press_puts_the_caret_at_the_line_end() {
        let mut editing = Editing::new();
        let field = editing.laid_out("hello big world");
        editing.click(0.0);
        editing.let_the_last_press_age(field);
        let x = editing.x_of(field, 7);

        editing.click(x);
        editing.click(x);
        editing.click(x);
        assert_eq!(editing.caret(field), (15, Some(0)));
        let UIValue::Px(last) = editing.caret_inset_at(field, 14) else {
            unreachable!()
        };
        let (_, width) = editing.glyph(field, 14);
        assert_eq!(editing.caret_inset(field), UIValue::Px(last + width));

        editing.keys(&[SHIFT, KeyCode::ArrowLeft]);
        assert_eq!(editing.caret(field), (14, Some(0)));
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
