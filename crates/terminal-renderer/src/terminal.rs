use concerto_ecs::Resource;

use derive_more::{Deref, DerefMut, From};

/// The ratatui terminal on standard output that frames are drawn to.
#[derive(Resource, Deref, DerefMut, From)]
pub struct TerminalContext(
    ratatui::prelude::Terminal<ratatui::prelude::CrosstermBackend<std::io::Stdout>>,
);

impl TerminalContext {
    /// Creates the context that draws frames to `terminal`.
    pub fn new(
        terminal: ratatui::prelude::Terminal<ratatui::prelude::CrosstermBackend<std::io::Stdout>>,
    ) -> TerminalContext {
        TerminalContext(terminal)
    }
}
