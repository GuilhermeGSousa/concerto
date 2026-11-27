use concerto_ecs::Resource;
use ratatui::{
    style::Style,
    widgets::{Block, Borders, Widget},
};

/// The latest frame as ASCII text, one line per row, ready to draw in the terminal.
#[derive(Resource, Default)]
pub struct TerminalFrame {
    buf: Option<String>,
}

impl TerminalFrame {
    /// Calls `f` with the frame's text buffer to rewrite it in place, keeping its allocation.
    pub fn scoped_buffer(&mut self, f: impl FnOnce(&mut String)) {
        let mut buf = self.buf.take().unwrap_or_default();
        f(&mut buf);
        self.buf = Some(buf);
    }

    /// Returns the frame as a ratatui widget, or `None` before the first frame is written.
    pub fn current_frame(&self) -> Option<CurrentFrame<'_>> {
        self.buf.as_deref().map(CurrentFrame)
    }
}

/// A ratatui widget that draws the frame's text inside a bordered block, clipped to its area.
pub struct CurrentFrame<'a>(&'a str);

impl Widget for CurrentFrame<'_> {
    fn render(self, area: ratatui::prelude::Rect, buf: &mut ratatui::prelude::Buffer) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title("And this is also a widget!");
        let inner = block.inner(area);
        block.render(area, buf);

        for (row, line) in self.0.lines().enumerate() {
            let y = inner.y + row as u16;
            if y >= inner.bottom() {
                break;
            }
            buf.set_stringn(inner.x, y, line, inner.width as usize, Style::default());
        }
    }
}
