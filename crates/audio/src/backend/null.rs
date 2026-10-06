use super::AudioBackend;
use crate::{PlayParams, SoundData, SoundHandle};

pub(crate) struct NullBackend;

impl NullBackend {
    pub(crate) fn new() -> Option<Self> {
        log::info!("concerto-audio: no native backend yet, audio is silent");
        Some(Self)
    }
}

impl AudioBackend for NullBackend {
    fn set_volumes(&mut self, _master: f32, _music: f32) {}
    fn play(&mut self, _handle: SoundHandle, _sound: &SoundData, _params: PlayParams) {}
    fn play_music(&mut self, _handle: SoundHandle, _sound: &SoundData, _volume: f32) {}
    fn stop_music(&mut self) {}
}
