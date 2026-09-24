use concerto_ecs::resource::{Res, ResMut};

use crate::{device::RenderDevice, queue::RenderQueue, render_asset::render_window::RenderWindow};

pub(crate) fn present_window(mut render_window: ResMut<RenderWindow>) {
    render_window.present();
}

pub fn finish_render(mut device: ResMut<RenderDevice>, queue: Res<RenderQueue>) {
    device.finish(&queue);
}
