//! Run-level state: which screen is up, which night, how far you have got.
use concerto::ecs::Resource;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Title screen over a slow drift through an empty, lit store.
    Title,
    /// The night's title card; the store is built behind it.
    Intro,
    /// Walking the floor.
    Playing,
    /// Caught: the jumpscare is playing.
    Caught,
    /// The death card, waiting for a retry.
    Dead,
    /// Made it out; the night's summary card.
    Escaped,
}

#[derive(Resource)]
pub struct Game {
    pub phase: Phase,
    /// Set while playing when the pointer lock was lost; everything freezes.
    pub paused: bool,
    /// 1-based night number.
    pub night: u32,
    /// Best night reached across runs (persisted on the web).
    pub best_night: u32,
    /// Seconds spent in the current phase, in real time.
    pub phase_time: f32,
    /// Seconds of game time spent on the floor this night.
    pub night_time: f32,
    pub deaths: u32,
    /// Bumped whenever a night is (re)built so stale entities can be cleared.
    pub generation: u32,
    /// Seed of the current night, so a retry replays the same floor.
    pub seed: u64,
}

impl Game {
    pub fn new(best_night: u32, seed: u64) -> Self {
        Self {
            phase: Phase::Title,
            paused: false,
            night: 1,
            best_night,
            phase_time: 0.0,
            night_time: 0.0,
            deaths: 0,
            generation: 0,
            seed,
        }
    }

    pub fn set_phase(&mut self, phase: Phase) {
        if self.phase != phase {
            log::info!(
                "phase: {:?} -> {:?} (night {})",
                self.phase,
                phase,
                self.night
            );
            self.phase = phase;
            self.phase_time = 0.0;
        }
    }

    /// Whether the night simulation should advance.
    pub fn is_live(&self) -> bool {
        self.phase == Phase::Playing && !self.paused
    }
}

/// Deterministic-enough gameplay randomness without pulling in a crate.
#[derive(Resource)]
pub struct Rand(u64);

impl Rand {
    pub fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 32) as u32
    }

    /// Uniform in `[0, 1)`.
    pub fn unit(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32
    }

    pub fn range(&mut self, min: f32, max: f32) -> f32 {
        min + (max - min) * self.unit()
    }

    pub fn index(&mut self, len: usize) -> usize {
        (self.next_u32() as usize) % len.max(1)
    }
}
