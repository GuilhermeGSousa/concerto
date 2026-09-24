use concerto_ecs::{
    events::event_reader::EventReader,
    resource::{Res, ResMut},
    Event, Query, With,
};
use concerto_render::{
    components::camera::RenderCamera, components::render_entity::RenderEntity,
    device::RenderDevice, resources::RenderContext,
};

use crate::readback::{TerminalOutput, TerminalRenderState};

#[derive(Event)]
pub struct TerminalResizeEvent {
    pub width: u16,
    pub height: u16,
}

pub(crate) fn handle_terminal_resize(
    mut events: EventReader<TerminalResizeEvent>,
    mut state: ResMut<TerminalRenderState>,
    device: Res<RenderDevice>,
    context: Res<RenderContext>,
    terminal_cameras: Query<&RenderEntity, With<TerminalOutput>>,
    render_cameras: Query<&mut RenderCamera>,
) {
    // Only care about the last resize event this frame
    let Some(ev) = events.read().last() else {
        return;
    };

    let (w, h) = (ev.width as u32, ev.height as u32);
    if w == 0 || h == 0 {
        return;
    }

    *state = TerminalRenderState::new(&**device, w, h);

    // Rebuild the RTT and depth texture on the terminal camera's render entity
    let Some(render_entity) = terminal_cameras.iter().next() else {
        return;
    };
    let Some(mut camera) = render_cameras.get_entity(**render_entity) else {
        return;
    };

    camera.resize_render_target(&**device, context.surface_config.format, w, h);
}
