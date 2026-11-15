use std::io::Cursor;
use std::time::Duration;

use concerto_foundation::assets::{Asset, LoadableAsset, asset_server::AssetLoadContext};
use kira::sound::static_sound::StaticSoundData;
use serde::{Deserialize, Serialize};

/// A sound decoded at load and ready to play; imported from `.ogg` and `.wav`.
#[derive(Serialize, Deserialize)]
pub struct AudioClip {
    bytes: Vec<u8>,
    #[serde(skip)]
    sound: Option<StaticSoundData>,
}

impl AudioClip {
    pub(crate) fn encoded(bytes: Vec<u8>) -> Self {
        Self { bytes, sound: None }
    }

    /// How long the clip plays at normal speed; zero until it is loaded.
    pub fn duration(&self) -> Duration {
        self.sound
            .as_ref()
            .map_or(Duration::ZERO, StaticSoundData::duration)
    }

    /// The decoded frames per second; zero until it is loaded.
    pub fn sample_rate(&self) -> u32 {
        self.sound.as_ref().map_or(0, |sound| sound.sample_rate)
    }
}

impl Asset for AudioClip {
    fn name() -> &'static str {
        "AudioClip"
    }
}

impl LoadableAsset for AudioClip {
    fn on_load(&mut self, _context: &AssetLoadContext) -> anyhow::Result<()> {
        let bytes = std::mem::take(&mut self.bytes);
        self.sound = Some(StaticSoundData::from_cursor(Cursor::new(bytes))?);
        Ok(())
    }
}
