use std::collections::HashMap;

use web_sys::{
    AudioBuffer, AudioBufferSourceNode, AudioContext, AudioContextState, AudioNode, GainNode,
};

use super::AudioBackend;
use crate::{PlayParams, SoundData, SoundHandle};

/// Simultaneous one-shots beyond this are dropped: a crowd of identical hit
/// sounds on one frame only clips.
const MAX_VOICES_PER_FRAME: usize = 12;

pub(crate) struct WebBackend {
    context: AudioContext,
    master: GainNode,
    music_bus: GainNode,
    buffers: HashMap<SoundHandle, AudioBuffer>,
    music: Option<AudioBufferSourceNode>,
    voices_this_frame: usize,
}

impl WebBackend {
    pub(crate) fn new() -> Option<Self> {
        let context = AudioContext::new()
            .map_err(|err| log::warn!("concerto-audio: no AudioContext: {err:?}"))
            .ok()?;
        let master = context.create_gain().ok()?;
        master
            .connect_with_audio_node(&context.destination())
            .ok()?;
        let music_bus = context.create_gain().ok()?;
        music_bus.connect_with_audio_node(&master).ok()?;
        Some(Self {
            context,
            master,
            music_bus,
            buffers: HashMap::new(),
            music: None,
            voices_this_frame: 0,
        })
    }

    fn buffer(&mut self, handle: SoundHandle, sound: &SoundData) -> Option<AudioBuffer> {
        if let Some(buffer) = self.buffers.get(&handle) {
            return Some(buffer.clone());
        }
        let length = sound.samples.len().max(1) as u32;
        let buffer = self
            .context
            .create_buffer(1, length, sound.sample_rate as f32)
            .ok()?;
        buffer.copy_to_channel(&sound.samples, 0).ok()?;
        self.buffers.insert(handle, buffer.clone());
        Some(buffer)
    }

    fn source(
        &mut self,
        handle: SoundHandle,
        sound: &SoundData,
        volume: f32,
        rate: f32,
        pan: f32,
        output: &GainNode,
    ) -> Option<AudioBufferSourceNode> {
        let buffer = self.buffer(handle, sound)?;
        let source = self.context.create_buffer_source().ok()?;
        source.set_buffer(Some(&buffer));
        source.playback_rate().set_value(rate.max(0.01));
        let gain = self.context.create_gain().ok()?;
        gain.gain().set_value(volume.max(0.0));
        source.connect_with_audio_node(&gain).ok()?;
        let last: AudioNode = if pan != 0.0 {
            let panner = self.context.create_stereo_panner().ok()?;
            panner.pan().set_value(pan);
            gain.connect_with_audio_node(&panner).ok()?;
            panner.into()
        } else {
            gain.into()
        };
        last.connect_with_audio_node(output).ok()?;
        Some(source)
    }
}

impl AudioBackend for WebBackend {
    fn set_volumes(&mut self, master: f32, music: f32) {
        // Browsers start the context suspended until the page has had a user
        // gesture; keep asking until it runs.
        if self.context.state() == AudioContextState::Suspended {
            let _ = self.context.resume();
        }
        self.master.gain().set_value(master);
        self.music_bus.gain().set_value(music);
        self.voices_this_frame = 0;
    }

    fn play(&mut self, handle: SoundHandle, sound: &SoundData, params: PlayParams) {
        if self.voices_this_frame >= MAX_VOICES_PER_FRAME {
            return;
        }
        self.voices_this_frame += 1;
        let master = self.master.clone();
        if let Some(source) = self.source(
            handle,
            sound,
            params.volume,
            params.rate,
            params.pan,
            &master,
        ) {
            let _ = source.start();
        }
    }

    fn play_music(&mut self, handle: SoundHandle, sound: &SoundData, volume: f32) {
        self.stop_music();
        let bus = self.music_bus.clone();
        if let Some(source) = self.source(handle, sound, volume, 1.0, 0.0, &bus) {
            source.set_loop(true);
            let _ = source.start();
            self.music = Some(source);
        }
    }

    fn stop_music(&mut self) {
        if let Some(music) = self.music.take() {
            #[allow(deprecated)]
            let _ = music.stop();
        }
    }
}
