//! A tiny offline synthesizer for building [`SoundData`] in code.
use crate::SoundData;

pub const SAMPLE_RATE: u32 = 44_100;

/// Oscillator shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wave {
    Sine,
    Triangle,
    Square,
    Saw,
    Noise,
}

/// One voice: a waveform with pitch, envelope and filter trajectories.
#[derive(Debug, Clone, Copy)]
pub struct Tone {
    pub wave: Wave,
    /// Frequency in Hz at the start and end of the tone; swept exponentially.
    pub freq: (f32, f32),
    pub duration: f32,
    /// Seconds to ramp in from silence.
    pub attack: f32,
    /// Shape of the fade-out: `envelope = (1 - t/duration)^decay`.
    pub decay: f32,
    pub volume: f32,
    /// Low-pass cutoff in Hz at the start and end, swept exponentially.
    pub lowpass: Option<(f32, f32)>,
    /// Vibrato as (rate Hz, depth as a fraction of the frequency).
    pub vibrato: Option<(f32, f32)>,
}

impl Tone {
    pub fn new(wave: Wave, freq: f32, duration: f32) -> Self {
        Self {
            wave,
            freq: (freq, freq),
            duration,
            attack: 0.002,
            decay: 1.0,
            volume: 1.0,
            lowpass: None,
            vibrato: None,
        }
    }

    pub fn sweep(mut self, to: f32) -> Self {
        self.freq.1 = to;
        self
    }

    pub fn attack(mut self, attack: f32) -> Self {
        self.attack = attack;
        self
    }

    pub fn decay(mut self, decay: f32) -> Self {
        self.decay = decay;
        self
    }

    pub fn volume(mut self, volume: f32) -> Self {
        self.volume = volume;
        self
    }

    pub fn lowpass(mut self, from: f32, to: f32) -> Self {
        self.lowpass = Some((from, to));
        self
    }

    pub fn vibrato(mut self, rate: f32, depth: f32) -> Self {
        self.vibrato = Some((rate, depth));
        self
    }
}

/// Deterministic xorshift noise source.
#[derive(Debug, Clone)]
pub struct Rng(u32);

impl Rng {
    pub fn new(seed: u32) -> Self {
        Self(seed.max(1))
    }

    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// Uniform in `[-1, 1]`.
    pub fn signed(&mut self) -> f32 {
        (self.next_u32() as f64 / u32::MAX as f64 * 2.0 - 1.0) as f32
    }
}

fn exp_lerp(from: f32, to: f32, t: f32) -> f32 {
    if from == to {
        return from;
    }
    if from <= 0.0 || to <= 0.0 {
        return from + (to - from) * t;
    }
    from * (to / from).powf(t)
}

/// A buffer that tones are rendered into.
pub struct Mix {
    samples: Vec<f32>,
    rng: Rng,
}

impl Mix {
    pub fn new(duration: f32) -> Self {
        Self::with_seed(duration, 0x9E37_79B9)
    }

    pub fn with_seed(duration: f32, seed: u32) -> Self {
        let len = (duration.max(0.0) * SAMPLE_RATE as f32).ceil() as usize;
        Self {
            samples: vec![0.0; len],
            rng: Rng::new(seed),
        }
    }

    /// Renders `tone` starting `offset` seconds in.
    pub fn add(&mut self, offset: f32, tone: Tone) -> &mut Self {
        let sr = SAMPLE_RATE as f32;
        let start = (offset.max(0.0) * sr) as usize;
        let len = (tone.duration * sr) as usize;
        let mut phase = 0.0f32;
        let mut filtered = 0.0f32;
        let mut alpha = 1.0f32;
        for i in 0..len {
            let Some(out) = self.samples.get_mut(start + i) else {
                break;
            };
            let t = i as f32 / sr;
            let progress = t / tone.duration.max(1e-6);

            let mut freq = exp_lerp(tone.freq.0, tone.freq.1, progress);
            if let Some((rate, depth)) = tone.vibrato {
                freq *= 1.0 + depth * (std::f32::consts::TAU * rate * t).sin();
            }
            phase = (phase + freq / sr).fract();

            let raw = match tone.wave {
                Wave::Sine => (std::f32::consts::TAU * phase).sin(),
                Wave::Triangle => 1.0 - 4.0 * (phase - 0.5).abs(),
                Wave::Square => {
                    if phase < 0.5 {
                        1.0
                    } else {
                        -1.0
                    }
                }
                Wave::Saw => 2.0 * phase - 1.0,
                Wave::Noise => self.rng.signed(),
            };

            let sample = match tone.lowpass {
                Some((from, to)) => {
                    if i == 0 || from != to {
                        let cutoff = exp_lerp(from, to, progress).min(sr * 0.45);
                        alpha = 1.0 - (-std::f32::consts::TAU * cutoff / sr).exp();
                    }
                    filtered += alpha * (raw - filtered);
                    filtered
                }
                None => raw,
            };

            let attack = if tone.attack > 0.0 {
                (t / tone.attack).min(1.0)
            } else {
                1.0
            };
            let release = (1.0 - progress).max(0.0).powf(tone.decay);
            *out += sample * attack * release * tone.volume;
        }
        self
    }

    /// Scales the mix so its loudest sample has magnitude `peak`.
    pub fn normalize(&mut self, peak: f32) -> &mut Self {
        let max = self.samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        if max > 0.0 {
            let gain = peak / max;
            self.samples.iter_mut().for_each(|s| *s *= gain);
        }
        self
    }

    /// Soft-clips every sample through `tanh`, taming summed peaks.
    pub fn saturate(&mut self, drive: f32) -> &mut Self {
        self.samples
            .iter_mut()
            .for_each(|s| *s = (*s * drive).tanh());
        self
    }

    pub fn build(&mut self) -> SoundData {
        SoundData {
            sample_rate: SAMPLE_RATE,
            samples: std::mem::take(&mut self.samples),
        }
    }
}

/// Frequency of a MIDI note number (69 = A4 = 440 Hz).
pub fn midi(note: i32) -> f32 {
    440.0 * 2f32.powf((note - 69) as f32 / 12.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mix_has_requested_length_and_normalizes() {
        let sound = Mix::new(0.5)
            .add(0.0, Tone::new(Wave::Sine, 440.0, 0.25).volume(3.0))
            .normalize(0.8)
            .build();
        assert_eq!(
            sound.samples.len(),
            (0.5 * SAMPLE_RATE as f32).ceil() as usize
        );
        let peak = sound.samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!((peak - 0.8).abs() < 1e-4);
        assert!(sound.samples[SAMPLE_RATE as usize * 3 / 8..]
            .iter()
            .all(|s| *s == 0.0));
    }

    #[test]
    fn tones_past_the_end_are_clipped() {
        let sound = Mix::new(0.1)
            .add(0.05, Tone::new(Wave::Noise, 0.0, 1.0))
            .build();
        assert_eq!(
            sound.samples.len(),
            (0.1 * SAMPLE_RATE as f32).ceil() as usize
        );
    }

    #[test]
    fn noise_is_deterministic() {
        let a = Mix::new(0.1)
            .add(0.0, Tone::new(Wave::Noise, 0.0, 0.1))
            .build();
        let b = Mix::new(0.1)
            .add(0.0, Tone::new(Wave::Noise, 0.0, 0.1))
            .build();
        assert_eq!(a.samples, b.samples);
    }

    #[test]
    fn midi_a4_is_440() {
        assert!((midi(69) - 440.0).abs() < 1e-3);
        assert!((midi(81) - 880.0).abs() < 1e-2);
    }
}
