use concerto_app::{plugins::PluginsState, App};
use concerto_render::{draw_target::DrawTarget, gpu::Gpu};
use ratatui::crossterm::event;

use crate::TerminalInput;

/// Builds an `App` with `build` and drives it from the terminal until Esc is pressed.
pub fn run(build: impl FnOnce(&mut App)) {
    let gpu = pollster::block_on(Gpu::request(&Gpu::instance(), None))
        .expect("no GPU adapter is available");
    let mut app = App::new();
    app.insert_resource(gpu).insert_resource(DrawTarget::None);
    build(&mut app);

    while app.plugin_state() != PluginsState::Ready {}
    app.finish_plugin_build();

    loop {
        app.update();
        profiling::finish_frame!();

        if app
            .get_resource::<TerminalInput>()
            .expect("TerminalInput resource does not exist, did you register the TerminalRender plugin?")
            .is_key_active(event::KeyCode::Esc)
        {
            break;
        }
    }

    ratatui::restore();
}
