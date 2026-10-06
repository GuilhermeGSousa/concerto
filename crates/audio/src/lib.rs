//! Minimal fire-and-forget audio: one-shot sound effects plus a single looping
//! music track.
use std::sync::Arc;

use concerto_app::{schedule_groups::LateUpdate, App, Plugin};
use concerto_ecs::{system::NonSendMarker, ResMut, Resource};

mod backend;
pub mod synth;

/// Mono PCM samples in `[-1, 1]`.
#[derive(Debug, Clone)]
pub struct SoundData {
    pub sample_rate: u32,
    pub samples: Vec<f32>,
}

impl SoundData {
    pub fn duration_secs(&self) -> f32 {
        self.samples.len() as f32 / self.sample_rate as f32
    }
}

/// Identifies a sound registered with [`Audio::add`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SoundHandle(u32);

/// Per-playback settings.
#[derive(Debug, Clone, Copy)]
pub struct PlayParams {
    /// Linear gain, multiplied by the master volume.
    pub volume: f32,
    /// Playback-rate multiplier; also shifts pitch.
    pub rate: f32,
    /// Stereo position: `-1.0` hard left, `0.0` center, `1.0` hard right.
    pub pan: f32,
}

impl Default for PlayParams {
    fn default() -> Self {
        Self {
            volume: 1.0,
            rate: 1.0,
            pan: 0.0,
        }
    }
}

impl PlayParams {
    pub fn volume(volume: f32) -> Self {
        Self {
            volume,
            ..Default::default()
        }
    }

    pub fn with_rate(mut self, rate: f32) -> Self {
        self.rate = rate;
        self
    }

    pub fn with_pan(mut self, pan: f32) -> Self {
        self.pan = pan.clamp(-1.0, 1.0);
        self
    }
}

pub(crate) enum AudioCommand {
    Play(SoundHandle, PlayParams),
    PlayMusic(SoundHandle, f32),
    StopMusic,
}

/// The game-facing audio API.
#[derive(Resource, Default)]
pub struct Audio {
    sounds: Vec<Arc<SoundData>>,
    commands: Vec<AudioCommand>,
    master_volume: f32,
    music_volume: f32,
    muted: bool,
}

impl Audio {
    pub fn new() -> Self {
        Self {
            master_volume: 1.0,
            music_volume: 1.0,
            ..Default::default()
        }
    }

    /// Registers a sound so it can be played by handle.
    pub fn add(&mut self, sound: SoundData) -> SoundHandle {
        self.sounds.push(Arc::new(sound));
        SoundHandle(self.sounds.len() as u32 - 1)
    }

    pub fn play(&mut self, sound: SoundHandle, params: PlayParams) {
        self.commands.push(AudioCommand::Play(sound, params));
    }

    /// Starts `sound` looping as the music track, replacing any current one.
    pub fn play_music(&mut self, sound: SoundHandle, volume: f32) {
        self.commands.push(AudioCommand::PlayMusic(sound, volume));
    }

    pub fn stop_music(&mut self) {
        self.commands.push(AudioCommand::StopMusic);
    }

    pub fn master_volume(&self) -> f32 {
        self.master_volume
    }

    pub fn set_master_volume(&mut self, volume: f32) {
        self.master_volume = volume.clamp(0.0, 1.0);
    }

    pub fn music_volume(&self) -> f32 {
        self.music_volume
    }

    pub fn set_music_volume(&mut self, volume: f32) {
        self.music_volume = volume.clamp(0.0, 1.0);
    }

    pub fn muted(&self) -> bool {
        self.muted
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }

    fn effective_master(&self) -> f32 {
        if self.muted {
            0.0
        } else {
            self.master_volume
        }
    }
}

fn flush_audio(_: NonSendMarker, mut audio: ResMut<Audio>) {
    let master = audio.effective_master();
    let music = audio.music_volume;
    let commands = std::mem::take(&mut audio.commands);
    backend::with_backend(|backend| {
        backend.set_volumes(master, music);
        for command in commands {
            match command {
                AudioCommand::Play(handle, params) => {
                    if let Some(sound) = audio.sounds.get(handle.0 as usize) {
                        backend.play(handle, sound, params);
                    }
                }
                AudioCommand::PlayMusic(handle, volume) => {
                    if let Some(sound) = audio.sounds.get(handle.0 as usize) {
                        backend.play_music(handle, sound, volume);
                    }
                }
                AudioCommand::StopMusic => backend.stop_music(),
            }
        }
    });
}

pub struct AudioPlugin;

impl Plugin for AudioPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Audio::new());
        app.add_system(LateUpdate, flush_audio);
    }
}
