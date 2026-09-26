use concerto_app::{
    plugins::Plugin,
    schedule_groups::{LateRender, Update},
};
use concerto_ecs::IntoSystemConfig;
use concerto_render::{device::RenderDevice, sets::RenderSet, systems::render::finish_render};

use crate::{
    frame::TerminalFrame,
    input::{poll_terminal_input, TerminalInput},
    readback::{readback_terminal_frame, TerminalRenderState},
    resize::{handle_terminal_resize, TerminalResizeEvent},
    runner::terminal_runner,
    terminal::TerminalContext,
};

/// Renders the camera tagged [`TerminalOutput`](crate::TerminalOutput) as ASCII in the terminal.
///
/// # Panics
///
/// Currently always panics during [`finish`](Plugin::finish): it looks for `RenderDevice` in
/// the main world, which no longer holds it. See
/// `docs/superpowers/specs/2026-09-25-terminal-renderer-split-worlds.md`.
pub struct TerminalRendererPlugin;

impl Plugin for TerminalRendererPlugin {
    fn build(&self, app: &mut concerto_app::App) {
        app.set_runner(terminal_runner);
        app.add_system(Update, poll_terminal_input);
        app.add_system(
            LateRender,
            readback_terminal_frame
                .in_set(RenderSet::Present)
                .after(finish_render),
        );
        app.add_system(Update, handle_terminal_resize);

        app.register_event::<TerminalResizeEvent>();

        app.insert_resource(TerminalFrame::default());
        app.insert_resource(TerminalContext::new(ratatui::init()));
    }

    fn finish(&self, app: &mut concerto_app::App) {
        let state = {
            let device = app.get_resource::<RenderDevice>().expect(
                "RenderDevice not found — make sure RenderPlugin is registered before TerminalRendererPlugin",
            );
            let terminal_size = app
                .get_resource::<TerminalContext>()
                .unwrap()
                .size()
                .unwrap();
            TerminalRenderState::new(
                &*device,
                terminal_size.width as u32,
                terminal_size.height as u32,
            )
        };

        app.insert_resource(state);
        app.insert_resource(TerminalInput::new());
    }
}
