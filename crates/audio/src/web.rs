use concerto_ecs::{CommandQueue, Res, events::event_reader::EventReader};
use concerto_window::events::WindowEvent;

use crate::{Audio, AudioBackend, AudioMixer};

pub(crate) fn open_audio_on_input(
    mut events: EventReader<WindowEvent>,
    audio: Option<Res<Audio>>,
    backend: Res<AudioBackend>,
    mixer: Res<AudioMixer>,
    mut cmd: CommandQueue,
) {
    let pressed = events.read().any(|event| {
        matches!(
            event,
            WindowEvent::Key { pressed: true, .. } | WindowEvent::MouseButton { pressed: true, .. }
        )
    });
    if pressed && audio.is_none() {
        cmd.insert_resource(Audio::open(*backend, &mixer));
    }
}
