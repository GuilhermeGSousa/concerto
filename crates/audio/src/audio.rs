use concerto_ecs::{Res, ResMut, Resource};
use concerto_foundation::time::Time;
use kira::{
    AudioManager, AudioManagerSettings, PlaySoundError,
    backend::{
        cpal::CpalBackend,
        mock::{MockBackend, MockBackendSettings},
    },
    sound::static_sound::{StaticSoundData, StaticSoundHandle},
    track::{MainTrackBuilder, MainTrackHandle, TrackBuilder, TrackHandle},
};

use crate::mixer::{AudioBus, AudioMixer, decibels};
use crate::player::FADE;

const MOCK_SAMPLE_RATE: u32 = 1000;

/// Which output [`AudioPlugin`](crate::AudioPlugin) opens.
///
/// `Device` opens the audio device, falling back to `Mock` when there is none;
/// `Mock` always opens the mock backend.
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioBackend {
    #[default]
    Device,
    Mock,
}

enum Output {
    Device(Box<AudioManager<CpalBackend>>),
    Mock(Box<AudioManager<MockBackend>>),
}

impl Output {
    fn main_track(&mut self) -> &mut MainTrackHandle {
        match self {
            Output::Device(manager) => manager.main_track(),
            Output::Mock(manager) => manager.main_track(),
        }
    }

    fn add_bus(&mut self, volume: f32) -> TrackHandle {
        let builder = TrackBuilder::new().volume(decibels(volume));
        match self {
            Output::Device(manager) => manager.add_sub_track(builder),
            Output::Mock(manager) => manager.add_sub_track(builder),
        }
        .expect("kira's default capacity holds the four buses")
    }
}

/// The audio device and the mixer's buses.
///
/// It is absent in an edit `App` and, on the web, until the first input.
#[derive(Resource)]
pub struct Audio {
    output: Output,
    buses: [TrackHandle; 4],
    follows_time: bool,
}

impl Audio {
    pub(crate) fn open(backend: AudioBackend, mixer: &AudioMixer) -> Self {
        let main = MainTrackBuilder::new().volume(decibels(mixer.master));
        let mut follows_time = false;
        let mut output = match backend {
            AudioBackend::Device => {
                match AudioManager::<CpalBackend>::new(AudioManagerSettings {
                    main_track_builder: main,
                    ..Default::default()
                }) {
                    Ok(manager) => Output::Device(Box::new(manager)),
                    Err(error) => {
                        log::warn!("no audio device, so the game plays silently: {error}");
                        follows_time = true;
                        Output::Mock(mock(MainTrackBuilder::new().volume(decibels(mixer.master))))
                    }
                }
            }
            AudioBackend::Mock => Output::Mock(mock(main)),
        };
        let buses = AudioBus::ALL.map(|bus| output.add_bus(mixer.bus(bus)));
        Self {
            output,
            buses,
            follows_time,
        }
    }

    /// Renders `seconds` of audio on the mock backend.
    ///
    /// It does nothing on a device, whose audio thread renders on its own.
    pub fn advance(&mut self, seconds: f32) {
        let Output::Mock(manager) = &mut self.output else {
            return;
        };
        let frames = (seconds.max(0.0) * MOCK_SAMPLE_RATE as f32).round() as u32;
        let backend = manager.backend_mut();
        for _ in 0..frames {
            backend.on_start_processing();
            backend.process();
        }
    }

    pub(crate) fn follows_time(&self) -> bool {
        self.follows_time
    }

    /// How many sounds are playing on `bus`.
    pub fn bus_sound_count(&self, bus: AudioBus) -> usize {
        self.buses[bus as usize].num_sounds()
    }

    pub(crate) fn play(
        &mut self,
        bus: AudioBus,
        sound: StaticSoundData,
    ) -> Result<StaticSoundHandle, PlaySoundError<()>> {
        self.buses[bus as usize].play(sound)
    }

    pub(crate) fn set_volumes(&mut self, mixer: &AudioMixer) {
        self.output
            .main_track()
            .set_volume(decibels(mixer.master), FADE);
        for bus in AudioBus::ALL {
            self.buses[bus as usize].set_volume(decibels(mixer.bus(bus)), FADE);
        }
    }
}

fn mock(main: MainTrackBuilder) -> Box<AudioManager<MockBackend>> {
    let manager = AudioManager::new(AudioManagerSettings {
        main_track_builder: main,
        internal_buffer_size: 1,
        backend_settings: MockBackendSettings {
            sample_rate: MOCK_SAMPLE_RATE,
        },
        ..Default::default()
    })
    .expect("the mock backend always starts");
    Box::new(manager)
}

pub(crate) fn advance_fallback(audio: Option<ResMut<Audio>>, time: Res<Time>) {
    if let Some(mut audio) = audio
        && audio.follows_time()
    {
        audio.advance(time.delta().as_secs_f32());
    }
}
