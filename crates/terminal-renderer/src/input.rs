use std::{collections::HashSet, time::Duration};

use concerto_ecs::{
    events::event_writer::EventWriter,
    resource::{ResMut, Resource},
};
use ratatui::crossterm::event::{Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind};

use crate::resize::TerminalResizeEvent;

#[derive(Resource)]
pub struct TerminalInput {
    // Keys that received a Press or Repeat event this frame — use for movement
    keys_active: HashSet<KeyCode>,
    // Keys that received a Press event this frame (not Repeat) — use for one-shot actions
    keys_just_pressed: HashSet<KeyCode>,
    mouse_col: u16,
    mouse_row: u16,
    prev_mouse_col: u16,
    prev_mouse_row: u16,
    mouse_buttons_held: HashSet<MouseButton>,
    scroll_delta: f32,
    // Used by poll_terminal_input to detect size changes without relying on Event::Resize
    last_terminal_size: (u16, u16),
}

impl TerminalInput {
    pub fn new() -> Self {
        // Initialise last_terminal_size to the actual current size so the first
        // frame doesn't spuriously emit a resize event.
        let last_terminal_size = ratatui::crossterm::terminal::size().unwrap_or((80, 24));
        Self {
            keys_active: HashSet::new(),
            keys_just_pressed: HashSet::new(),
            mouse_col: 0,
            mouse_row: 0,
            prev_mouse_col: 0,
            prev_mouse_row: 0,
            mouse_buttons_held: HashSet::new(),
            scroll_delta: 0.0,
            last_terminal_size,
        }
    }

    fn begin_frame(&mut self) {
        self.keys_active.clear();
        self.keys_just_pressed.clear();
        self.scroll_delta = 0.0;
        self.prev_mouse_col = self.mouse_col;
        self.prev_mouse_row = self.mouse_row;
    }

    /// True if the key had a Press or Repeat event this frame. Use for held-down movement.
    pub fn is_key_active(&self, key: KeyCode) -> bool {
        self.keys_active.contains(&key)
    }

    /// True only on the frame the key was first pressed (not on auto-repeat).
    pub fn is_key_just_pressed(&self, key: KeyCode) -> bool {
        self.keys_just_pressed.contains(&key)
    }

    /// Current mouse position as (col, row) in character cells.
    pub fn mouse_position(&self) -> (u16, u16) {
        (self.mouse_col, self.mouse_row)
    }

    /// Delta from previous frame as (col_delta, row_delta).
    pub fn mouse_delta(&self) -> (i16, i16) {
        (
            self.mouse_col as i16 - self.prev_mouse_col as i16,
            self.mouse_row as i16 - self.prev_mouse_row as i16,
        )
    }

    pub fn is_mouse_button_held(&self, btn: MouseButton) -> bool {
        self.mouse_buttons_held.contains(&btn)
    }

    /// Positive = scroll up, negative = scroll down. Resets to 0 each frame.
    pub fn scroll_delta(&self) -> f32 {
        self.scroll_delta
    }
}

impl Default for TerminalInput {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) fn poll_terminal_input(
    mut input: ResMut<TerminalInput>,
    mut events: EventWriter<TerminalResizeEvent>,
) {
    input.begin_frame();

    // Detect resize by polling the terminal size directly each frame.
    // Event::Resize from the crossterm event stream is unreliable when ratatui's
    // backend also handles SIGWINCH, so we use an ioctl instead.
    if let Ok(size) = ratatui::crossterm::terminal::size() {
        if size != input.last_terminal_size {
            input.last_terminal_size = size;
            events.write(TerminalResizeEvent {
                width: size.0,
                height: size.1,
            });
        }
    }

    while ratatui::crossterm::event::poll(Duration::ZERO).unwrap_or(false) {
        match ratatui::crossterm::event::read() {
            Ok(Event::Key(key_event)) => match key_event.kind {
                KeyEventKind::Press => {
                    input.keys_just_pressed.insert(key_event.code);
                    input.keys_active.insert(key_event.code);
                }
                KeyEventKind::Repeat => {
                    input.keys_active.insert(key_event.code);
                }
                KeyEventKind::Release => {}
            },
            Ok(Event::Mouse(mouse_event)) => {
                input.mouse_col = mouse_event.column;
                input.mouse_row = mouse_event.row;
                match mouse_event.kind {
                    MouseEventKind::Down(btn) => {
                        input.mouse_buttons_held.insert(btn);
                    }
                    MouseEventKind::Up(btn) => {
                        input.mouse_buttons_held.remove(&btn);
                    }
                    MouseEventKind::ScrollUp => {
                        input.scroll_delta += 1.0;
                    }
                    MouseEventKind::ScrollDown => {
                        input.scroll_delta -= 1.0;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}
