use concerto_ecs::{
    events::event_reader::EventReader,
    resource::{Res, ResMut},
};
use concerto_window::{plugin::Window, winit_events::WindowEvent};

use crate::resources::UIRenderer;

pub(crate) fn handle_window_events(
    mut window_events: EventReader<WindowEvent>,
    mut ui_renderer: ResMut<UIRenderer>,
    window: Res<Window>,
) {
    for event in window_events.read() {
        let _ = ui_renderer
            .state
            .on_window_event(&window.window_handle, event);
    }
}
