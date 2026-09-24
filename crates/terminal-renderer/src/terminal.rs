use concerto_ecs::Resource;

use derive_more::{Deref, DerefMut, From};

#[derive(Resource, Deref, DerefMut, From)]
pub struct TerminalContext(
    ratatui::prelude::Terminal<ratatui::prelude::CrosstermBackend<std::io::Stdout>>,
);

impl TerminalContext {
    pub fn new(
        terminal: ratatui::prelude::Terminal<ratatui::prelude::CrosstermBackend<std::io::Stdout>>,
    ) -> TerminalContext {
        TerminalContext(terminal)
    }
}
