use concerto_ecs::{Res, ResMut, Resource, query::change_detection::DetectChanges};
use concerto_reflect::Reflect;
use kira::Decibels;

use crate::Audio;

/// Linear volume of the master and each bus: 0 or less is silent, 1 unchanged.
#[derive(Resource, Reflect, Clone, Copy, Debug, PartialEq)]
pub struct AudioMixer {
    /// The volume of everything played.
    pub master: f32,
    /// The volume of the [`AudioBus::Music`] bus.
    pub music: f32,
    /// The volume of the [`AudioBus::Ambience`] bus.
    pub ambience: f32,
    /// The volume of the [`AudioBus::Effects`] bus.
    pub effects: f32,
    /// The volume of the [`AudioBus::Dialogue`] bus.
    pub dialogue: f32,
}

impl AudioMixer {
    pub(crate) fn bus(&self, bus: AudioBus) -> f32 {
        match bus {
            AudioBus::Music => self.music,
            AudioBus::Ambience => self.ambience,
            AudioBus::Effects => self.effects,
            AudioBus::Dialogue => self.dialogue,
        }
    }
}

impl Default for AudioMixer {
    fn default() -> Self {
        Self {
            master: 1.0,
            music: 1.0,
            ambience: 1.0,
            effects: 1.0,
            dialogue: 1.0,
        }
    }
}

/// The mixer bus a sound plays on, under the master.
#[derive(Reflect, Default, Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioBus {
    /// Score and songs.
    Music,
    /// Background sound of a place, such as wind or a crowd.
    Ambience,
    /// Sound effects; the default.
    #[default]
    Effects,
    /// Spoken lines.
    Dialogue,
}

impl AudioBus {
    pub(crate) const ALL: [AudioBus; 4] = [
        AudioBus::Music,
        AudioBus::Ambience,
        AudioBus::Effects,
        AudioBus::Dialogue,
    ];
}

pub(crate) fn decibels(volume: f32) -> Decibels {
    if volume <= 0.0 {
        Decibels::SILENCE
    } else {
        Decibels((20.0 * volume.log10()).max(Decibels::SILENCE.0))
    }
}

pub(crate) fn apply_mixer(mixer: Res<AudioMixer>, audio: Option<ResMut<Audio>>) {
    let Some(mut audio) = audio else {
        return;
    };
    if mixer.is_changed() || audio.is_added() {
        audio.set_volumes(&mixer);
    }
}
