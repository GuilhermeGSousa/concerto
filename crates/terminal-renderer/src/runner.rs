use concerto_app::{plugins::PluginsState, runner::AppExit, App};

use ratatui::crossterm::event;

use crate::TerminalInput;

pub(crate) fn terminal_runner(mut app: App) -> AppExit {
    if app.plugin_state() != PluginsState::Finished {
        while app.plugin_state() != PluginsState::Ready {}
    }

    if app.plugin_state() == PluginsState::Ready {
        app.finish_plugin_build();
    }

    loop {
        app.update();

        if app
            .get_resource::<TerminalInput>()
            .expect("TerminalInput resource does not exist, did you register the TerminalRender plugin?")
            .is_key_active(event::KeyCode::Esc)
        {
            break;
        }
    }

    ratatui::restore();
    AppExit::Success
}
