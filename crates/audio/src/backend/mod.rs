use crate::{PlayParams, SoundData, SoundHandle};

#[cfg(target_arch = "wasm32")]
mod web;
#[cfg(target_arch = "wasm32")]
use web::WebBackend as ActiveBackend;

#[cfg(not(target_arch = "wasm32"))]
mod null;
#[cfg(not(target_arch = "wasm32"))]
use null::NullBackend as ActiveBackend;

pub(crate) trait AudioBackend {
    fn set_volumes(&mut self, master: f32, music: f32);
    fn play(&mut self, handle: SoundHandle, sound: &SoundData, params: PlayParams);
    fn play_music(&mut self, handle: SoundHandle, sound: &SoundData, volume: f32);
    fn stop_music(&mut self);
}

thread_local! {
    static BACKEND: std::cell::RefCell<Option<ActiveBackend>> = const { std::cell::RefCell::new(None) };
}

pub(crate) fn with_backend(f: impl FnOnce(&mut dyn AudioBackend)) {
    BACKEND.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            *slot = ActiveBackend::new();
        }
        if let Some(backend) = slot.as_mut() {
            f(backend);
        }
    });
}
