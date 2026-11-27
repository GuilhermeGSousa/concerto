//! Sound playback: audio clips imported from sources, played by [`AudioPlayer`] entities on the
//! buses of an [`AudioMixer`].

mod audio;
mod clip;
mod importer;
mod mixer;
mod player;
mod plugin;
#[cfg(target_arch = "wasm32")]
mod web;

pub use audio::{Audio, AudioBackend};
pub use clip::AudioClip;
pub use importer::AudioImporter;
pub use mixer::{AudioBus, AudioMixer};
pub use player::{AudioPlayer, PlaybackEnd, SoundFinished};
pub use plugin::AudioPlugin;
