//! Every sound in the game, synthesized at startup (no audio files).
use concerto::{
    audio::{
        Audio, PlayParams, SoundHandle,
        synth::{Mix, Tone, Wave, midi},
    },
    ecs::{ResMut, Resource},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sfx {
    Footstep,
    FlashlightClick,
    FlashlightDie,
    Flicker,
    /// Plastic and fibreglass shifting somewhere you are not looking.
    Creak,
    KeyPickup,
    BatteryPickup,
    Unlock,
    Escape,
    /// The jumpscare hit.
    Stinger,
    DeathDrone,
    Heartbeat,
    Click,
    NightStart,
}

#[derive(Resource, Default)]
pub struct Sounds {
    handles: Vec<(Sfx, SoundHandle)>,
    pub ambience: Option<SoundHandle>,
    /// Per-play pitch jitter so repeats don't sound mechanical.
    jitter: u32,
}

impl Sounds {
    fn handle(&self, sfx: Sfx) -> Option<SoundHandle> {
        self.handles
            .iter()
            .find(|(s, _)| *s == sfx)
            .map(|(_, h)| *h)
    }

    fn pitch(&mut self, sfx: Sfx) -> f32 {
        self.jitter = self.jitter.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        let spread = ((self.jitter >> 16) % 1000) as f32 / 1000.0 - 0.5;
        match sfx {
            Sfx::Footstep => 1.0 + spread * 0.2,
            Sfx::Creak => 1.0 + spread * 0.5,
            Sfx::Flicker => 1.0 + spread * 0.3,
            _ => 1.0,
        }
    }

    pub fn play(&mut self, audio: &mut Audio, sfx: Sfx, volume: f32) {
        self.play_at(audio, sfx, volume, 0.0);
    }

    /// Plays with a stereo position (`-1` left .. `1` right).
    pub fn play_at(&mut self, audio: &mut Audio, sfx: Sfx, volume: f32, pan: f32) {
        let pitch = self.pitch(sfx);
        if let Some(handle) = self.handle(sfx) {
            audio.play(
                handle,
                PlayParams::volume(volume).with_rate(pitch).with_pan(pan),
            );
        }
    }
}

fn build(sfx: Sfx) -> Mix {
    match sfx {
        Sfx::Footstep => {
            // Rubber sole on vinyl: a soft thump and a squeak of grit.
            let mut mix = Mix::with_seed(0.14, 7);
            mix.add(
                0.0,
                Tone::new(Wave::Sine, 110.0, 0.08)
                    .sweep(60.0)
                    .decay(3.0)
                    .volume(0.7),
            )
            .add(
                0.0,
                Tone::new(Wave::Noise, 0.0, 0.06)
                    .lowpass(2400.0, 500.0)
                    .decay(3.0)
                    .volume(0.5),
            );
            mix
        }
        Sfx::FlashlightClick => {
            let mut mix = Mix::with_seed(0.06, 13);
            mix.add(
                0.0,
                Tone::new(Wave::Noise, 0.0, 0.012)
                    .lowpass(9000.0, 4000.0)
                    .decay(2.0),
            )
            .add(
                0.018,
                Tone::new(Wave::Noise, 0.0, 0.01)
                    .lowpass(7000.0, 3000.0)
                    .decay(2.0)
                    .volume(0.6),
            );
            mix
        }
        Sfx::FlashlightDie => {
            let mut mix = Mix::with_seed(0.8, 17);
            mix.add(
                0.0,
                Tone::new(Wave::Saw, 120.0, 0.7)
                    .sweep(40.0)
                    .lowpass(900.0, 100.0)
                    .decay(1.2)
                    .volume(0.6),
            )
            .add(
                0.0,
                Tone::new(Wave::Noise, 0.0, 0.3)
                    .lowpass(3000.0, 300.0)
                    .decay(2.0)
                    .volume(0.4),
            );
            mix
        }
        Sfx::Flicker => {
            // A fluorescent ballast buzzing and catching.
            let mut mix = Mix::with_seed(0.25, 19);
            mix.add(
                0.0,
                Tone::new(Wave::Square, 120.0, 0.22)
                    .lowpass(2500.0, 800.0)
                    .decay(1.5)
                    .volume(0.5),
            )
            .add(
                0.0,
                Tone::new(Wave::Noise, 0.0, 0.05)
                    .lowpass(8000.0, 3000.0)
                    .decay(2.0)
                    .volume(0.4),
            );
            mix
        }
        Sfx::Creak => {
            // Stiff joints grinding: a scraped, resonant squeal over a knock.
            let mut mix = Mix::with_seed(0.6, 23);
            mix.add(
                0.0,
                Tone::new(Wave::Saw, 180.0, 0.45)
                    .sweep(260.0)
                    .lowpass(1600.0, 700.0)
                    .vibrato(23.0, 0.08)
                    .attack(0.04)
                    .decay(1.3)
                    .volume(0.45),
            )
            .add(
                0.0,
                Tone::new(Wave::Noise, 0.0, 0.4)
                    .lowpass(900.0, 400.0)
                    .attack(0.05)
                    .decay(1.2)
                    .volume(0.5),
            )
            .add(
                0.32,
                Tone::new(Wave::Sine, 90.0, 0.12)
                    .sweep(55.0)
                    .decay(3.0)
                    .volume(0.6),
            );
            mix
        }
        Sfx::KeyPickup => {
            let mut mix = Mix::new(0.9);
            for (i, note) in [81, 88].into_iter().enumerate() {
                mix.add(
                    i as f32 * 0.07,
                    Tone::new(Wave::Sine, midi(note), 0.7)
                        .decay(2.5)
                        .volume(0.5),
                )
                .add(
                    i as f32 * 0.07,
                    Tone::new(Wave::Sine, midi(note) * 2.76, 0.3)
                        .decay(4.0)
                        .volume(0.15),
                );
            }
            mix
        }
        Sfx::BatteryPickup => {
            let mut mix = Mix::new(0.3);
            mix.add(
                0.0,
                Tone::new(Wave::Square, 700.0, 0.06)
                    .lowpass(3000.0, 2000.0)
                    .decay(2.0)
                    .volume(0.35),
            )
            .add(
                0.08,
                Tone::new(Wave::Square, 1050.0, 0.1)
                    .lowpass(3000.0, 2000.0)
                    .decay(2.0)
                    .volume(0.35),
            );
            mix
        }
        Sfx::Unlock => {
            // A heavy bolt drawn back somewhere far off.
            let mut mix = Mix::with_seed(1.5, 29);
            mix.add(
                0.0,
                Tone::new(Wave::Noise, 0.0, 0.25)
                    .lowpass(1800.0, 300.0)
                    .decay(2.0)
                    .volume(0.7),
            )
            .add(
                0.0,
                Tone::new(Wave::Sine, 70.0, 0.4)
                    .sweep(45.0)
                    .decay(2.0)
                    .volume(0.8),
            )
            .add(
                0.3,
                Tone::new(Wave::Triangle, midi(57), 1.1)
                    .decay(1.5)
                    .volume(0.3),
            )
            .add(
                0.3,
                Tone::new(Wave::Triangle, midi(64), 1.1)
                    .decay(1.5)
                    .volume(0.25),
            );
            mix
        }
        Sfx::Escape => {
            let mut mix = Mix::with_seed(3.0, 31);
            for (i, note) in [57, 60, 64, 69].into_iter().enumerate() {
                mix.add(
                    i as f32 * 0.25,
                    Tone::new(Wave::Triangle, midi(note), 2.2)
                        .attack(0.1)
                        .decay(1.5)
                        .volume(0.3),
                );
            }
            mix.add(
                0.0,
                Tone::new(Wave::Noise, 0.0, 2.5)
                    .lowpass(300.0, 1200.0)
                    .attack(1.0)
                    .decay(1.0)
                    .volume(0.15),
            );
            mix
        }
        Sfx::Stinger => {
            // Dissonant brass-and-noise slam with a shriek on top.
            let mut mix = Mix::with_seed(1.4, 37);
            for (note, vol) in [(40, 0.7), (41, 0.6), (46, 0.5), (52, 0.4), (53, 0.4)] {
                mix.add(
                    0.0,
                    Tone::new(Wave::Saw, midi(note), 1.3)
                        .lowpass(4000.0, 600.0)
                        .vibrato(6.0, 0.02)
                        .decay(1.2)
                        .volume(vol),
                );
            }
            mix.add(
                0.0,
                Tone::new(Wave::Noise, 0.0, 0.9)
                    .lowpass(12000.0, 800.0)
                    .decay(1.5)
                    .volume(0.9),
            )
            .add(
                0.0,
                Tone::new(Wave::Saw, midi(88), 0.8)
                    .sweep(midi(94))
                    .vibrato(11.0, 0.03)
                    .lowpass(6000.0, 3000.0)
                    .decay(1.4)
                    .volume(0.35),
            )
            .add(
                0.0,
                Tone::new(Wave::Sine, 55.0, 1.0)
                    .sweep(30.0)
                    .decay(1.5)
                    .volume(1.0),
            );
            mix.saturate(2.2);
            mix
        }
        Sfx::DeathDrone => {
            let mut mix = Mix::new(3.5);
            for (note, vol) in [(33, 0.6), (40, 0.4), (45, 0.25)] {
                mix.add(
                    0.0,
                    Tone::new(Wave::Saw, midi(note), 3.5)
                        .lowpass(500.0, 150.0)
                        .attack(0.3)
                        .decay(1.2)
                        .volume(vol),
                );
            }
            mix
        }
        Sfx::Heartbeat => {
            // Lub-dub.
            let mut mix = Mix::new(0.5);
            mix.add(
                0.0,
                Tone::new(Wave::Sine, 62.0, 0.14).sweep(42.0).decay(2.5),
            )
            .add(
                0.17,
                Tone::new(Wave::Sine, 55.0, 0.12)
                    .sweep(38.0)
                    .decay(2.5)
                    .volume(0.7),
            );
            mix
        }
        Sfx::Click => {
            let mut mix = Mix::new(0.08);
            mix.add(
                0.0,
                Tone::new(Wave::Square, 900.0, 0.04)
                    .lowpass(3000.0, 1200.0)
                    .decay(3.0)
                    .volume(0.4),
            );
            mix
        }
        Sfx::NightStart => {
            // A distant store PA chime, slightly out of tune.
            let mut mix = Mix::new(2.6);
            for (i, note) in [76, 72, 67, 60].into_iter().enumerate() {
                mix.add(
                    i as f32 * 0.42,
                    Tone::new(Wave::Sine, midi(note) * 0.993, 1.2)
                        .vibrato(4.5, 0.004)
                        .decay(2.0)
                        .volume(0.45),
                )
                .add(
                    i as f32 * 0.42,
                    Tone::new(Wave::Sine, midi(note + 12), 0.5)
                        .decay(3.0)
                        .volume(0.1),
                );
            }
            mix
        }
    }
}

/// A 24-second loop of fluorescent hum, air-handling rumble and a slow,
/// barely-there minor drone.
fn ambience() -> Mix {
    let length = 24.0;
    let mut mix = Mix::with_seed(length, 97);
    // Mains hum and its buzz.
    mix.add(
        0.0,
        Tone::new(Wave::Sine, 60.0, length).decay(0.0).volume(0.35),
    )
    .add(
        0.0,
        Tone::new(Wave::Sine, 120.0, length).decay(0.0).volume(0.25),
    )
    .add(
        0.0,
        Tone::new(Wave::Square, 120.0, length)
            .lowpass(900.0, 900.0)
            .decay(0.0)
            .volume(0.04),
    );
    // HVAC: filtered noise.
    mix.add(
        0.0,
        Tone::new(Wave::Noise, 0.0, length)
            .lowpass(180.0, 180.0)
            .decay(0.0)
            .volume(0.9),
    );
    // Drone swells, sparse and detuned.
    for (start, note, dur) in [
        (0.0, 38, 11.0),
        (6.0, 45, 9.0),
        (12.0, 41, 11.0),
        (17.0, 44, 7.0),
    ] {
        mix.add(
            start,
            Tone::new(Wave::Triangle, midi(note), dur)
                .attack(dur * 0.45)
                .decay(1.5)
                .vibrato(0.2, 0.006)
                .lowpass(500.0, 500.0)
                .volume(0.35),
        );
    }
    mix
}

pub const ALL: [Sfx; 14] = [
    Sfx::Footstep,
    Sfx::FlashlightClick,
    Sfx::FlashlightDie,
    Sfx::Flicker,
    Sfx::Creak,
    Sfx::KeyPickup,
    Sfx::BatteryPickup,
    Sfx::Unlock,
    Sfx::Escape,
    Sfx::Stinger,
    Sfx::DeathDrone,
    Sfx::Heartbeat,
    Sfx::Click,
    Sfx::NightStart,
];

pub fn load_sounds(mut audio: ResMut<Audio>, mut sounds: ResMut<Sounds>) {
    for sfx in ALL {
        let peak = if sfx == Sfx::Stinger { 0.95 } else { 0.8 };
        let data = build(sfx).normalize(peak).build();
        let handle = audio.add(data);
        sounds.handles.push((sfx, handle));
    }
    sounds.ambience = Some(audio.add(ambience().normalize(0.6).build()));
    audio.set_music_volume(0.7);
}
